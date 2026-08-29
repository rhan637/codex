# Adaptive Context Budget 使用说明

## 功能状态

`adaptive_context_budget` 是当前分支提供的实验功能，默认关闭。

它不会扩大模型真实的上下文窗口，也不会绕过模型 catalog 的限制。它只控制 Codex 何时自动执行 compaction：先从较小的软预算开始，再根据每次 compaction 后实际剩余的上下文量决定是否升入更大的软预算档位。

默认策略为：

```text
272K → 487K → 872K
```

只支持升档，不会自动降档。

## 快速启用

编辑当前 Codex 实际使用的 `config.toml`。通常文件位于：

```text
$CODEX_HOME/config.toml
```

如果没有显式设置 `CODEX_HOME`，通常使用：

```text
~/.codex/config.toml
```

加入：

```toml
[features.adaptive_context_budget]
enabled = true
context_window_tiers = [272000, 487000, 872000]
keep_below_percent = 45
expand_at_or_above_percent = 65
ambiguous_compactions_before_expand = 2
```

这些字段都可以由用户调整；上面的值也是当前代码默认值。因此只写下面的配置也能启用默认策略：

```toml
[features.adaptive_context_budget]
enabled = true
```

## 启用前必须检查的冲突

启用时不能同时使用以下配置。

### 1. Token Budget feature

不能同时启用：

```toml
[features.token_budget]
enabled = true
```

或：

```toml
[features]
token_budget = true
```

### 2. 显式自动压缩限制

不能在顶层显式设置：

```toml
model_auto_compact_token_limit = 900000
```

如果当前配置中存在该字段，需要先删除它。Adaptive Context Budget 会自行计算自动压缩软阈值。

### 3. `body_after_prefix` scope

不能设置：

```toml
model_auto_compact_token_limit_scope = "body_after_prefix"
```

Adaptive Context Budget 使用完整活动上下文作为反馈基础。可以省略该字段，也可以显式设置：

```toml
model_auto_compact_token_limit_scope = "total"
```

### Catalog 限制不属于冲突

模型 catalog 自带的 auto-compact limit 不需要删除，也不会导致配置错误。它会作为不可突破的上限参与实际阈值计算。

## 配置字段

### `enabled`

```toml
enabled = true
```

启用或关闭功能。默认关闭。

### `context_window_tiers`

```toml
context_window_tiers = [272000, 487000, 872000]
```

允许使用的软预算档位，单位为 token。

要求：

- 至少有一个档位。
- 每个值必须大于零。
- 必须严格递增。
- 新线程的第一档必须不超过当前模型运行时最大窗口。

可以使用自定义档位，例如：

```toml
context_window_tiers = [200000, 400000, 800000]
```

超过当前模型能力的未来档位可以保留在策略中；Codex 只会在当前模型真正支持下一档时升档。

### `keep_below_percent`

```toml
keep_below_percent = 45
```

成功 compaction 后，如果：

```text
U_after / B < 45%
```

Codex 认为压缩效果足够好，保持当前档位，并把中间区累计次数清零。

其中：

- `B` 是 compaction 前当前软预算。
- `U_after` 是安装压缩历史后重新计算的完整上下文占用。

### `expand_at_or_above_percent`

```toml
expand_at_or_above_percent = 65
```

成功 compaction 后，如果：

```text
U_after / B >= 65%
```

Codex 认为压缩效果不足，立即尝试升入下一档。

### `ambiguous_compactions_before_expand`

```toml
ambiguous_compactions_before_expand = 2
```

如果：

```text
45% <= U_after / B < 65%
```

本次结果进入中间区。连续达到配置次数后，Codex 尝试升档。

默认值 `2` 表示中间区结果需要连续出现两次。

## 自动压缩阈值

当前软预算记作 `B`。

Adaptive Context Budget 计算：

```text
soft_compact_limit = floor(0.9 × B)

effective_compact_limit =
    min(soft_compact_limit, catalog_auto_compact_limit)
```

如果模型 catalog 没有额外限制，则默认档位对应：

| 当前软预算 | 自动压缩线 |
|---:|---:|
| 272,000 | 244,800 |
| 487,000 | 438,300 |
| 872,000 | 784,800 |

模型硬窗口仍负责输入安全限制、裁剪和 token accounting；软预算不会替代模型硬窗口。

## 哪些 compaction 会更新档位

只有以下 compaction 会更新 Governor：

```text
trigger = Auto
reason = ContextLimit
```

以下操作只把当前 checkpoint 原样复制到新的 `CompactedItem`，不会改变档位或累计次数：

- 用户手动 `/compact`。
- 模型降窗触发的 compaction。
- `comp_hash` 变化触发的 compaction。
- 其他不属于自动上下文限制的压缩。

失败的 compaction 不更新内存状态，也不会写入成功 checkpoint。

## 升档限制

即使反馈要求升档，也必须满足：

1. 策略中存在下一档。
2. 当前模型运行时最大窗口支持下一档。
3. 下一档的实际自动压缩线高于当前档。

第三项用于处理 catalog cap。例如：

```text
当前档实际压缩线 = 200K
下一档实际压缩线 = 200K
```

这时升档没有实际效果，Codex 会保持当前档并记录 `catalog_capped`。

## 新线程、恢复线程和 fork

### 新线程

新线程把完整策略与初始状态写入 `SessionMeta`：

```text
target_context_budget_tokens = 第一档
ambiguous_compaction_count = 0
```

### 已有线程

策略随线程固定。

线程一旦保存了 Adaptive Context Budget checkpoint，后续修改全局配置不会改变该线程的既有策略；新配置只影响新线程。

### Resume

恢复线程时，Codex 会：

1. 重放 rollout。
2. 应用 rollback。
3. 找到最新仍然有效的 `CompactedItem` checkpoint。
4. 没有 compaction checkpoint 时使用 `SessionMeta` 初始 checkpoint。
5. 恢复当前档位与中间区累计次数。

### Fork

fork 子线程继承父线程 rollback 后仍然有效的当前 checkpoint，不会重置到第一档。继承结果会写入子线程自己的 `SessionMeta`。

### 旧 rollout

如果旧线程完全没有 Adaptive Context Budget 数据，Codex 会使用恢复时的配置生成策略，并根据有效历史占用选择一个兼容档位。第一次成功 compaction 后，完整策略会被写入 checkpoint。

## 模型切换

切换模型前，Codex 检查：

```text
当前 target_context_budget_tokens <= 候选模型运行时最大窗口
```

如果当前档位不兼容，模型切换会被原子拒绝：

- 模型不变。
- 当前档位不变。
- 中间区累计次数不变。
- 不写入成功的设置变更。

只有未来档位超过候选模型能力不会阻止切换；Codex 只检查当前目标档位。

## 如何确认功能正在工作

### 1. 查看 TUI token meter

启用后，Codex 会通过 `target_context_budget_tokens` 报告当前软预算。TUI 的上下文百分比优先使用软预算作为分母。

功能关闭时该字段为 `null`，TUI 回退到原来的 `model_context_window`。

### 2. 使用 `/status`

检查 context window 行显示的分母。启用并建立 token usage 后，分母应对应当前软预算档位。

### 3. 查看 trace 日志

从源码启动 TUI 时建议：

```bash
cd codex-rs
RUST_LOG=trace just codex -c log_dir=/tmp/codex-adaptive-context-budget-logs
```

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

### 4. 查看 rollout

成功 compaction 的 rollout `CompactedItem` 中应出现：

```text
adaptive_context_budget.policy
adaptive_context_budget.state
```

## 常见配置错误

### 与显式 compact limit 冲突

```text
features.adaptive_context_budget conflicts with model_auto_compact_token_limit
```

删除顶层 `model_auto_compact_token_limit`。

### 与 Token Budget 冲突

```text
features.adaptive_context_budget conflicts with features.token_budget
```

关闭 Token Budget feature。

### 使用了 body-after-prefix

```text
features.adaptive_context_budget requires model_auto_compact_token_limit_scope = "total"
```

删除该字段或设置为 `"total"`。

### 第一档超过模型最大窗口

```text
adaptive context budget target ... exceeds model ... maximum ...
```

降低第一档、扩大经过 catalog clamp 的显式模型窗口，或选择支持该档位的模型。

## 如何关闭

删除配置块，或设置：

```toml
[features.adaptive_context_budget]
enabled = false
```

功能关闭后，历史中已有的 Governor checkpoint 会被忽略，自动压缩恢复原有行为。

## 当前不包含的能力

当前 MVP 不包含：

- 自动降档。
- PRUNE 阶段。
- 语义重要性评分。
- 新的设置页面或独立 UI。
- 绕过模型 catalog 上限。
- 修改 Responses API compaction item 的内部内容。

OpenAI 官方把 `/responses/compact` 的输出定义为可供后续 Responses 请求继续使用的压缩 items；Adaptive Context Budget 是当前分支在 Codex host 侧增加的压缩时机与持久化策略。官方 API 参考：<https://developers.openai.com/api/reference/java/resources/responses/methods/compact>。
