# 开发与 CI

## 本地开发

推荐的本地检查命令：

```powershell
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
cargo test --verbose
```

在 Windows 上，`build.rs` 会在编译时把 `WinGetUtil.dll` 放到生成的可执行文件旁边。它会按这个顺序尝试：

- `WINGET_CLI_ROOT` 或仓库内置的 `winget-cli` 子模块，再调用 `scripts/build-wingetutil.ps1`

在 Linux 和 macOS 上，`build.rs` 会在编译时把 `makemsix` 放到生成的可执行文件旁边。它会按这个顺序尝试：

- `MSIX_PACKAGING_ROOT` 或仓库内置的 `msix-packaging` 子模块，再调用 `scripts/build-makemsix.sh`

构建过程不再接受 DLL 路径覆盖，也不再扫描兄弟目录中的旧 `WinGetUtil.dll` 输出，更不兼容历史遗留的运行时搜索路径。干净工作区应当依赖内置子模块或显式设置 `WINGET_CLI_ROOT`。

## 测试覆盖

- Rust 单元测试覆盖多文件清单合并与安装器哈希过滤。
- Windows 端到端测试会构建 `tests/data/e2e-repo` 中的示例仓库。
- 当 `makemsix` 可用时，Rust `v1` 端到端测试也可以在 Linux 和 macOS 上运行。
- 如果机器上缺少对应 backend 所需的运行时或打包依赖，端到端测试会自动跳过。
- i18n 运行时测试覆盖 locale 规范化、回退行为，以及从 `locales/` 加载翻译。

## 本地化

- CLI 面向用户的消息由 `rust-i18n` 提供。
- 翻译字符串存放在 `locales/` 中，而不是硬编码在 Rust 源码里。
- 新增一种语言通常只需要新增语言文件，除非程序增加了新的消息键。

## GitHub Actions

仓库内置两个工作流：

- `ci.yml`
  - 执行 `cargo fmt --all --check`
  - 执行 `cargo clippy --all-targets --all-features -- -D warnings`
  - 在 Linux、macOS、Windows 上执行 `cargo test --verbose`
  - 在 test/build 作业中检出一级子模块，让 `build.rs` 自动准备 `WinGetUtil.dll` 和 `makemsix`
  - 生成 Linux x86_64、macOS、Windows x86_64 与 Windows aarch64 的 workflow artifact
- `release.yml`
  - 在 `v*` tag 上触发
  - 以 release 模式构建 Rust CLI
  - 在编译过程中由 `build.rs` 准备 `WinGetUtil.dll`
  - 打包 Linux x86_64、macOS、Windows x86_64 与 Windows aarch64 发布包，并上传到 GitHub Release

下游仓库应当在自己的 workflow 中直接下载本仓库发布的 Windows release 产物，例如使用 `robinraju/release-downloader`，而不是再依赖本仓库提供的复用 Action。

## 发布包结构

Windows 发布 zip 内包含：

- `winget-source-builder.exe`
- `WinGetUtil.dll`

Rust 主程序需要从被索引的源模板仓库中读取 `packaging/`，而不是从 builder 发布包中读取。
