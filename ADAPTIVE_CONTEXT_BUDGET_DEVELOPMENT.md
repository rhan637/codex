# Adaptive Context Budget 开发说明

## 文档范围

本文说明提交 `045a3465c4` 中 Adaptive Context Budget MVP 的实现逻辑、持久化边界、恢复流程、模型切换约束、API/TUI 传播和测试结构。

该功能建立在 Codex 原有 compaction 链路之上，没有重新实现 Responses API compaction。

## 目标

原有 Codex 使用模型信息或配置中的固定自动压缩阈值。MVP 增加一个线程级 Governor：

```text
272K → 487K → 872K
```

Governor 根据每次成功自动上下文压缩后的完整占用决定是否升档。

核心原则：

- 软预算只决定自动 compaction 时机。
- 模型硬窗口继续负责 token accounting、输入裁剪和安全限制。
- 不突破模型 catalog 限制。
- 策略随线程固定。
- resume 和 fork 使用 rollback 后仍有效的 checkpoint。
- 只升档，不降档。

## 关键概念

### Rollout

Rollout 是按时间顺序持久化的线程 item 日志。它保存可以重放线程的重要边界，而不是直接序列化整个进程内存。

相关 item 包括：

```text
SessionMeta
ResponseItem
CompactedItem
TurnContext
WorldState
EventMsg
```

### SessionMeta

`SessionMeta` 是线程创建时持久化的线程级元数据。MVP 在其中增加可选的初始 Adaptive Context Budget checkpoint。

### SessionState

`SessionState` 是活动线程的内存状态，受 Session mutex 保护。MVP 在其中保存 `AdaptiveContextBudgetRuntime`。

`SessionMeta` 与 `SessionState` 不是同一层：

```text
SessionMeta  = 持久化的线程初始档案
SessionState = 当前进程中的可变运行状态
Rollout      = 可重放的线程变化记录
```

### ContextManager

`ContextManager` 持有当前模型可见历史、token usage、reference context 和 world-state baseline。

成功 compaction 后，旧历史会被 `replacement_history` 整体替换。

### 三种容易混淆的 compaction item

#### Responses API `type=compaction`

由 `/responses/compact` 或 Remote Compaction V2 生成，可能包含 opaque `encrypted_content`。它是模型上下文中的 `ResponseItem`。

#### Rollout `CompactedItem`

Codex host 持久化的压缩安装 checkpoint，包含 replacement history、window metadata 和 Adaptive Context Budget checkpoint。

#### UI `ContextCompactionItem`

用于 app-server/TUI 展示 compaction started/completed 生命周期，不承担 Governor 持久化。

## 新增数据模型

### Policy

```rust
AdaptiveContextBudgetPolicy {
    policy_version,
    context_window_tiers,
    keep_below_percent,
    expand_at_or_above_percent,
    ambiguous_compactions_before_expand,
}
```

Policy 是线程固定的规范化策略。

### State

```rust
AdaptiveContextBudgetState {
    policy_version,
    target_context_budget_tokens,
    ambiguous_compaction_count,
}
```

State 是每次 compaction 后可能变化的部分。

### Checkpoint

```rust
AdaptiveContextBudgetCheckpoint {
    policy,
    state,
}
```

Checkpoint 是在一个确定的历史安装点恢复 Governor 所需的最小完整快照。

### Token usage

`TokenUsageInfo` 与 app-server `ThreadTokenUsage` 增加：

```rust
target_context_budget_tokens: Option<i64>
```

功能关闭时为 `None`，保持旧客户端和旧 UI 行为。

## 配置解析

Feature 使用结构化 `FeatureToml<AdaptiveContextBudgetConfigToml>`，允许布尔启用或配置表。

默认策略：

```text
tiers     = [272000, 487000, 872000]
keep      = 45
expand    = 65
ambiguous = 2
```

加载顺序：

```text
ConfigToml
  ↓ FeaturesToml
Feature enablement
  ↓
resolve_adaptive_context_budget_config
  ↓ normalize + validate
AdaptiveContextBudgetPolicy
  ↓
Config.adaptive_context_budget
```

校验内容：

- 档位非空、正数、严格递增。
- `0 < keep < expand <= 100`。
- 中间区累计次数至少为 1。
- 与 Token Budget 冲突。
- 与 `body_after_prefix` 冲突。
- 与用户显式 `model_auto_compact_token_limit` 冲突。

Catalog 自带 compact limit 不冲突；它在运行时作为上限。

## 软预算与硬窗口

当前软预算：

```text
B = target_context_budget_tokens
```

自动压缩阈值：

```text
effective_compact_limit =
    min(floor(0.9 × B), catalog_auto_compact_limit)
```

运行时硬窗口：

1. 优先使用经过现有 catalog clamp 的显式 `model_context_window`。
2. 否则使用 catalog `max_context_window`。
3. 缺失时退回 resolved context window。
4. 继续应用模型现有 effective-context percentage 得到模型可用硬窗口。

`context_window_token_status` 同时比较：

- 当前上下文是否达到 Governor 软压缩线。
- 当前上下文是否达到模型硬窗口。

因此软预算不会被错误用于输入裁剪或模型安全判断。

## 自动压缩入口

采样前 `run_pre_sampling_compact` 获取 `ContextWindowTokenStatus`。

如果 `token_limit_reached`：

```text
capture StepContext
  ↓
run_auto_compact(reason = ContextLimit)
```

原有 dispatcher 保持不变：

```text
Remote Compaction V2 feature 可用
  → normal Responses request + CompactionTrigger

Provider 支持 remote compaction，但 V2 feature 关闭
  → POST /responses/compact

Provider 不支持 remote compaction
  → local summarization compaction
```

三条路径最终汇合到 `Session::replace_compacted_history`。

## 压缩反馈状态机

成功安装 replacement history 后，重新估算完整上下文：

```text
U_after = estimated tokens of replacement history + base instructions
```

比较使用 `i128` 整数交叉相乘，避免浮点边界误差。

### 低区

```text
U_after × 100 < keep × B
```

行为：

- 保持当前档位。
- `ambiguous_compaction_count = 0`。

### 高区

```text
U_after × 100 >= expand × B
```

行为：

- 尝试升入下一档。
- `ambiguous_compaction_count = 0`。

### 中间区

```text
keep × B <= U_after × 100 < expand × B
```

行为：

- 累加 `ambiguous_compaction_count`。
- 达到策略次数后尝试升档。
- 尝试后清零。

默认 45/65 在压缩大约于 90% 软预算触发时，可近似解释为：

```text
压缩后保留不足压缩前的 50%：保持
保留约 50%～72.2%：观察
保留至少约 72.2%：立即尝试升档
```

### 升档约束

升档必须满足：

1. 存在下一档，否则记录 `max_tier`。
2. 候选档位不超过模型运行时最大窗口，否则记录 `model_window_capped`。
3. 候选档位的实际 compact limit 高于当前档，否则记录 `catalog_capped`。

未来档位不会在策略规范化时被删除。

## 哪些 compaction 会更新状态

只有：

```text
CompactionTrigger::Auto
CompactionReason::ContextLimit
```

会应用反馈状态机。

以下 compaction 仍写 checkpoint，但复制当前状态：

- Manual/UserRequested。
- Auto/ModelDownshift。
- Auto/CompHashChanged。
- 没有 Governor feedback metadata 的窗口重置。

失败发生在统一安装边界以前，因此不会：

- 替换活动历史。
- 更新 Governor。
- 写入成功 `CompactedItem` checkpoint。

## 成功 compaction 的原子顺序

旧流程的关键部分是：

```text
替换内存历史
→ 持久化 CompactedItem
→ recompute_token_usage
→ TokenCount
```

Governor 必须先获得 `U_after`，再持久化最新 checkpoint，因此新流程为：

```text
1. 为 replacement history 分配缺失的 item ID
2. 替换 SessionState 中的 ContextManager 历史
3. 重算 U_after，但不发送 TokenCount
4. 对 Auto + ContextLimit 应用 Governor feedback
5. 获取最新 AdaptiveContextBudgetCheckpoint
6. 构造 CompactedItem
7. 持久化 CompactedItem
8. 持久化 WorldState / TurnContext baseline
9. 发送一次包含最新软预算的 TokenCount
10. compaction 路径发送 lifecycle completed
```

这样不会先发旧软档位，再发新软档位。

## CompactedItem 持久化内容

MVP 在既有 `CompactedItem` 上增加可选 checkpoint，没有新增 `RolloutItem` variant。

当前 `CompactedItem` 包含：

```text
message
replacement_history
replacement_history_metadata sidecar
mcp_resource_origins
window_number
first_window_id
previous_window_id
window_id
adaptive_context_budget
```

`replacement_history` 是活动模型历史的完整替换基线，不是增量 patch。

Rollout wire 层继续兼容缺少新字段的旧 rollout；新字段使用 `Option` 和 serde default。

## SessionState

`SessionState` 增加：

```rust
adaptive_context_budget: Option<AdaptiveContextBudgetRuntime>
```

Runtime 保存：

```text
checkpoint
checkpoint_error
```

`checkpoint_error` 允许线程历史被读取，但会在采样入口阻止继续运行。

`SessionState::token_info()` 在返回 token usage snapshot 时注入当前 `target_context_budget_tokens`，避免在多个 token 更新路径中重复维护同一字段。

## 新线程持久化

新线程创建时：

1. 从 Config policy 创建 Runtime。
2. 初始目标为第一档。
3. 校验第一档是否超过模型运行时最大窗口。
4. 把初始 checkpoint 传给 `CreateThreadParams`。
5. ThreadStore/rollout recorder 写入 `SessionMeta.adaptive_context_budget`。

## Resume、rollback 与 checkpoint 重建

不能简单读取 rollout 中最后一个 checkpoint，因为它可能属于已被 rollback 的 turn。

重建逻辑从尾部逆向扫描：

```text
按 TurnStarted / TurnComplete / TurnAborted 划分 replay segment
  ↓
识别 UserMessage 和其他真实 turn boundary
  ↓
累计 ThreadRolledBack.num_turns
  ↓
丢弃被 rollback 的 segment
  ↓
收集仍有效的 CompactedItem checkpoint
```

恢复优先级：

1. 最新仍有效的 `CompactedItem` checkpoint。
2. `SessionMeta` 初始 checkpoint。

重建同时检测：

- 未知 policy version。
- policy/state version 不一致。
- 无效档位或 count。
- 同一线程内 policy 漂移。

错误保存在 Runtime 中，读取线程仍然允许，采样入口返回明确错误。

## Fork

fork 创建新 `SessionMeta` 前复用 rollback-aware Governor reconstruction。

子线程继承：

```text
policy
target_context_budget_tokens
ambiguous_compaction_count
checkpoint_error
```

子线程不会重置到第一档。

## 旧 rollout 兼容

完全没有 Governor 数据的 rollout：

1. SessionState 先从当前配置创建初始 Runtime。
2. 完成历史与 rollback 重建。
3. 估算有效历史 token。
4. 在模型支持的档位中选择最小且不会立即达到 compact limit 的档位。
5. 没有这样的档位时选择模型支持的最大策略档位。
6. 第一次成功 compaction 后写入完整 checkpoint。

Feature 关闭时忽略历史 Governor 数据，保持原有行为。

## AutoCompactWindow 恢复

Adaptive Context Budget 与既有 `AutoCompactWindow` 是两套并行状态。

`AutoCompactWindow` 负责：

```text
window_number
first_window_id
previous_window_id
window_id
prefill baseline
per-window reminder flags
```

成功 compaction 调用 `advance()`：

```text
window_number += 1
previous_window_id = old window_id
window_id = new UUIDv7
```

这些 metadata 写入 `CompactedItem`。Resume 时 rollout reconstruction 找到最新有效 window metadata，再调用 `restore_auto_compact_window`。

Governor checkpoint 恢复当前软预算；AutoCompactWindow 恢复上下文窗口身份和计数。二者不可混用。

## 模型切换

Session settings 更新前先解析候选模型 metadata，并在持有 SessionState lock 的提交路径中校验：

```text
current target soft budget <= candidate runtime maximum
```

不兼容时，在任何设置或运行时副作用发生前返回 ConstraintError，因此拒绝是原子的。

只检查当前目标档位；未来策略档位超过候选模型能力不阻止切换。

每次正常采样入口还会再次校验，用于处理 resume 到不兼容模型的情况。切换到兼容模型后，无需修改 checkpoint 即可解除阻塞。

## Token usage、app-server 与 TUI

Core `TokenUsageInfo` 增加 nullable `target_context_budget_tokens`。

App-server v2 `ThreadTokenUsage` 透传该字段，并更新 JSON schema、TypeScript fixture 与 stable/experimental precomputed exports。

TUI 选择 context meter 分母时：

```text
target_context_budget_tokens
    or model_context_window
```

因此：

- Feature 开启：显示软预算档位。
- Feature 关闭或旧事件：回退硬窗口。

没有新增页面、弹窗或设置组件。

## Tracing

Governor 更新记录：

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

这些字段可用于后续统计 `U_after / B`、真实压缩保留率和升档效果。

## 测试结构

### 状态机单元测试

覆盖：

- 低区、高区和中间区累计。
- 低区打断累计。
- 精确边界。
- 最大档、模型 cap、catalog cap。
- `i128` 安全计算。
- 未知/损坏 checkpoint。
- 只有 Auto + ContextLimit 更新反馈。
- 旧 rollout 兼容档位选择。

### Core 集成测试

覆盖：

- Local、legacy remote 和 Remote V2 首次软阈值 compaction。
- 高区反馈升档与 checkpoint 持久化。
- 新线程 `SessionMeta` 初始 checkpoint。
- 手动 compaction 复制状态。
- compaction 失败不落 checkpoint。
- Feature 关闭时保持旧请求序列和 nullable usage。
- compaction 后只出现一次最新 TokenCount。
- 模型切换原子拒绝及未来档位兼容。

### 重建测试

覆盖 rollback-aware checkpoint 选择和 policy drift。

### API/UI 测试

覆盖 app-server nullable 字段、schema fixture、TUI token meter 和 status snapshot。

## 生成物

配置结构变化生成：

```text
codex-rs/core/config.schema.json
```

App-server API 变化生成：

```text
JSON schema bundle
v2 notification schema
TypeScript ThreadTokenUsage
stable/experimental precomputed exports
```

UI 变化生成一个 status snapshot。

## 全部改动文件

以下清单来自功能提交 `045a3465c4`。

### App-server protocol 与生成 schema

- `codex-rs/app-server-protocol/schema/json/ServerNotification.json`
- `codex-rs/app-server-protocol/schema/json/codex_app_server_protocol.schemas.json`
- `codex-rs/app-server-protocol/schema/json/codex_app_server_protocol.v2.schemas.json`
- `codex-rs/app-server-protocol/schema/json/v2/ThreadTokenUsageUpdatedNotification.json`
- `codex-rs/app-server-protocol/schema/precomputed/app-server-exports-experimental.json.zst`
- `codex-rs/app-server-protocol/schema/precomputed/app-server-exports-stable.json.zst`
- `codex-rs/app-server-protocol/schema/typescript/v2/ThreadTokenUsage.ts`
- `codex-rs/app-server-protocol/src/protocol/thread_history.rs`
- `codex-rs/app-server-protocol/src/protocol/thread_history_projection_tests.rs`
- `codex-rs/app-server-protocol/src/protocol/v2/thread.rs`

### App-server 实现与测试

- `codex-rs/app-server/src/bespoke_event_handling.rs`
- `codex-rs/app-server/src/external_agent_migration/session_importer.rs`
- `codex-rs/app-server/tests/common/rollout.rs`
- `codex-rs/app-server/tests/suite/conversation_summary.rs`
- `codex-rs/app-server/tests/suite/v2/remote_thread_store.rs`
- `codex-rs/app-server/tests/suite/v2/thread_read.rs`
- `codex-rs/app-server/tests/suite/v2/thread_resume.rs`
- `codex-rs/app-server/tests/suite/v2/thread_timeline.rs`
- `codex-rs/app-server/tests/suite/v2/thread_unarchive.rs`

### 配置与约束

- `codex-rs/config/src/constraint.rs`
- `codex-rs/core/config.schema.json`
- `codex-rs/features/src/feature_configs.rs`
- `codex-rs/features/src/lib.rs`

### Core Governor 与 compaction

- `codex-rs/core/src/adaptive_context_budget.rs`
- `codex-rs/core/src/adaptive_context_budget_tests.rs`
- `codex-rs/core/src/compact.rs`
- `codex-rs/core/src/compact_remote.rs`
- `codex-rs/core/src/compact_remote_v2.rs`
- `codex-rs/core/src/config/config_tests.rs`
- `codex-rs/core/src/config/mod.rs`
- `codex-rs/core/src/lib.rs`

### Core Session、恢复与运行状态

- `codex-rs/core/src/session/context_window.rs`
- `codex-rs/core/src/session/mod.rs`
- `codex-rs/core/src/session/rollout_reconstruction.rs`
- `codex-rs/core/src/session/rollout_reconstruction_tests.rs`
- `codex-rs/core/src/session/session.rs`
- `codex-rs/core/src/session/tests.rs`
- `codex-rs/core/src/session/turn.rs`
- `codex-rs/core/src/session/turn_context.rs`
- `codex-rs/core/src/state/session.rs`

### Core 集成与兼容测试

- `codex-rs/core/src/agent/control_tests.rs`
- `codex-rs/core/tests/suite/adaptive_context_budget.rs`
- `codex-rs/core/tests/suite/mod.rs`
- `codex-rs/core/tests/suite/model_switching.rs`
- `codex-rs/core/tests/suite/sqlite_state.rs`

### Shared protocol 与 rollout history

- `codex-rs/history/src/lib.rs`
- `codex-rs/history/src/rollout_payload.rs`
- `codex-rs/history/src/tests.rs`
- `codex-rs/protocol/src/protocol.rs`

### Rollout recorder 与测试

- `codex-rs/rollout/src/compression_tests.rs`
- `codex-rs/rollout/src/metadata_tests.rs`
- `codex-rs/rollout/src/recorder.rs`
- `codex-rs/rollout/src/recorder_tests.rs`
- `codex-rs/rollout/src/session_index_tests.rs`
- `codex-rs/rollout/src/state_db_tests.rs`
- `codex-rs/rollout/src/tests.rs`

### Thread store 与 state projection

- `codex-rs/state/src/extract.rs`
- `codex-rs/state/src/runtime/threads.rs`
- `codex-rs/thread-store/src/in_memory.rs`
- `codex-rs/thread-store/src/local/create_thread.rs`
- `codex-rs/thread-store/src/local/mod.rs`
- `codex-rs/thread-store/src/local/model_context_tests.rs`
- `codex-rs/thread-store/src/local/pending_thread_metadata_tests.rs`
- `codex-rs/thread-store/src/local/revert_thread_tests.rs`
- `codex-rs/thread-store/src/local/rollout_migration_tests.rs`
- `codex-rs/thread-store/src/local/thread_history_materialization_tests.rs`
- `codex-rs/thread-store/src/thread_metadata_sync.rs`
- `codex-rs/thread-store/src/types.rs`

### TUI 实现、测试与快照

- `codex-rs/tui/src/app/tests.rs`
- `codex-rs/tui/src/chatwidget.rs`
- `codex-rs/tui/src/chatwidget/status_controls.rs`
- `codex-rs/tui/src/chatwidget/tests/helpers.rs`
- `codex-rs/tui/src/chatwidget/tests/status_and_layout.rs`
- `codex-rs/tui/src/resume_picker_transcript_preview_tests.rs`
- `codex-rs/tui/src/status/card.rs`
- `codex-rs/tui/src/status/snapshots/codex_tui__status__tests__status_context_window_uses_adaptive_budget.snap`
- `codex-rs/tui/src/status/tests.rs`
- `codex-rs/tui/src/token_usage.rs`

### 其他共享构造与兼容测试

- `codex-rs/exec/tests/event_processor_with_json_output.rs`
- `codex-rs/ext/goal/tests/goal_extension_backend.rs`
- `codex-rs/external-agent-migration/src/sessions/export.rs`
- `codex-rs/thread-manager-sample/src/main.rs`

## 未修改的边界

MVP 没有修改：

- 模型 catalog 内容。
- Responses API 协议本身。
- Sandbox 环境变量逻辑。
- 新 UI 页面或设置组件。
- PRUNE 或语义评分。
- 自动降档。

OpenAI 官方 `/responses/compact` 文档：<https://developers.openai.com/api/reference/java/resources/responses/methods/compact>。
