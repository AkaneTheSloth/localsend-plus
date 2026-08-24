# LocalSend+

LocalSend+ 是 LocalSend 的增强分支，目标是让局域网内的大文件传输更快、更稳、更省心。

- **分支**：`localsend-plus`（git 分支名不支持 `+` 字符）
- **开发者标注**：`Goose x Akane`
- **上游**：<https://github.com/localsend/localsend>
- **基线版本**：v1.18.2

> UI/UX 与原版完全兼容：所有增强都在传输层（Rust core）内部完成，
> 不改变既有页面、交互与设置项结构。

---

## 已实现的功能

### 1. 断点续传 / 字节范围请求（Range）
- **下载侧**（Web 下载 / Download API）：`packages/core/src/http/server/web.rs` 的
  `GET /api/localsend/v2/download` 现在支持 HTTP `Range: bytes=start-end` 头，
  返回 `206 Partial Content` + `Content-Range`，并始终声明 `Accept-Ranges: bytes`。
- 客户端新增 `LsHttpClientV2::download_range(...)`，可只拉取一个字节区间。
- 网络波动导致下载中断后，可以从中断点续传，而不是从头再来。

### 2. 多线程传输（并行分块）
- 客户端新增 `LsHttpClientV2::download_to_writer_parallel(...)`：把一个大文件切成
  `threads` 个字节区间，用多条独立连接并发拉取，再按顺序写回目标。
- 多条连接互相独立，单条连接丢包/阻塞不会拖慢其它分块（消除队头阻塞在下载侧的
  影响）。
- 对空文件或 `threads == 1` 自动回退到单连接顺序下载。

### 3. QUIC 传输 + TCP 降级
- 新增 `packages/core/src/http/transport.rs`：`Transport`（`Quic` / `Tcp`）与
  `select_transport()`，按「是否编译进 QUIC → 是否能绑定 UDP socket」的顺序决策。
- **无法启用 QUIC 时，会在日志中输出原因并自动降级到 TCP**，绝不影响传输可用性：
  - 未编译：`QUIC not compiled in (build without --features quic); falling back to TCP`
  - 运行期失败：`QUIC unavailable (<原因>); falling back to TCP`
- QUIC 后端在 `packages/core/src/http/quic.rs`，由 Cargo feature `quic` 控制
  （`quinn` 依赖），默认**不**编译进 `full`，避免影响既有 CI 构建。

### 4. 硬件加速指令集自动选择
- 新增 `packages/core/src/util/accel.rs`：运行时检测
  - x86_64：SSE2 / SSE4.2 / AVX / AVX2 / AVX-512F / AES-NI / SHA-NI
  - aarch64：NEON / ARMv8 AES / ARMv8 SHA-2
- 结果进程内缓存一次，并在服务启动时打印日志（`Hardware acceleration detected: ...`）。
- 提供 `sha256_backend()` / `aes_backend()` 选择语义，供 crypto/hash 后端按平台
  选择最优实现（SHA-NI/ARMv8 硬加速 → SIMD 软件路径 → 可移植路径）。

### 5. Zero-copy 直传
- 下载侧：`file_content_body()` / `file_reader_body()` 对
  `FileContent::Path` / `FileContent::Fd`（Android SAF）直接用
  `tokio_util::io::ReaderStream` 从文件流式读取，**不再经过中间 channel 多拷贝一次**。

### 6. 明文传输
- 原有架构本就支持 `tls_config: None`（明文 HTTP）。LocalSend+ 保留该路径，用于
  局域网内对速度敏感、且无需加密的场景。

---

## 尚未完成 / 待办（Roadmap）

| 功能 | 状态 | 说明 |
|------|------|------|
| QUIC 后端（quinn 0.11） | 脚手架已就位，**需编译验证** | `http/quic.rs` 按 quinn 0.11 API 编写，需 `--features quic` 编译并实测；若 API 有出入需微调。 |
| 上传侧（push 流程）断点续传 | 设计完成，待实现 | 需要在 `upload` 端点增加 `offset` 参数、`save.rs` 支持追加写入、`session.rs` 记录分片进度。下载侧已可用。 |
| 上传侧多线程分块 | 设计完成，待实现 | 与上一条同源：接收端需支持并发分片写入 + 按文件聚合校验。 |
| 蓝牙传输 | 未实现 | 需要引入蓝牙平台插件（Android/iOS/macOS）并新增发现 + 传输通道。 |
| 数据线 / USB 传输（仅 Android） | 未实现 | 需要 ADB/文件提供器通道或 SAF 文档访问，属 Android 原生层工作。 |
| WiFi 直连（WiFi Direct） | 未实现 | Android 需要 `WifiP2pManager` 原生通道；跨平台无统一 API。 |

---

## 验证方式（重要）

本仓库当前工作机**未安装 Rust toolchain（rustup 无默认工具链）与 Flutter**，
因此本次改动**未经过编译/测试**。请在有环境处执行：

```bash
# Rust core
cd packages/core
cargo test --features full

# QUIC 后端（单独编译验证）
cargo check --features full,quic

# Flutter（如修改了 Dart）
cd app
fvm flutter pub get
fvm flutter analyze
```

---

## 代码结构（新增/改动）

```
packages/core/
├── Cargo.toml                      # +quinn 依赖、+quic feature、tokio-util +io
├── src/
│   ├── util/accel.rs               # 新增：硬件加速检测
│   ├── util/mod.rs                 # +pub mod accel
│   ├── http/transport.rs           # 新增：QUIC/TCP 选择与降级
│   ├── http/quic.rs                # 新增：QUIC 后端（feature-gated）
│   ├── http/mod.rs                 # +transport、+quic 模块
│   ├── http/server/mod.rs          # 启动时打印加速/降级日志
│   ├── http/server/web.rs          # +Range 断点续传、+zero-copy 流式下载
│   └── http/client/v2.rs           # +download_range、+并行分块下载
app/
├── lib/pages/about/about_page.dart # 开发者标注 Goose x Akane
└── pubspec.yaml                    # 描述更新
```
