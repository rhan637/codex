use crate::config::Config;
use crate::responses_metadata::CompactionTurnMetadata;
use codex_analytics::CompactionReason;
use codex_analytics::CompactionTrigger;
use codex_protocol::openai_models::ModelInfo;
use codex_protocol::protocol::ADAPTIVE_CONTEXT_BUDGET_POLICY_VERSION;
use codex_protocol::protocol::AdaptiveContextBudgetCheckpoint;
use codex_protocol::protocol::AdaptiveContextBudgetPolicy;
use codex_protocol::protocol::AdaptiveContextBudgetState;

#[derive(Clone, Debug)]
pub(crate) struct AdaptiveContextBudgetRuntime {
    checkpoint: AdaptiveContextBudgetCheckpoint,
    checkpoint_error: Option<String>,
}

impl AdaptiveContextBudgetRuntime {
    pub(crate) fn initial(policy: AdaptiveContextBudgetPolicy) -> Self {
        let target_context_budget_tokens = policy.context_window_tiers[0];
        Self {
            checkpoint: AdaptiveContextBudgetCheckpoint {
                state: AdaptiveContextBudgetState {
                    policy_version: policy.policy_version,
                    target_context_budget_tokens,
                    ambiguous_compaction_count: 0,
                },
                policy,
            },
            checkpoint_error: None,
        }
    }

    pub(crate) fn restored(
        checkpoint: AdaptiveContextBudgetCheckpoint,
        checkpoint_error: Option<String>,
    ) -> Self {
        let checkpoint_error = checkpoint_error.or_else(|| validate_checkpoint(&checkpoint).err());
        Self {
            checkpoint,
            checkpoint_error,
        }
    }

    pub(crate) fn checkpoint(&self) -> AdaptiveContextBudgetCheckpoint {
        self.checkpoint.clone()
    }

    pub(crate) fn target_context_budget_tokens(&self) -> i64 {
        self.checkpoint.state.target_context_budget_tokens
    }

    pub(crate) fn ensure_model_compatible(
        &self,
        config: &Config,
        model_info: &ModelInfo,
    ) -> Result<(), String> {
        self.ensure_model_compatible_with_maximum(
            runtime_max_context_window(config, model_info),
            &model_info.slug,
        )
    }

    fn ensure_model_compatible_with_maximum(
        &self,
        maximum: Option<i64>,
        model: &str,
    ) -> Result<(), String> {
        if let Some(error) = &self.checkpoint_error {
            return Err(error.clone());
        }
        let Some(maximum) = maximum else {
            return Ok(());
        };
        let target = self.target_context_budget_tokens();
        if target > maximum {
            return Err(format!(
                "adaptive context budget target {target} exceeds model {model} maximum {maximum}"
            ));
        }
        Ok(())
    }

    pub(crate) fn restore_legacy_compatible_tier(
        &mut self,
        active_context_tokens: i64,
        config: &Config,
        model_info: &ModelInfo,
    ) {
        self.restore_legacy_compatible_tier_with_limits(
            active_context_tokens,
            runtime_max_context_window(config, model_info),
            catalog_auto_compact_limit(model_info),
        );
    }

    fn restore_legacy_compatible_tier_with_limits(
        &mut self,
        active_context_tokens: i64,
        maximum: Option<i64>,
        catalog_limit: Option<i64>,
    ) {
        let maximum = maximum.unwrap_or(i64::MAX);
        let policy = &self.checkpoint.policy;
        let compatible_tier = policy
            .context_window_tiers
            .iter()
            .copied()
            .filter(|tier| *tier <= maximum)
            .find(|tier| active_context_tokens < effective_compact_limit(*tier, catalog_limit))
            .or_else(|| {
                policy
                    .context_window_tiers
                    .iter()
                    .copied()
                    .rfind(|tier| *tier <= maximum)
            });
        if let Some(target) = compatible_tier {
            self.checkpoint.state.target_context_budget_tokens = target;
            self.checkpoint.state.ambiguous_compaction_count = 0;
        }
    }

    pub(crate) fn apply_compaction_feedback(
        &mut self,
        active_context_tokens_after: i64,
        config: &Config,
        model_info: &ModelInfo,
    ) {
        self.apply_compaction_feedback_with_limits(
            active_context_tokens_after,
            runtime_max_context_window(config, model_info),
            catalog_auto_compact_limit(model_info),
        );
    }

    fn apply_compaction_feedback_with_limits(
        &mut self,
        active_context_tokens_after: i64,
        maximum: Option<i64>,
        catalog_limit: Option<i64>,
    ) {
        if self.checkpoint_error.is_some() {
            return;
        }
        let old_target = self.checkpoint.state.target_context_budget_tokens;
        let old_count = self.checkpoint.state.ambiguous_compaction_count;
        let policy = &self.checkpoint.policy;
        let usage = i128::from(active_context_tokens_after.max(0));
        let budget = i128::from(old_target);
        let keep = i128::from(policy.keep_below_percent);
        let expand = i128::from(policy.expand_at_or_above_percent);
        let mut should_expand = false;
        let decision = if usage * 100 < keep * budget {
            self.checkpoint.state.ambiguous_compaction_count = 0;
            "keep"
        } else if usage * 100 >= expand * budget {
            self.checkpoint.state.ambiguous_compaction_count = 0;
            should_expand = true;
            "expand"
        } else {
            let count = old_count.saturating_add(1);
            if count >= policy.ambiguous_compactions_before_expand {
                self.checkpoint.state.ambiguous_compaction_count = 0;
                should_expand = true;
                "expand_ambiguous"
            } else {
                self.checkpoint.state.ambiguous_compaction_count = count;
                "ambiguous"
            }
        };

        let mut max_tier = false;
        let mut catalog_capped = false;
        let mut model_window_capped = false;
        if should_expand {
            let Some(current_index) = policy
                .context_window_tiers
                .iter()
                .position(|tier| *tier == old_target)
            else {
                return;
            };
            if let Some(next_target) = policy.context_window_tiers.get(current_index + 1).copied() {
                if maximum.is_some_and(|maximum| next_target > maximum) {
                    model_window_capped = true;
                } else if effective_compact_limit(next_target, catalog_limit)
                    <= effective_compact_limit(old_target, catalog_limit)
                {
                    catalog_capped = true;
                } else {
                    self.checkpoint.state.target_context_budget_tokens = next_target;
                }
            } else {
                max_tier = true;
            }
        }

        tracing::info!(
            target_context_budget_tokens = old_target,
            active_context_tokens_after,
            old_target_context_budget_tokens = old_target,
            new_target_context_budget_tokens = self.checkpoint.state.target_context_budget_tokens,
            old_ambiguous_compaction_count = old_count,
            new_ambiguous_compaction_count = self.checkpoint.state.ambiguous_compaction_count,
            model_max_context_window = maximum,
            catalog_auto_compact_limit = catalog_limit,
            effective_compact_limit = effective_compact_limit(old_target, catalog_limit),
            keep_below_percent = policy.keep_below_percent,
            expand_at_or_above_percent = policy.expand_at_or_above_percent,
            max_tier,
            catalog_capped,
            model_window_capped,
            decision,
            "updated adaptive context budget after compaction"
        );
    }
}

pub(crate) fn runtime_max_context_window(config: &Config, model_info: &ModelInfo) -> Option<i64> {
    if config.model_context_window.is_some() {
        model_info
            .context_window
            .or_else(|| model_info.resolved_context_window())
    } else {
        model_info
            .max_context_window
            .or_else(|| model_info.resolved_context_window())
    }
}

pub(crate) fn usable_runtime_context_window(
    config: &Config,
    model_info: &ModelInfo,
) -> Option<i64> {
    runtime_max_context_window(config, model_info).map(|context_window| {
        context_window.saturating_mul(model_info.effective_context_window_percent) / 100
    })
}

pub(crate) fn catalog_auto_compact_limit(model_info: &ModelInfo) -> Option<i64> {
    model_info.auto_compact_token_limit
}

pub(crate) fn effective_compact_limit(budget: i64, catalog_limit: Option<i64>) -> i64 {
    let soft_limit = i64::try_from(i128::from(budget).saturating_mul(9) / 10).unwrap_or(i64::MAX);
    catalog_limit.map_or(soft_limit, |limit| soft_limit.min(limit))
}

pub(crate) fn should_apply_compaction_feedback(compaction: Option<CompactionTurnMetadata>) -> bool {
    matches!(
        compaction.map(|compaction| (compaction.trigger(), compaction.reason())),
        Some((CompactionTrigger::Auto, CompactionReason::ContextLimit))
    )
}

pub(crate) fn validate_checkpoint(
    checkpoint: &AdaptiveContextBudgetCheckpoint,
) -> Result<(), String> {
    let policy = &checkpoint.policy;
    let state = &checkpoint.state;
    if policy.policy_version != ADAPTIVE_CONTEXT_BUDGET_POLICY_VERSION {
        return Err(format!(
            "unsupported adaptive context budget policy version {}",
            policy.policy_version
        ));
    }
    if state.policy_version != policy.policy_version {
        return Err("adaptive context budget policy/state version mismatch".to_string());
    }
    if policy.context_window_tiers.is_empty()
        || policy.context_window_tiers.iter().any(|tier| *tier <= 0)
        || policy
            .context_window_tiers
            .windows(2)
            .any(|tiers| tiers[0] >= tiers[1])
        || policy.keep_below_percent == 0
        || policy.keep_below_percent >= policy.expand_at_or_above_percent
        || policy.expand_at_or_above_percent > 100
        || policy.ambiguous_compactions_before_expand == 0
        || !policy
            .context_window_tiers
            .contains(&state.target_context_budget_tokens)
        || state.ambiguous_compaction_count >= policy.ambiguous_compactions_before_expand
    {
        return Err("invalid adaptive context budget checkpoint".to_string());
    }
    Ok(())
}

#[cfg(test)]
#[path = "adaptive_context_budget_tests.rs"]
mod tests;
