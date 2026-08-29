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

- [Usage guide / 使用说明](./ADAPTIVE_CONTEXT_BUDGET_USAGE.md)
- [Development guide / 开发说明](./ADAPTIVE_CONTEXT_BUDGET_DEVELOPMENT.md)

## Build and use this fork / 构建并使用本 Fork

> [!WARNING]
> The installation commands in the upstream README install the official OpenAI
> release and do not contain this fork's changes.
>
> 下方官方 README 中的安装命令安装的是 OpenAI 官方版本，不包含本 Fork 的改动。

Build this fork from source / 从源码构建本 Fork：

```bash
git clone https://github.com/rhan637/codex.git
cd codex
git switch codex/local-dev
cd codex-rs
cargo build
./target/debug/codex
```

After building, follow the
[usage guide](./ADAPTIVE_CONTEXT_BUDGET_USAGE.md) to enable and configure the
feature.

构建完成后，请按照[使用说明](./ADAPTIVE_CONTEXT_BUDGET_USAGE.md)启用并配置该功能。

---

## Upstream Codex README / OpenAI 官方 README

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
