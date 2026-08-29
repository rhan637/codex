use super::*;
use crate::responses_metadata::CompactionTurnMetadata;
use codex_analytics::CompactionImplementation;
use codex_analytics::CompactionPhase;
use codex_analytics::CompactionReason;
use codex_analytics::CompactionTrigger;
use pretty_assertions::assert_eq;

fn policy() -> AdaptiveContextBudgetPolicy {
    AdaptiveContextBudgetPolicy {
        policy_version: ADAPTIVE_CONTEXT_BUDGET_POLICY_VERSION,
        context_window_tiers: vec![1_000, 2_000, 4_000],
        keep_below_percent: 40,
        expand_at_or_above_percent: 60,
        ambiguous_compactions_before_expand: 2,
    }
}

fn expected_state(target: i64, ambiguous_count: u32) -> AdaptiveContextBudgetState {
    AdaptiveContextBudgetState {
        policy_version: ADAPTIVE_CONTEXT_BUDGET_POLICY_VERSION,
        target_context_budget_tokens: target,
        ambiguous_compaction_count: ambiguous_count,
    }
}

#[test]
fn feedback_boundaries_and_ambiguous_accumulation_are_exact() {
    let mut runtime = AdaptiveContextBudgetRuntime::initial(policy());

    runtime.apply_compaction_feedback_with_limits(400, Some(4_000), None);
    assert_eq!(runtime.checkpoint().state, expected_state(1_000, 1));

    runtime.apply_compaction_feedback_with_limits(599, Some(4_000), None);
    assert_eq!(runtime.checkpoint().state, expected_state(2_000, 0));

    runtime.apply_compaction_feedback_with_limits(1_200, Some(4_000), None);
    assert_eq!(runtime.checkpoint().state, expected_state(4_000, 0));
}

#[test]
fn low_feedback_clears_an_ambiguous_streak() {
    let mut runtime = AdaptiveContextBudgetRuntime::initial(policy());
    runtime.apply_compaction_feedback_with_limits(500, Some(4_000), None);
    runtime.apply_compaction_feedback_with_limits(399, Some(4_000), None);

    assert_eq!(runtime.checkpoint().state, expected_state(1_000, 0));
}

#[test]
fn expansion_respects_max_model_and_catalog_caps() {
    let mut max_tier = AdaptiveContextBudgetRuntime::initial(policy());
    max_tier.checkpoint.state.target_context_budget_tokens = 4_000;
    max_tier.apply_compaction_feedback_with_limits(2_400, Some(4_000), None);
    assert_eq!(max_tier.checkpoint().state, expected_state(4_000, 0));

    let mut model_capped = AdaptiveContextBudgetRuntime::initial(policy());
    model_capped.apply_compaction_feedback_with_limits(600, Some(1_000), None);
    assert_eq!(model_capped.checkpoint().state, expected_state(1_000, 0));

    let mut catalog_capped = AdaptiveContextBudgetRuntime::initial(policy());
    catalog_capped.apply_compaction_feedback_with_limits(600, Some(4_000), Some(500));
    assert_eq!(catalog_capped.checkpoint().state, expected_state(1_000, 0));
}

#[test]
fn feedback_math_is_safe_at_i64_limits() {
    let policy = AdaptiveContextBudgetPolicy {
        context_window_tiers: vec![i64::MAX - 1, i64::MAX],
        ..policy()
    };
    let mut runtime = AdaptiveContextBudgetRuntime::initial(policy);

    runtime.apply_compaction_feedback_with_limits(i64::MAX, Some(i64::MAX), None);

    assert_eq!(runtime.checkpoint().state, expected_state(i64::MAX, 0));
}

#[test]
fn compatibility_checks_only_the_current_target() {
    let mut runtime = AdaptiveContextBudgetRuntime::initial(policy());
    assert_eq!(
        runtime.ensure_model_compatible_with_maximum(Some(1_000), "small"),
        Ok(())
    );

    runtime.checkpoint.state.target_context_budget_tokens = 2_000;
    assert_eq!(
        runtime.ensure_model_compatible_with_maximum(Some(1_000), "small"),
        Err("adaptive context budget target 2000 exceeds model small maximum 1000".to_string())
    );
}

#[test]
fn invalid_and_unknown_checkpoints_block_compatibility() {
    let mut checkpoint = AdaptiveContextBudgetRuntime::initial(policy()).checkpoint();
    checkpoint.policy.policy_version = ADAPTIVE_CONTEXT_BUDGET_POLICY_VERSION + 1;
    checkpoint.state.policy_version = checkpoint.policy.policy_version;
    let runtime = AdaptiveContextBudgetRuntime::restored(checkpoint, None);
    assert_eq!(
        runtime.ensure_model_compatible_with_maximum(Some(4_000), "model"),
        Err("unsupported adaptive context budget policy version 2".to_string())
    );

    let mut checkpoint = AdaptiveContextBudgetRuntime::initial(policy()).checkpoint();
    checkpoint.state.target_context_budget_tokens = 3_000;
    let runtime = AdaptiveContextBudgetRuntime::restored(checkpoint, None);
    assert_eq!(
        runtime.ensure_model_compatible_with_maximum(Some(4_000), "model"),
        Err("invalid adaptive context budget checkpoint".to_string())
    );
}

#[test]
fn only_automatic_context_limit_compaction_changes_feedback_state() {
    let metadata = |trigger, reason| {
        Some(CompactionTurnMetadata::new(
            trigger,
            reason,
            CompactionImplementation::Responses,
            CompactionPhase::PreTurn,
        ))
    };

    assert!(should_apply_compaction_feedback(metadata(
        CompactionTrigger::Auto,
        CompactionReason::ContextLimit,
    )));
    for compaction in [
        metadata(CompactionTrigger::Manual, CompactionReason::UserRequested),
        metadata(CompactionTrigger::Auto, CompactionReason::ModelDownshift),
        metadata(CompactionTrigger::Auto, CompactionReason::CompHashChanged),
        None,
    ] {
        assert!(!should_apply_compaction_feedback(compaction));
    }
}

#[test]
fn legacy_restore_selects_first_non_overflowing_supported_tier() {
    let mut runtime = AdaptiveContextBudgetRuntime::initial(policy());
    runtime.restore_legacy_compatible_tier_with_limits(950, Some(4_000), None);
    assert_eq!(runtime.checkpoint().state, expected_state(2_000, 0));

    runtime.restore_legacy_compatible_tier_with_limits(3_900, Some(2_000), None);
    assert_eq!(runtime.checkpoint().state, expected_state(2_000, 0));
}
