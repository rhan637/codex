## About this fork / 关于本 Fork

> [!IMPORTANT]
> **English:** This repository is a personal fork of
> [OpenAI Codex](https://github.com/openai/codex). The original Codex project is
> developed and maintained by OpenAI. The Adaptive Context Budget feature in this
> fork is developed and maintained by
> [rhan637](https://github.com/rhan637).
>
> **中文：** 本仓库是 [OpenAI Codex](https://github.com/openai/codex) 的个人
> Fork。Codex 原项目由 OpenAI 开发和维护；本 Fork 中的 Adaptive Context Budget
>（自适应上下文预算）功能由 [rhan637](https://github.com/rhan637) 开发和维护。

### Adaptive Context Budget / 自适应上下文预算

Have you ever been deep into a complex Codex task, only to have repeated
compactions interrupt the workflow just when the earlier context matters most?

This fork explores a more adaptive approach: start with a conservative soft
context budget, then automatically expand it when the results of compaction show
that the current budget is no longer sufficient for the task.

Instead of using one fixed automatic compaction threshold, Codex gains more room
only when the task actually becomes complex—without bypassing the model's real
context-window or catalog limits.

你是否遇到过这样的情况：Codex 正在处理一个复杂、长期的任务，却偏偏在最需要保留
前文信息时反复触发 compaction（上下文压缩），不断打断任务的连续性？

本 Fork 尝试提供一种更加自适应的解决方案：先从相对保守的软上下文预算开始；当压缩
后的实际占用表明当前预算已经不足以支撑任务时，再自动扩大预算档位。

它不再让 Codex 始终受制于一个固定的自动压缩阈值，也不会一开始就把预算拉到最大；
只有当任务真正变得复杂时，才为 Codex 提供更多上下文空间，同时仍然遵守模型真实的
上下文窗口和 catalog 上限。

### Build this fork / 构建本 Fork

> [!WARNING]
> The installation commands in the original OpenAI README install the official
> OpenAI release and do not include this fork's Adaptive Context Budget feature.
>
> OpenAI 官方 README 中的安装命令安装的是官方版本，不包含本 Fork 的自适应上下文
> 预算功能。

```bash
git clone https://github.com/rhan637/codex.git
cd codex
git switch codex/local-dev
cd codex-rs
cargo build
```

Complete build requirements are available in the original
[installing and building guide](./docs/install.md).

完整的构建环境要求见官方[安装与构建说明](./docs/install.md)。

### Enable the feature / 启用功能

Add the following setting to your Codex `config.toml`:

在 Codex 的 `config.toml` 中加入：

```toml
[features.adaptive_context_budget]
enabled = true
```

The feature is now enabled. All other settings are optional and use the following
defaults when omitted:

这样就可以启用该功能。其他配置均为可选项；如果不填写，将采用以下默认值：

```toml
[features.adaptive_context_budget]
enabled = true
context_window_tiers = [272000, 487000, 872000]
keep_below_percent = 45
expand_at_or_above_percent = 65
ambiguous_compactions_before_expand = 2
```

You can add and customize these optional parameters to define your own
context-budget tiers and expansion policy.

你可以按需加入并修改这些可选参数，自定义上下文预算档位和升档策略。

### Default policy / 默认策略

```text
272K → 487K → 872K
```

After an automatic compaction, Codex compares the remaining context with the
current soft budget:

每次自动 compact 后，Codex 会比较剩余上下文与当前软预算：

- Below 45%: keep the current tier.

  低于 45%：保持当前档位。
- From 45% to below 65%: expand after two consecutive results.

  达到 45% 但低于 65%：连续出现两次后尝试升档。
- At or above 65%: immediately attempt to expand.

  达到或超过 65%：立即尝试升档。

The governor only expands to tiers supported by the current model and catalog.

Governor 只会升入当前模型和 catalog 实际支持的档位。

### Detailed documentation / 详细文档

- [`ADAPTIVE_CONTEXT_BUDGET_USAGE.md`](./ADAPTIVE_CONTEXT_BUDGET_USAGE.md) —
  complete configuration, usage, compatibility, and troubleshooting guide.

  完整的配置、使用、兼容性和故障排查说明。
- [`ADAPTIVE_CONTEXT_BUDGET_DEVELOPMENT.md`](./ADAPTIVE_CONTEXT_BUDGET_DEVELOPMENT.md)
  — implementation design, compaction flow, persistence, checkpoints, tests, and
  the complete changed-file list.

  实现思路、compaction 流程、持久化、checkpoint、测试和完整改动文件清单。

---

<p align="center"><strong>Codex CLI</strong> is a coding agent from OpenAI that runs locally on your computer.
<p align="center">
  <img src="https://github.com/openai/codex/blob/main/.github/codex-cli-splash.png" alt="Codex CLI splash" width="80%" />
</p>
</br>
If you want Codex in your code editor (VS Code, Cursor, Windsurf), <a href="https://developers.openai.com/codex/ide">install in your IDE.</a>
</br>If you want the desktop app experience, run <code>codex app</code> or visit <a href="https://chatgpt.com/codex?app-landing-page=true">the Codex App page</a>.
</br>If you are looking for the <em>cloud-based agent</em> from OpenAI, <strong>Codex Web</strong>, go to <a href="https://chatgpt.com/codex">chatgpt.com/codex</a>.</p>

---

## Quickstart

### Installing and running Codex CLI

Run the following on Mac or Linux to install Codex CLI:

```shell
curl -fsSL https://chatgpt.com/codex/install.sh | sh
```

Run the following on Windows to install Codex CLI:

```shell
powershell -ExecutionPolicy ByPass -c "irm https://chatgpt.com/codex/install.ps1 | iex"
```

The standalone installers download from `https://releases.openai.com/codex` by default and fall back to GitHub Releases if a metadata or asset download is unavailable. To force GitHub Releases, set `CODEX_INSTALLER_USE_RELEASES_OPENAI_COM` to `false` (`0` and `no` are also accepted):

```shell
curl -fsSL https://chatgpt.com/codex/install.sh | CODEX_INSTALLER_USE_RELEASES_OPENAI_COM=false sh
```

```powershell
$env:CODEX_INSTALLER_USE_RELEASES_OPENAI_COM='false'; irm https://chatgpt.com/codex/install.ps1 | iex
```

Codex CLI can also be installed via the following package managers:

```shell
# Install using npm
npm install -g @openai/codex
```

```shell
# Install using Homebrew
brew install --cask codex
```

Then simply run `codex` to get started.

<details>
<summary>You can also go to the <a href="https://github.com/openai/codex/releases/latest">latest GitHub Release</a> and download the appropriate binary for your platform.</summary>

Each GitHub Release contains many executables, but in practice, you likely want one of these:

- macOS
  - Apple Silicon/arm64: `codex-aarch64-apple-darwin.tar.gz`
  - x86_64 (older Mac hardware): `codex-x86_64-apple-darwin.tar.gz`
- Linux
  - x86_64: `codex-x86_64-unknown-linux-musl.tar.gz`
  - arm64: `codex-aarch64-unknown-linux-musl.tar.gz`

Each archive contains a single entry with the platform baked into the name (e.g., `codex-x86_64-unknown-linux-musl`), so you likely want to rename it to `codex` after extracting it.

</details>

### Using Codex with your ChatGPT plan

Run `codex` and select **Sign in with ChatGPT**. We recommend signing into your ChatGPT account to use Codex as part of your Plus, Pro, Business, Edu, or Enterprise plan. [Learn more about what's included in your ChatGPT plan](https://help.openai.com/en/articles/11369540-codex-in-chatgpt).

You can also use Codex with an API key, but this requires [additional setup](https://developers.openai.com/codex/auth#sign-in-with-an-api-key).

## Docs

- [**Codex Documentation**](https://developers.openai.com/codex)
- [**Contributing**](./docs/contributing.md)
- [**Installing & building**](./docs/install.md)
- [**Open source fund**](./docs/open-source-fund.md)

This repository is licensed under the [Apache-2.0 License](LICENSE).
