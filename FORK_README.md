# Codex Fork by rhan637 / rhan637 的 Codex Fork

> [!IMPORTANT]
> **English:** This repository is a personal fork of
> [openai/codex](https://github.com/openai/codex). The original Codex project is
> developed and maintained by OpenAI. Fork-specific changes are maintained by
> [rhan637](https://github.com/rhan637).
>
> **中文：** 本仓库是 [OpenAI Codex](https://github.com/openai/codex) 的个人
> Fork。Codex 原项目由 OpenAI 开发和维护；本 Fork 特有的改动由
> [rhan637](https://github.com/rhan637) 开发和维护。

## Fork-specific changes / Fork 特有改动

### Adaptive Context Budget / 自适应上下文预算

This fork adds an opt-in Adaptive Context Budget governor for long-running Codex
sessions. Instead of using one fixed automatic compaction threshold, each thread
starts with a smaller soft context budget and can expand to larger tiers according
to the actual context retained after compaction.

本 Fork 为长时间运行的 Codex 线程增加了一个可选的“自适应上下文预算”控制器。
线程不再始终使用一个固定的自动压缩阈值，而是从较小的软预算开始，再根据每次压缩
后实际保留的上下文量，决定是否扩大到更高的预算档位。

Default policy / 默认策略：

```text
272K → 487K → 872K
```

- Post-compaction usage below 45% keeps the current tier. / 压缩后占用低于 45%：保持当前档位。
- Usage at or above 65% attempts to expand immediately. / 占用达到或超过 65%：立即尝试升档。
- Two consecutive results in the 45%–65% range attempt to expand. / 连续两次处于 45%–65%：尝试升档。
- The feature is experimental and disabled by default. / 该功能目前为实验功能，默认关闭。
- Model context-window and catalog limits are still respected. / 模型上下文窗口和 catalog 上限仍然有效。

Documentation / 文档：

- [`ADAPTIVE_CONTEXT_BUDGET_USAGE.md`](./ADAPTIVE_CONTEXT_BUDGET_USAGE.md) —
  complete configuration, usage, verification, compatibility, and troubleshooting
  guide. / 完整的配置、使用、验证、兼容性和故障排查说明。
- [`ADAPTIVE_CONTEXT_BUDGET_DEVELOPMENT.md`](./ADAPTIVE_CONTEXT_BUDGET_DEVELOPMENT.md)
  — implementation design, compaction flow, persistence, checkpoint recovery,
  model switching, tests, and the complete changed-file list. /
  实现思路、压缩流程、持久化、checkpoint 恢复、模型切换、测试和完整改动文件清单。

## Build and use this fork / 构建并使用本 Fork

> [!WARNING]
> The installation commands in the upstream README install the official OpenAI
> release and do not contain this fork's changes.
>
> 官方 README 中的安装命令安装的是 OpenAI 官方版本，不包含本 Fork 的改动。

### 1. Build this fork / 构建本 Fork

Clone the fork, switch to the feature branch, and build the Rust workspace. /
克隆本 Fork、切换到功能分支，并构建 Rust workspace：

```bash
git clone https://github.com/rhan637/codex.git
cd codex
git switch codex/local-dev
cd codex-rs
cargo build
```

The resulting binary is `codex-rs/target/debug/codex`. / 构建产物位于
`codex-rs/target/debug/codex`。

For complete toolchain and platform requirements, see the upstream
[installing and building guide](./docs/install.md). /
完整的工具链和平台要求见官方[安装与构建说明](./docs/install.md)。

### 2. Locate the active configuration / 找到实际生效的配置文件

Codex reads the configuration from: / Codex 从以下位置读取配置：

```text
$CODEX_HOME/config.toml
```

If `CODEX_HOME` is not set, the usual location is: / 如果没有设置
`CODEX_HOME`，通常使用：

```text
~/.codex/config.toml
```

When Codex Desktop runs through WSL, `CODEX_HOME` may point to a mounted Windows
directory instead of the WSL home directory. Edit the configuration used by the
running Codex process. / 当 Codex Desktop 通过 WSL 运行时，`CODEX_HOME` 可能指向
挂载的 Windows 目录，而不是 WSL home；应修改当前 Codex 进程实际使用的配置文件。

### 3. Check incompatible settings / 检查冲突配置

Before enabling the feature, remove or disable: / 启用前需要删除或关闭：

- `features.token_budget`
- the top-level `model_auto_compact_token_limit`
- `model_auto_compact_token_limit_scope = "body_after_prefix"`

The model catalog's built-in auto-compact limit is not a configuration conflict;
it remains an upper bound. / 模型 catalog 自带的自动压缩限制不属于配置冲突，仍会
作为不可突破的上限。

### 4. Enable the feature / 启用功能

Add the following block to the active `config.toml`: / 在实际生效的
`config.toml` 中加入：

```toml
[features.adaptive_context_budget]
enabled = true
context_window_tiers = [272000, 487000, 872000]
keep_below_percent = 45
expand_at_or_above_percent = 65
ambiguous_compactions_before_expand = 2
```

These are the current defaults, so the minimal configuration is also valid: /
以上均为当前默认值，因此也可以只写最小配置：

```toml
[features.adaptive_context_budget]
enabled = true
```

All policy values are user-configurable. The tier list must contain positive,
strictly increasing token values, and the percentages must satisfy
`0 < keep < expand <= 100`. / 所有策略值都允许用户配置；档位必须是正数且严格
递增，百分比必须满足 `0 < keep < expand <= 100`。

### 5. Start Codex and create a thread / 启动 Codex 并创建线程

From `codex-rs`, launch the binary built from this fork: / 在 `codex-rs` 目录中
启动本 Fork 构建出的二进制：

```bash
./target/debug/codex
```

Create a new thread after enabling the feature. The normalized policy is pinned
to the thread when its initial checkpoint is created. Later global configuration
changes do not rewrite a policy already stored by that thread. / 启用后请创建新线程；
规范化后的策略会在初始 checkpoint 创建时固定到该线程，之后修改全局配置不会改写
该线程已经保存的策略。

### 6. Understand the runtime behavior / 理解运行时行为

For the current soft budget `B`, automatic compaction is triggered at: /
当前软预算为 `B` 时，自动压缩线为：

```text
effective_compact_limit = min(floor(0.9 × B), catalog_auto_compact_limit)
```

Without a lower catalog limit, the default tiers use these trigger points: /
如果 catalog 没有更低的限制，默认档位对应以下压缩线：

| Soft budget / 软预算 | Auto-compaction trigger / 自动压缩线 |
|---:|---:|
| 272,000 | 244,800 |
| 487,000 | 438,300 |
| 872,000 | 784,800 |

After a successful automatic context-limit compaction, Codex compares the full
post-compaction usage `U_after` with the budget `B`: / 自动上下文限制压缩成功后，
Codex 比较完整的压缩后占用 `U_after` 与当前预算 `B`：

- `U_after / B < 45%`: keep the current tier and clear the middle-range count. /
  保持当前档位，并清零中间区计数。
- `45% <= U_after / B < 65%`: increment the middle-range count; two consecutive
  results attempt to expand. / 中间区计数加一；连续两次后尝试升档。
- `U_after / B >= 65%`: immediately attempt to expand to the next tier. /
  立即尝试升入下一档。

The governor only moves upward. It never exceeds the model's runtime context
window, and it does not bypass the catalog limit. Manual compaction does not
change the tier. / Governor 只会升档，不会自动降档；它不会超过模型运行时上下文
窗口，也不会绕过 catalog 上限；手动压缩不会改变档位。

### 7. Verify that it is active / 验证功能是否生效

Inside the TUI, run: / 在 TUI 中运行：

```text
/status
```

After token usage is available, the context-window denominator should use the
current soft-budget tier. It starts at `272K` with the default policy and changes
only after the governor successfully expands. / 产生 token usage 后，上下文窗口分母
应使用当前软预算档位；默认从 `272K` 开始，并只在 Governor 成功升档后改变。

For trace fields, rollout checkpoint inspection, custom policies, resume/fork
behavior, model-switching constraints, and error messages, read the complete
[`ADAPTIVE_CONTEXT_BUDGET_USAGE.md`](./ADAPTIVE_CONTEXT_BUDGET_USAGE.md). /
如需查看 trace 字段、rollout checkpoint、自定义策略、resume/fork、模型切换约束和
错误处理，请阅读完整的
[`ADAPTIVE_CONTEXT_BUDGET_USAGE.md`](./ADAPTIVE_CONTEXT_BUDGET_USAGE.md)。

### 8. Read the implementation design / 阅读实现思路

The implementation reuses Codex's existing compaction paths and adds a thread-level
governor around their shared successful-history installation boundary. The policy
and current state are persisted as versioned checkpoints in `SessionMeta` and
`CompactedItem`, then reconstructed with rollback awareness for resume and fork. /
实现复用 Codex 现有的 compaction 路径，在它们共同的“成功安装压缩历史”边界增加
线程级 Governor；策略和当前状态以带版本的 checkpoint 写入 `SessionMeta` 与
`CompactedItem`，并在 resume 和 fork 时结合 rollback 结果进行重建。

The complete architecture, old/new flow comparison, persistence model, checkpoint
semantics, state machine, API/TUI propagation, tests, and changed files are
documented in
[`ADAPTIVE_CONTEXT_BUDGET_DEVELOPMENT.md`](./ADAPTIVE_CONTEXT_BUDGET_DEVELOPMENT.md). /
完整的架构、新旧流程对比、持久化模型、checkpoint 含义、状态机、API/TUI 传播、测试
和改动文件清单见
[`ADAPTIVE_CONTEXT_BUDGET_DEVELOPMENT.md`](./ADAPTIVE_CONTEXT_BUDGET_DEVELOPMENT.md)。
