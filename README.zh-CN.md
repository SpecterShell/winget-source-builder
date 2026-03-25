# winget-source-builder

[English](README.md) | [繁體中文](README.zh-TW.md)

`winget-source-builder` 是一个面向 Windows 的静态 WinGet 源构建工具，目标是让第三方仓库也能生成可静态托管的 WinGet 源。它按文件状态而不是 Git 提交来跟踪变化，维护一个可增量更新的内部状态库，并输出包含 `source2.msix`、包级 sidecar 文件和托管合并清单的发布目录。

面向用户的消息现在通过 `locales/` 下的外部语言文件管理。新增语言不需要再修改 Rust 源代码。

## 功能

- 基于 SQLite 状态库的文件级增量构建。
- 使用 Rust 并行执行扫描、哈希、合并与差异计算。
- 内容寻址的托管清单路径与 `versionData.mszyml`。
- 兼容 WinGet 的 `source2.msix` 输出。
- 核心层已经保留格式抽象，未来可以通过新增 writer 适配新的 catalog 版本。

## 依赖

- 完整构建流程需要 Windows 10/11。
- 运行时需要 `winget-source-builder.exe` 同目录下的 `WinGetUtil.dll`。Windows 构建会默认从仓库内置的 `winget-cli` 子模块自动生成它。
- 需要 Windows SDK 的 `makeappx.exe`，或通过 `MAKEAPPX_EXE` 指定其路径。
- 从源码仓库运行时需要 Rust stable，并执行 `git submodule update --init --recursive`。
- 被索引的源仓库需要包含 `packaging/`，例如 `winget-source-template` 提供的模板结构。

## 快速开始

在源码仓库中构建：

```powershell
git submodule update --init --recursive
cargo run -- build `
  --repo C:\path\to\source-repo\manifests `
  --state C:\path\to\builder-state `
  --out C:\path\to\publish-root `
  --lang zh-CN `
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

- `source2.msix`
- `packages/<PackageIdentifier>/<hash8>/versionData.mszyml`
- `manifests/...`

状态目录：

- `state.sqlite`
- `validation-queue.json`
- `writer/mutable-v2.db`

## GitHub Action

本仓库同时提供一个可复用的 GitHub Action，入口位于 [action.yml](https://github.com/SpecterShell/winget-source-builder/blob/main/action.yml)。

推荐的消费方是一个包含以下目录的源模板仓库：

- `manifests/`
- `packaging/`

目录布局与工作流示例可参考 `winget-source-template`。

## 文档

- [使用说明](docs/zh-CN/usage.md)
- [架构说明](docs/zh-CN/architecture.md)
- [开发与 CI](docs/zh-CN/development.md)
