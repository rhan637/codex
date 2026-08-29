# Adaptive Context Budget Usage Guide / 使用说明

## Feature status / 功能状态

`adaptive_context_budget` is an experimental feature provided by this branch and is disabled by default.

`adaptive_context_budget` 是当前分支提供的实验功能，默认关闭。

It does not enlarge the model's real context window or bypass model catalog limits. It only controls when Codex performs automatic compaction: it starts with a smaller soft budget and decides whether to move to a larger soft-budget tier according to the context that actually remains after each compaction.

它不会扩大模型真实的上下文窗口，也不会绕过模型 catalog 的限制。它只控制 Codex 何时自动执行 compaction：先从较小的软预算开始，再根据每次 compaction 后实际剩余的上下文量决定是否升入更大的软预算档位。

The default policy is:

默认策略为：

```text
272K → 487K → 872K
```

The governor only moves upward; it does not automatically move down.

只支持升档，不会自动降档。

## Quick enablement / 快速启用

Edit the `config.toml` used by the running Codex process. The file is usually located at:

编辑当前 Codex 实际使用的 `config.toml`。通常文件位于：

```text
$CODEX_HOME/config.toml
```

If `CODEX_HOME` is not explicitly set, the usual location is:

如果没有显式设置 `CODEX_HOME`，通常使用：

```text
~/.codex/config.toml
```

Add:

加入：

```toml
[features.adaptive_context_budget]
enabled = true
context_window_tiers = [272000, 487000, 872000]
keep_below_percent = 45
expand_at_or_above_percent = 65
ambiguous_compactions_before_expand = 2
```

Users can adjust all of these fields, and the values above are also the current code defaults. Therefore, the following minimal configuration also enables the default policy:

这些字段都可以由用户调整；上面的值也是当前代码默认值。因此只写下面的配置也能启用默认策略：

```toml
[features.adaptive_context_budget]
enabled = true
```

## Conflicts to check before enabling / 启用前必须检查的冲突

The feature cannot be enabled together with the following settings.

启用时不能同时使用以下配置。

### 1. Token Budget feature

Do not enable either of the following at the same time:

不能同时启用：

```toml
[features.token_budget]
enabled = true
```

or:

或：

```toml
[features]
token_budget = true
```

### 2. Explicit automatic compaction limit / 显式自动压缩限制

Do not explicitly set the following top-level field:

不能在顶层显式设置：

```toml
model_auto_compact_token_limit = 900000
```

If the field exists in the current configuration, remove it first. Adaptive Context Budget calculates its own soft automatic-compaction threshold.

如果当前配置中存在该字段，需要先删除它。Adaptive Context Budget 会自行计算自动压缩软阈值。

### 3. `body_after_prefix` scope

Do not set:

不能设置：

```toml
model_auto_compact_token_limit_scope = "body_after_prefix"
```

Adaptive Context Budget uses the complete active context as its feedback basis. You can omit this field or explicitly set it to:

Adaptive Context Budget 使用完整活动上下文作为反馈基础。可以省略该字段，也可以显式设置：

```toml
model_auto_compact_token_limit_scope = "total"
```

### Catalog limits are not a conflict / Catalog 限制不属于冲突

The model catalog's built-in auto-compact limit does not need to be removed and does not cause a configuration error. It remains an upper bound in the effective-threshold calculation.

模型 catalog 自带的 auto-compact limit 不需要删除，也不会导致配置错误。它会作为不可突破的上限参与实际阈值计算。

## Configuration fields / 配置字段

### `enabled`

```toml
enabled = true
```

Enables or disables the feature. The default is disabled.

启用或关闭功能。默认关闭。

### `context_window_tiers`

```toml
context_window_tiers = [272000, 487000, 872000]
```

Defines the allowed soft-budget tiers in tokens.

允许使用的软预算档位，单位为 token。

Requirements:

要求：

- At least one tier is required.
  至少有一个档位。
- Every value must be greater than zero.
  每个值必须大于零。
- Values must be strictly increasing.
  必须严格递增。
- The first tier for a new thread must not exceed the current model's runtime maximum window.
  新线程的第一档必须不超过当前模型运行时最大窗口。

Custom tiers are supported, for example:

可以使用自定义档位，例如：

```toml
context_window_tiers = [200000, 400000, 800000]
```

Future tiers that exceed the current model's capability may remain in the policy. Codex only moves upward when the current model actually supports the next tier.

超过当前模型能力的未来档位可以保留在策略中；Codex 只会在当前模型真正支持下一档时升档。

### `keep_below_percent`

```toml
keep_below_percent = 45
```

After a successful compaction, if:

成功 compaction 后，如果：

```text
U_after / B < 45%
```

Codex considers the compaction effective enough, keeps the current tier, and resets the middle-range counter.

Codex 认为压缩效果足够好，保持当前档位，并把中间区累计次数清零。

Where:

其中：

- `B` is the current soft budget before compaction.
  `B` 是 compaction 前当前软预算。
- `U_after` is the complete context usage recomputed after the compacted history is installed.
  `U_after` 是安装压缩历史后重新计算的完整上下文占用。

### `expand_at_or_above_percent`

```toml
expand_at_or_above_percent = 65
```

After a successful compaction, if:

成功 compaction 后，如果：

```text
U_after / B >= 65%
```

Codex considers the compaction insufficient and immediately attempts to move to the next tier.

Codex 认为压缩效果不足，立即尝试升入下一档。

### `ambiguous_compactions_before_expand`

```toml
ambiguous_compactions_before_expand = 2
```

If:

如果：

```text
45% <= U_after / B < 65%
```

the result falls into the middle range. Codex attempts to expand after this happens consecutively for the configured number of compactions.

本次结果进入中间区。连续达到配置次数后，Codex 尝试升档。

The default value `2` means that two consecutive middle-range results are required.

默认值 `2` 表示中间区结果需要连续出现两次。

## Automatic compaction threshold / 自动压缩阈值

Let the current soft budget be `B`.

当前软预算记作 `B`。

Adaptive Context Budget calculates:

Adaptive Context Budget 计算：

```text
soft_compact_limit = floor(0.9 × B)

effective_compact_limit =
    min(soft_compact_limit, catalog_auto_compact_limit)
```

If the model catalog imposes no additional limit, the default tiers map to:

如果模型 catalog 没有额外限制，则默认档位对应：

| Current soft budget / 当前软预算 | Automatic compaction threshold / 自动压缩线 |
|---:|---:|
| 272,000 | 244,800 |
| 487,000 | 438,300 |
| 872,000 | 784,800 |

The model's hard window continues to govern input safety limits, truncation, and token accounting. The soft budget does not replace the model's hard window.

模型硬窗口仍负责输入安全限制、裁剪和 token accounting；软预算不会替代模型硬窗口。

## Which compactions update the tier / 哪些 compaction 会更新档位

Only the following kind of compaction updates the Governor:

只有以下 compaction 会更新 Governor：

```text
trigger = Auto
reason = ContextLimit
```

The following operations copy the current checkpoint unchanged into the new `CompactedItem`; they do not change the tier or middle-range counter:

以下操作只把当前 checkpoint 原样复制到新的 `CompactedItem`，不会改变档位或累计次数：

- A user-triggered `/compact`.
  用户手动 `/compact`。
- Compaction triggered by a model window downshift.
  模型降窗触发的 compaction。
- Compaction triggered by a `comp_hash` change.
  `comp_hash` 变化触发的 compaction。
- Any other compaction that is not an automatic context-limit compaction.
  其他不属于自动上下文限制的压缩。

A failed compaction does not update in-memory state or write a successful checkpoint.

失败的 compaction 不更新内存状态，也不会写入成功 checkpoint。

## Expansion limits / 升档限制

Even when the feedback requests expansion, all of the following conditions must hold:

即使反馈要求升档，也必须满足：

1. The policy contains a next tier.
   策略中存在下一档。
2. The current model's runtime maximum window supports the next tier.
   当前模型运行时最大窗口支持下一档。
3. The next tier's effective automatic-compaction threshold is higher than the current tier's threshold.
   下一档的实际自动压缩线高于当前档。

The third condition handles a catalog cap. For example:

第三项用于处理 catalog cap。例如：

```text
current tier effective threshold = 200K
next tier effective threshold = 200K

当前档实际压缩线 = 200K
下一档实际压缩线 = 200K
```

In this case, expansion has no practical effect. Codex keeps the current tier and records `catalog_capped`.

这时升档没有实际效果，Codex 会保持当前档并记录 `catalog_capped`。

## New threads, resumed threads, and forks / 新线程、恢复线程和 fork

### New threads / 新线程

A new thread writes the complete policy and initial state into `SessionMeta`:

新线程把完整策略与初始状态写入 `SessionMeta`：

```text
target_context_budget_tokens = first tier / 第一档
ambiguous_compaction_count = 0
```

### Existing threads / 已有线程

The policy is pinned to the thread.

策略随线程固定。

Once a thread has stored an Adaptive Context Budget checkpoint, later global configuration changes do not alter that thread's existing policy. The new configuration only affects new threads.

线程一旦保存了 Adaptive Context Budget checkpoint，后续修改全局配置不会改变该线程的既有策略；新配置只影响新线程。

### Resume

When resuming a thread, Codex:

恢复线程时，Codex 会：

1. Replay the rollout.
   重放 rollout。
2. Apply rollback.
   应用 rollback。
3. Find the latest `CompactedItem` checkpoint that remains valid.
   找到最新仍然有效的 `CompactedItem` checkpoint。
4. Use the initial `SessionMeta` checkpoint if there is no compaction checkpoint.
   没有 compaction checkpoint 时使用 `SessionMeta` 初始 checkpoint。
5. Restore the current tier and middle-range counter.
   恢复当前档位与中间区累计次数。

### Fork

A forked child thread inherits the parent's current checkpoint that remains valid after rollback; it does not reset to the first tier. The inherited checkpoint is written into the child thread's own `SessionMeta`.

fork 子线程继承父线程 rollback 后仍然有效的当前 checkpoint，不会重置到第一档。继承结果会写入子线程自己的 `SessionMeta`。

### Legacy rollouts / 旧 rollout

If an old thread has no Adaptive Context Budget data at all, Codex creates a policy from the configuration available at resume time and selects a compatible tier according to the effective history usage. The complete policy is written into a checkpoint after the first successful compaction.

如果旧线程完全没有 Adaptive Context Budget 数据，Codex 会使用恢复时的配置生成策略，并根据有效历史占用选择一个兼容档位。第一次成功 compaction 后，完整策略会被写入 checkpoint。

## Model switching / 模型切换

Before switching models, Codex checks:

切换模型前，Codex 检查：

```text
current target_context_budget_tokens <= candidate model runtime maximum window

当前 target_context_budget_tokens <= 候选模型运行时最大窗口
```

If the current tier is incompatible, the model switch is atomically rejected:

如果当前档位不兼容，模型切换会被原子拒绝：

- The model remains unchanged.
  模型不变。
- The current tier remains unchanged.
  当前档位不变。
- The middle-range counter remains unchanged.
  中间区累计次数不变。
- No successful settings update is written.
  不写入成功的设置变更。

Future tiers that exceed the candidate model's capability do not block the switch. Codex only checks the current target tier.

只有未来档位超过候选模型能力不会阻止切换；Codex 只检查当前目标档位。

## How to confirm the feature is working / 如何确认功能正在工作

### 1. Check the TUI token meter / 查看 TUI token meter

When the feature is enabled, Codex reports the current soft budget through `target_context_budget_tokens`. The TUI context percentage uses this soft budget as its preferred denominator.

启用后，Codex 会通过 `target_context_budget_tokens` 报告当前软预算。TUI 的上下文百分比优先使用软预算作为分母。

When the feature is disabled, this field is `null`, and the TUI falls back to the original `model_context_window`.

功能关闭时该字段为 `null`，TUI 回退到原来的 `model_context_window`。

### 2. Use `/status` / 使用 `/status`

Check the denominator shown on the context-window line. Once the feature is enabled and token usage is available, the denominator should match the current soft-budget tier.

检查 context window 行显示的分母。启用并建立 token usage 后，分母应对应当前软预算档位。

### 3. Inspect trace logs / 查看 trace 日志

When launching the TUI from source, use:

从源码启动 TUI 时建议：

```bash
cd codex-rs
RUST_LOG=trace just codex -c log_dir=/tmp/codex-adaptive-context-budget-logs
```

Governor update logs include:

Governor 更新日志会包含：

```text
target_context_budget_tokens
active_context_tokens_after
old_target_context_budget_tokens
new_target_context_budget_tokens
old_ambiguous_compaction_count
new_ambiguous_compaction_count
model_max_context_window
catalog_auto_compact_limit
effective_compact_limit
keep_below_percent
expand_at_or_above_percent
max_tier
catalog_capped
model_window_capped
decision
```

### 4. Inspect the rollout / 查看 rollout

A successful compaction's rollout `CompactedItem` should contain:

成功 compaction 的 rollout `CompactedItem` 中应出现：

```text
adaptive_context_budget.policy
adaptive_context_budget.state
```

## Common configuration errors / 常见配置错误

### Conflict with an explicit compact limit / 与显式 compact limit 冲突

```text
features.adaptive_context_budget conflicts with model_auto_compact_token_limit
```

Remove the top-level `model_auto_compact_token_limit`.

删除顶层 `model_auto_compact_token_limit`。

### Conflict with Token Budget / 与 Token Budget 冲突

```text
features.adaptive_context_budget conflicts with features.token_budget
```

Disable the Token Budget feature.

关闭 Token Budget feature。

### Using body-after-prefix / 使用了 body-after-prefix

```text
features.adaptive_context_budget requires model_auto_compact_token_limit_scope = "total"
```

Remove this field or set it to `"total"`.

删除该字段或设置为 `"total"`。

### First tier exceeds the model maximum / 第一档超过模型最大窗口

```text
adaptive context budget target ... exceeds model ... maximum ...
```

Lower the first tier, increase the explicit model window subject to the catalog clamp, or select a model that supports the tier.

降低第一档、扩大经过 catalog clamp 的显式模型窗口，或选择支持该档位的模型。

## How to disable the feature / 如何关闭

Delete the configuration block or set:

删除配置块，或设置：

```toml
[features.adaptive_context_budget]
enabled = false
```

After the feature is disabled, existing Governor checkpoints in history are ignored and automatic compaction returns to its original behavior.

功能关闭后，历史中已有的 Governor checkpoint 会被忽略，自动压缩恢复原有行为。

## Capabilities not included / 当前不包含的能力

The current MVP does not include:

当前 MVP 不包含：

- Automatic downshifting.
  自动降档。
- A PRUNE phase.
  PRUNE 阶段。
- Semantic importance scoring.
  语义重要性评分。
- A new settings page or standalone UI.
  新的设置页面或独立 UI。
- Bypassing model catalog limits.
  绕过模型 catalog 上限。
- Modifying the internal contents of Responses API compaction items.
  修改 Responses API compaction item 的内部内容。

OpenAI defines the output of `/responses/compact` as compacted items that can be used to continue subsequent Responses requests. Adaptive Context Budget is the compaction-timing and persistence policy added by this branch on the Codex host side. Official API reference: <https://developers.openai.com/api/reference/java/resources/responses/methods/compact>.

OpenAI 官方把 `/responses/compact` 的输出定义为可供后续 Responses 请求继续使用的压缩 items；Adaptive Context Budget 是当前分支在 Codex host 侧增加的压缩时机与持久化策略。官方 API 参考：<https://developers.openai.com/api/reference/java/resources/responses/methods/compact>。
