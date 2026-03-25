# winget-source-builder

[English](README.md) | [繁體中文](README.zh-TW.md)

`winget-source-builder` 是一个面向第三方仓库的静态 WinGet 源构建工具。它按文件状态而不是 Git 提交来跟踪变化，维护一个可增量更新的内部状态库，并输出包含 `source.msix` 或 `source2.msix`、所需 sidecar 文件以及托管合并清单的发布目录。

面向用户的消息现在通过 `locales/` 下的外部语言文件管理。新增语言不需要再修改 Rust 源代码。

## 功能

- 基于 SQLite 状态库的文件级增量构建。
- 使用 Rust 并行执行扫描、哈希、合并与差异计算。
- 内容寻址的托管清单路径与 `versionData.mszyml`。
- 兼容 WinGet 的 `source.msix` 与 `source2.msix` 输出。
- 核心层已经保留格式抽象，未来可以通过新增 writer 适配新的 catalog 版本。

## 依赖

- 完整 WinGetUtil 路径以及 `v2` sidecar 生成需要 Windows 10/11。
- 运行时需要 `winget-source-builder.exe` 同目录下的 `WinGetUtil.dll`。Windows 构建会默认从仓库内置的 `winget-cli` 子模块自动生成它。
- 需要 Windows SDK 的 `makeappx.exe` 或 `makemsix`。非 Windows 构建会默认从仓库内置的 `msix-packaging` 子模块构建 `makemsix`。
- 从源码仓库运行时需要 Rust stable，并执行 `git submodule update --init winget-cli msix-packaging`。
- 被索引的源仓库需要包含 `packaging/`，例如 `winget-source-template` 提供的模板结构。

## 快速开始

在源码仓库中构建：

```powershell
git submodule update --init winget-cli msix-packaging
cargo run -- build `
  --repo C:\path\to\source-repo\manifests `
  --state C:\path\to\builder-state `
  --out C:\path\to\publish-root `
  --lang zh-CN `
  --backend rust `
  --format v2
```

从打包好的 Windows 产物运行：

```powershell
.\winget-source-builder.exe build `
  --repo C:\path\to\source-repo\manifests `
  --state C:\path\to\builder-state `
  --out C:\path\to\publish-root `
  --lang zh-CN `
  --format v2
```

输出目录：

- `--format v1` 时输出 `source.msix`，`--format v2` 时输出 `source2.msix`
- `packages/<PackageIdentifier>/<hash8>/versionData.mszyml` 仅在 `--format v2` 下生成
- `manifests/...`

状态目录：

- `state.sqlite`
- `validation-queue.json`
- 使用 WinGetUtil backend 时会有 `writer/mutable-v1.db` 或 `writer/mutable-v2.db`

推荐参考 `winget-source-template` 的工作流模式：用 `robinraju/release-downloader` 从 GitHub Releases 下载预构建 builder，再在模板仓库自己的 workflow 里直接执行该二进制。

## 文档

- [使用说明](docs/zh-CN/usage.md)
- [架构说明](docs/zh-CN/architecture.md)
- [开发与 CI](docs/zh-CN/development.md)
