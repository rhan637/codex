use anyhow::Result;
use codex_core::TurnInputRequest;
use codex_core::config::Config;
use codex_features::Feature;
use codex_history::InitialHistory;
use codex_history::RolloutItem;
use codex_login::CodexAuth;
use codex_model_provider_info::built_in_model_providers;
use codex_models_manager::manager::RefreshStrategy;
use codex_protocol::openai_models::InputModality;
use codex_protocol::openai_models::ModelsResponse;
use codex_protocol::protocol::ADAPTIVE_CONTEXT_BUDGET_POLICY_VERSION;
use codex_protocol::protocol::AdaptiveContextBudgetPolicy;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::Op;
use codex_protocol::protocol::ThreadSettingsOverrides;
use codex_protocol::user_input::UserInput;
use codex_rollout::RolloutRecorder;
use core_test_support::responses::ev_assistant_message;
use core_test_support::responses::ev_completed;
use core_test_support::responses::ev_completed_with_tokens;
use core_test_support::responses::mount_compact_json_once;
use core_test_support::responses::mount_models_once;
use core_test_support::responses::mount_sse_sequence;
use core_test_support::responses::sse;
use core_test_support::responses::sse_failed;
use core_test_support::responses::start_mock_server;
use core_test_support::test_codex::TestCodex;
use core_test_support::test_codex::test_codex;
use core_test_support::wait_for_event;
use pretty_assertions::assert_eq;
use serde_json::json;

use super::model_switching::test_model_info;

fn policy() -> AdaptiveContextBudgetPolicy {
    AdaptiveContextBudgetPolicy {
        policy_version: ADAPTIVE_CONTEXT_BUDGET_POLICY_VERSION,
        context_window_tiers: vec![100, 200],
        keep_below_percent: 40,
        expand_at_or_above_percent: 60,
        ambiguous_compactions_before_expand: 2,
    }
}

fn configure_adaptive_context_budget(config: &mut Config) {
    config.model_context_window = Some(1_000);
    config
        .features
        .enable(Feature::AdaptiveContextBudget)
        .expect("adaptive context budget should be configurable");
    config.adaptive_context_budget = Some(policy());
}

async fn rollout_items(test: &TestCodex) -> Result<std::sync::Arc<Vec<RolloutItem>>> {
    test.codex.flush_rollout().await?;
    let rollout_path = test.codex.rollout_path().expect("rollout path");
    let InitialHistory::Resumed(rollout) =
        RolloutRecorder::get_rollout_history(&rollout_path).await?
    else {
        panic!("materialized rollout should load as resumed history");
    };
    Ok(rollout.history)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn auto_compaction_feedback_persists_expanded_budget_and_reports_it() -> Result<()> {
    let server = start_mock_server().await;
    let summary = vec!["dense-summary"; 80].join(" ");
    let responses = mount_sse_sequence(
        &server,
        vec![
            sse(vec![
                ev_assistant_message("first", "first response"),
                ev_completed_with_tokens("first-response", /*total_tokens*/ 95),
            ]),
            sse(vec![
                ev_assistant_message("summary", &summary),
                ev_completed_with_tokens("compact-response", /*total_tokens*/ 20),
            ]),
            sse(vec![
                ev_assistant_message("second", "second response"),
                ev_completed_with_tokens("second-response", /*total_tokens*/ 30),
            ]),
        ],
    )
    .await;
    let mut provider = built_in_model_providers(/*openai_base_url*/ None)["openai"].clone();
    provider.name = "Adaptive context test provider".to_string();
    provider.base_url = Some(format!("{}/v1", server.uri()));
    provider.supports_websockets = false;
    provider.stream_max_retries = Some(0);

    let mut builder = test_codex().with_config(move |config| {
        config.model_provider = provider;
        configure_adaptive_context_budget(config);
    });
    let test = builder.build_with_auto_env(&server).await?;

    test.submit_turn("fill the first soft budget").await?;
    test.submit_turn("trigger automatic compaction").await?;
    let requests = responses.requests();
    assert_eq!(requests.len(), 3);
    let token_usage = test
        .codex
        .token_usage_info()
        .await
        .expect("token usage should be available");
    assert_eq!(token_usage.target_context_budget_tokens, Some(200));

    let rollout = rollout_items(&test).await?;
    let session_checkpoint = rollout.iter().find_map(|item| match item {
        RolloutItem::SessionMeta(meta) => meta.meta.adaptive_context_budget.as_ref(),
        _ => None,
    });
    assert_eq!(
        session_checkpoint.map(|checkpoint| checkpoint.state.target_context_budget_tokens),
        Some(100)
    );
    let checkpoint = rollout.iter().rev().find_map(|item| match item {
        RolloutItem::Compacted(compacted) => compacted.adaptive_context_budget.as_ref(),
        _ => None,
    });
    assert_eq!(
        checkpoint.map(|checkpoint| checkpoint.state.target_context_budget_tokens),
        Some(200)
    );
    assert!(rollout.iter().any(|item| {
        matches!(
            item,
            RolloutItem::EventMsg(EventMsg::TokenCount(event))
                if event.info.as_ref().and_then(|info| info.target_context_budget_tokens)
                    == Some(200)
        )
    }));
    let compacted_index = rollout
        .iter()
        .position(|item| matches!(item, RolloutItem::Compacted(_)))
        .expect("compaction checkpoint");
    let post_compaction = &rollout[compacted_index + 1..];
    let next_response = post_compaction
        .iter()
        .position(|item| matches!(item, RolloutItem::ResponseItem(_)))
        .unwrap_or(post_compaction.len());
    let post_compaction_token_events = post_compaction[..next_response]
        .iter()
        .filter(|item| matches!(item, RolloutItem::EventMsg(EventMsg::TokenCount(_))))
        .count();
    assert_eq!(post_compaction_token_events, 1);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn remote_v2_auto_compaction_uses_adaptive_threshold() -> Result<()> {
    let server = start_mock_server().await;
    let responses = mount_sse_sequence(
        &server,
        vec![
            sse(vec![
                ev_assistant_message("first", "first response"),
                ev_completed_with_tokens("first-response", /*total_tokens*/ 95),
            ]),
            sse(vec![
                json!({
                    "type": "response.output_item.done",
                    "item": {
                        "type": "compaction",
                        "encrypted_content": vec!["dense-summary"; 80].join(" "),
                    }
                }),
                ev_completed("compact-response"),
            ]),
            sse(vec![
                ev_assistant_message("second", "second response"),
                ev_completed_with_tokens("second-response", /*total_tokens*/ 30),
            ]),
        ],
    )
    .await;
    let mut provider = built_in_model_providers(/*openai_base_url*/ None)["openai"].clone();
    provider.base_url = Some(format!("{}/v1", server.uri()));
    provider.supports_websockets = false;
    let mut builder = test_codex().with_config(move |config| {
        config.model_provider = provider;
        configure_adaptive_context_budget(config);
        config
            .features
            .enable(Feature::RemoteCompactionV2)
            .expect("remote compaction v2 should be configurable");
    });
    let test = builder.build_with_auto_env(&server).await?;

    test.submit_turn("fill the first soft budget").await?;
    test.submit_turn("trigger remote v2 compaction").await?;

    let requests = responses.requests();
    assert_eq!(requests.len(), 3);
    assert_eq!(
        requests[1].input().last(),
        Some(&json!({"type": "compaction_trigger"}))
    );
    assert_eq!(
        test.codex
            .token_usage_info()
            .await
            .and_then(|info| info.target_context_budget_tokens),
        Some(200)
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn legacy_remote_auto_compaction_uses_adaptive_threshold() -> Result<()> {
    let server = start_mock_server().await;
    let responses = mount_sse_sequence(
        &server,
        vec![
            sse(vec![
                ev_assistant_message("first", "first response"),
                ev_completed_with_tokens("first-response", /*total_tokens*/ 95),
            ]),
            sse(vec![
                ev_assistant_message("second", "second response"),
                ev_completed_with_tokens("second-response", /*total_tokens*/ 30),
            ]),
        ],
    )
    .await;
    let compact_mock = mount_compact_json_once(
        &server,
        json!({
            "output": [{
                "type": "compaction",
                "encrypted_content": vec!["dense-summary"; 80].join(" "),
            }]
        }),
    )
    .await;
    let mut provider = built_in_model_providers(/*openai_base_url*/ None)["openai"].clone();
    provider.base_url = Some(format!("{}/v1", server.uri()));
    provider.supports_websockets = false;
    let mut builder = test_codex().with_config(move |config| {
        config.model_provider = provider;
        configure_adaptive_context_budget(config);
        config
            .features
            .disable(Feature::RemoteCompactionV2)
            .expect("remote compaction v2 should be configurable");
    });
    let test = builder.build_with_auto_env(&server).await?;

    test.submit_turn("fill the first soft budget").await?;
    test.submit_turn("trigger legacy remote compaction").await?;

    assert_eq!(responses.requests().len(), 2);
    assert_eq!(
        compact_mock.single_request().path(),
        "/v1/responses/compact"
    );
    assert_eq!(
        test.codex
            .token_usage_info()
            .await
            .and_then(|info| info.target_context_budget_tokens),
        Some(200)
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn manual_compaction_copies_adaptive_state_without_feedback() -> Result<()> {
    let server = start_mock_server().await;
    mount_sse_sequence(
        &server,
        vec![
            sse(vec![
                ev_assistant_message("first", "first response"),
                ev_completed_with_tokens("first-response", /*total_tokens*/ 30),
            ]),
            sse(vec![
                ev_assistant_message("summary", &vec!["dense-summary"; 80].join(" ")),
                ev_completed_with_tokens("compact-response", /*total_tokens*/ 20),
            ]),
        ],
    )
    .await;
    let mut provider = built_in_model_providers(/*openai_base_url*/ None)["openai"].clone();
    provider.name = "Adaptive context test provider".to_string();
    provider.base_url = Some(format!("{}/v1", server.uri()));
    provider.supports_websockets = false;
    provider.stream_max_retries = Some(0);
    let mut builder = test_codex().with_config(move |config| {
        config.model_provider = provider;
        configure_adaptive_context_budget(config);
    });
    let test = builder.build_with_auto_env(&server).await?;

    test.submit_turn("before manual compaction").await?;
    test.codex.submit(Op::Compact).await?;
    wait_for_event(&test.codex, |event| {
        matches!(event, EventMsg::TurnComplete(_))
    })
    .await;

    let rollout = rollout_items(&test).await?;
    let checkpoint = rollout.iter().rev().find_map(|item| match item {
        RolloutItem::Compacted(compacted) => compacted.adaptive_context_budget.as_ref(),
        _ => None,
    });
    assert_eq!(
        checkpoint.map(|checkpoint| checkpoint.state.clone()),
        Some(codex_protocol::protocol::AdaptiveContextBudgetState {
            policy_version: ADAPTIVE_CONTEXT_BUDGET_POLICY_VERSION,
            target_context_budget_tokens: 100,
            ambiguous_compaction_count: 0,
        })
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn failed_auto_compaction_does_not_persist_or_change_adaptive_state() -> Result<()> {
    let server = start_mock_server().await;
    mount_sse_sequence(
        &server,
        vec![
            sse(vec![
                ev_assistant_message("first", "first response"),
                ev_completed_with_tokens("first-response", /*total_tokens*/ 95),
            ]),
            sse_failed("compact-failed", "server_error", "compaction failed"),
        ],
    )
    .await;
    let mut provider = built_in_model_providers(/*openai_base_url*/ None)["openai"].clone();
    provider.name = "Adaptive context test provider".to_string();
    provider.base_url = Some(format!("{}/v1", server.uri()));
    provider.supports_websockets = false;
    provider.stream_max_retries = Some(0);
    let mut builder = test_codex().with_config(move |config| {
        config.model_provider = provider;
        configure_adaptive_context_budget(config);
    });
    let test = builder.build_with_auto_env(&server).await?;

    test.submit_turn("fill the first soft budget").await?;
    test.codex
        .start_or_steer_turn(TurnInputRequest::user_input(vec![UserInput::Text {
            text: "trigger failed compaction".to_string(),
            text_elements: Vec::new(),
        }]))
        .await?;
    wait_for_event(&test.codex, |event| matches!(event, EventMsg::Error(_))).await;

    assert_eq!(
        test.codex
            .token_usage_info()
            .await
            .and_then(|info| info.target_context_budget_tokens),
        Some(100)
    );
    assert!(
        !rollout_items(&test)
            .await?
            .iter()
            .any(|item| matches!(item, RolloutItem::Compacted(_)))
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feature_disabled_keeps_existing_request_sequence_and_usage_shape() -> Result<()> {
    let server = start_mock_server().await;
    let responses = mount_sse_sequence(
        &server,
        vec![
            sse(vec![
                ev_assistant_message("first", "first response"),
                ev_completed_with_tokens("first-response", /*total_tokens*/ 95),
            ]),
            sse(vec![
                ev_assistant_message("second", "second response"),
                ev_completed_with_tokens("second-response", /*total_tokens*/ 30),
            ]),
        ],
    )
    .await;
    let mut provider = built_in_model_providers(/*openai_base_url*/ None)["openai"].clone();
    provider.name = "Adaptive context test provider".to_string();
    provider.base_url = Some(format!("{}/v1", server.uri()));
    provider.supports_websockets = false;
    let mut builder = test_codex().with_config(move |config| {
        config.model_provider = provider;
        config.model_context_window = Some(1_000);
    });
    let test = builder.build_with_auto_env(&server).await?;

    test.submit_turn("first turn").await?;
    test.submit_turn("second turn").await?;

    assert_eq!(responses.requests().len(), 2);
    assert_eq!(
        test.codex
            .token_usage_info()
            .await
            .and_then(|info| info.target_context_budget_tokens),
        None
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn incompatible_model_switch_is_atomic_but_future_tiers_do_not_block() -> Result<()> {
    let server = start_mock_server().await;
    let mut large = test_model_info(
        "large-model",
        "Large Model",
        "large",
        vec![InputModality::Text],
    );
    large.context_window = Some(100);
    large.max_context_window = Some(400);
    let mut small = large.clone();
    small.slug = "small-model".to_string();
    small.display_name = "Small Model".to_string();
    small.max_context_window = Some(100);
    let mut compatible = large.clone();
    compatible.slug = "compatible-model".to_string();
    compatible.display_name = "Compatible Model".to_string();
    compatible.max_context_window = Some(250);
    mount_models_once(
        &server,
        ModelsResponse {
            models: vec![large, small, compatible],
        },
    )
    .await;
    let mut builder = test_codex()
        .with_auth(CodexAuth::create_dummy_chatgpt_auth_for_testing())
        .with_config(|config| {
            config.model = Some("large-model".to_string());
            config
                .features
                .enable(Feature::AdaptiveContextBudget)
                .expect("adaptive context budget should be configurable");
            config.adaptive_context_budget = Some(AdaptiveContextBudgetPolicy {
                context_window_tiers: vec![200, 300],
                ..policy()
            });
        });
    let test = builder.build_with_auto_env(&server).await?;
    test.thread_manager
        .get_models_manager()
        .list_models(
            RefreshStrategy::Online,
            codex_core::test_support::default_http_client_factory(),
        )
        .await;

    let submission_id = test
        .codex
        .submit(Op::ThreadSettings {
            thread_settings: ThreadSettingsOverrides {
                model: Some("small-model".to_string()),
                ..Default::default()
            },
        })
        .await?;
    let rejected = loop {
        let event = test.codex.next_event().await.expect("settings event");
        if event.id == submission_id {
            break event.msg;
        }
    };
    let EventMsg::Error(error) = rejected else {
        panic!("incompatible switch should fail");
    };
    assert_eq!(
        error.message,
        "invalid thread settings override: adaptive context budget target 200 exceeds model small-model maximum 100"
    );
    assert_eq!(
        test.codex.thread_settings_snapshot().await.model,
        "large-model"
    );

    core_test_support::submit_thread_settings(
        &test.codex,
        ThreadSettingsOverrides {
            model: Some("compatible-model".to_string()),
            ..Default::default()
        },
    )
    .await?;
    assert_eq!(
        test.codex.thread_settings_snapshot().await.model,
        "compatible-model"
    );
    Ok(())
}
