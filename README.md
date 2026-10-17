# LocalSend+ 🚀

> LocalSend 的娱乐向增强分支。快，更快，再快一点。
>
> **这不是官方项目**。这是一个随时可能停更的娱乐 fork，不承诺传输质量，
> 但欢迎所有人 fork、改、玩。
>
> **这个仓库是 AI 写出来的。** 人类所有者只提需求和定方向，代码、提交、CI、Release
> 全部由 AI agent 完成 —— 详见下方免责声明。

---

## 中文

### 这是什么

LocalSend+ 基于开源的 [LocalSend](https://github.com/localsend/localsend)（Apache 2.0，基线 v1.18.2），
在**保留原版全部功能与 UI** 的前提下，对传输层做了大手术：

- 🚀 把默认传输从 **TCP** 换成了 **QUIC**（基于 quinn 0.11）
- ⚡ 用 **tokio 多线程**并行分块传输，多条连接同时跑
- 🔓 支持 **明文传输**（局域网内对速度敏感、不需要加密的场景）

它依然是那个"不需要互联网、不需要服务器、局域网内直接互传"的 LocalSend，
只是更快了。

### 核心特性

| 特性 | 说明 |
|------|------|
| 🚀 **QUIC 传输** | 以 QUIC 替代传统 TCP 传输；当 QUIC 不可用（未编译进二进制 / 运行期绑定失败）时**自动降级回 TCP**，绝不影响传输可用性 |
| ⚡ **tokio 多线程并行** | 大文件切成多个字节区间，多条独立连接并发拉取，再按序写回；消除单连接队头阻塞对下载速度的影响 |
| 🔓 **明文传输** | 保留 TLS 关闭路径，局域网内速度敏感、无需加密的场景可以直接明文直传 |
| 📦 **断点续传** | 下载侧支持 HTTP `Range`（`206 Partial Content`）；上传侧支持 `offset` 协议 + `409` 协商，中断后从断点继续，而不是从头再来 |
| 🔬 **硬件加速** | 运行时检测 SSE2/SSE4.2/AVX/AVX2/AVX-512/AES-NI/SHA-NI（x86_64）、NEON/AES/SHA2（aarch64），自动选择最优哈希/加密后端 |
| ✨ **零拷贝直传** | 文件通过流式读取直出，不再经过中间 channel 多拷贝一次 |

更多技术细节见 [LOCALSEND_PLUS.md](LOCALSEND_PLUS.md)。

### 免责声明

- 这是一个**娱乐 fork**，与 LocalSend 官方无关。
- 本项目**不对传输质量负责**：QUIC / 多线程 / 明文等改动可能引入未知问题，请自行评估风险后再用。
- 项目**随时可能停更**，没有维护承诺，也没有 issue 响应保证。
- 但**欢迎所有人 fork**：想怎么改怎么改，开心就好 🎉
- 🤖 **这个仓库是 AI 做的活。** 人类所有者只负责**提需求、定方向、判断「这样对不对」**，
  其余全部由 AI agent 完成：

  | 工作 | 谁做 |
  |---|---|
  | 编写与修改源码 | AI |
  | 提交、推送、合并、处理 issue / PR | AI |
  | 跑测试、排查 CI 失败 | AI |
  | 构建 APK、打包并发布 Release | AI |
  | 撰写文档（包括这份 README） | AI |

  **没有人逐行 review 过这些改动**，所以**不保证代码正确**——遇到 bug 很正常，
  请自行 review 源码后再用。发现问题欢迎开 issue：只要这个项目还在动，AI 就会接着修。
  最初由 Goose（DeepSeek-V4-Pro）完成，现由跑在 Android 设备上的 Minis agent 维护。
- 已安装旧版本的用户升级时若提示"签名不一致"，请卸载后重装（个人签名密钥）。

### 构建

与上游相同，从 `app` 目录执行：

```bash
flutter pub get
flutter run        # 开发运行
flutter build apk  # Android
flutter build windows
flutter build linux
```

> 注意：本项目要求 Rust 工具链（rust-toolchain.toml 指定 1.97.1）。
> QUIC 后端由 Cargo feature `quic` 控制，默认**不**编译进 `full`；
> 需要完整 QUIC 能力时请自行启用该 feature 构建。

### 许可证

[Apache License 2.0](LICENSE)。上游版权归 [Tien Do Nam](https://github.com/Tienisto) 及 LocalSend 贡献者所有。
本分支的修改同样以 Apache 2.0 发布。

---

## English

> **This repository is AI work.** The human owner only sets requirements and direction; the code,
> the commits, CI, and the releases are all produced by an AI agent — see the disclaimer below.

### What is this

LocalSend+ is a **fun/experimental fork** of the open-source [LocalSend](https://github.com/localsend/localsend)
(Apache 2.0, based on v1.18.2). It keeps **all original features and UI**, but performs major
surgery on the **transport layer**:

- 🚀 Replaces the default transport from **TCP** to **QUIC** (based on quinn 0.11)
- ⚡ Uses **tokio multi-threading** for parallel chunked transfers over multiple connections
- 🔓 Supports **plain-text transfer** (for speed-sensitive LAN scenarios where encryption is not needed)

It is still the same LocalSend — no internet, no server, direct peer-to-peer over your local network.
Just faster.

### Key Features

| Feature | Description |
|---------|-------------|
| 🚀 **QUIC transport** | QUIC replaces legacy TCP; when QUIC is unavailable (not compiled in / runtime bind failure) it **automatically falls back to TCP**, so availability is never affected |
| ⚡ **tokio parallel transfer** | Large files are split into byte ranges and fetched concurrently over independent connections, then written back in order; removes head-of-line blocking on the download side |
| 🔓 **Plain-text transfer** | The TLS-off path is preserved for speed-sensitive LAN scenarios |
| 📦 **Resumable transfer** | Download side supports HTTP `Range` (`206 Partial Content`); upload side supports an `offset` protocol with `409` negotiation, so interrupted transfers resume from where they stopped |
| 🔬 **Hardware acceleration** | Runtime detection of SSE2/SSE4.2/AVX/AVX2/AVX-512/AES-NI/SHA-NI (x86_64) and NEON/AES/SHA2 (aarch64), with automatic selection of the fastest hash/crypto backend |
| ✨ **Zero-copy streaming** | Files are streamed directly from disk with no extra copy through intermediate channels |

See [LOCALSEND_PLUS.md](LOCALSEND_PLUS.md) for more technical details (in Chinese).

### Disclaimer

- This is a **fun fork**, not affiliated with the official LocalSend project.
- This project is **NOT responsible for transfer quality**: QUIC / multi-threading / plain-text changes
  may introduce unknown issues. Use at your own risk.
- This project **may be abandoned at any time**. No maintenance commitment, no issue-response guarantee.
- But **everyone is welcome to fork it** — modify it however you like. Have fun! 🎉
- 🤖 **This repository is AI work.** The human owner sets direction and requirements — that is all.
  Everything else is done by an AI agent:

  | Task | Who |
  |---|---|
  | Writing and modifying the source | AI |
  | Committing, pushing, merging, triaging issues / PRs | AI |
  | Running tests, debugging CI failures | AI |
  | Building APKs, packaging and publishing releases | AI |
  | Writing documentation, including this README | AI |

  **No one reviews these changes line by line**, so **correctness is not guaranteed** — bugs are to
  be expected; review the source code before using it. Issues are welcome — as long as this project is
  still alive, the AI will keep fixing them.
  Originally built by Goose (DeepSeek-V4-Pro); currently maintained by a Minis agent running on an
  Android device.
- If you installed an older build, you may need to uninstall and reinstall when the signature mismatches (personal signing key).

### Building

Same as upstream, from the `app` directory:

```bash
flutter pub get
flutter run        # run in dev mode
flutter build apk  # Android
flutter build windows
flutter build linux
```

> Note: a Rust toolchain is required (1.97.1 per `rust-toolchain.toml`).
> The QUIC backend is gated behind the Cargo feature `quic` and is **not** part of `full` by default;
> enable the feature explicitly if you want full QUIC support.

### License

[Apache License 2.0](LICENSE). Upstream copyright belongs to [Tien Do Nam](https://github.com/Tienisto)
and the LocalSend contributors. Modifications in this fork are also released under Apache 2.0.
