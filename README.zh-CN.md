# winget-source-builder

[简体中文](README.zh-CN.md) | [繁體中文](README.zh-TW.md)

[![CI 状态](https://github.com/SpecterShell/winget-source-builder/actions/workflows/ci.yml/badge.svg)](https://github.com/SpecterShell/winget-source-builder/actions/workflows/ci.yml)
[![许可证](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![最新版本](https://img.shields.io/github/v/release/SpecterShell/winget-source-builder)](https://github.com/SpecterShell/winget-source-builder/releases)

> 从你的清单仓库构建 WinGet 兼容的源索引 — 快速、增量、开箱即可发布。

`winget-source-builder` 帮助你搭建私有的 WinGet 软件包仓库。如果你维护了一套软件清单集合，希望用户可以通过 `winget source add` 命令从你的源安装软件，本工具可以帮你生成所需的索引和安装包。

构建工具会扫描你的 YAML 清单，通过 SHA256 哈希跟踪文件变更，维护增量状态数据库。每次运行仅处理发生变更的内容，后续构建几乎可以瞬间完成。最终输出可直接部署的 MSIX 包（`source.msix` 或 `source2.msix`）以及托管清单文件。

**适用人群：** 软件包仓库维护者、软件分发方，以及需要替代微软社区仓库、搭建私有或公共 WinGet 源的企业组织。

## 安装方式

### 下载预构建二进制文件

从 [GitHub 发布页面](https://github.com/SpecterShell/winget-source-builder/releases) 获取对应平台的最新版本。

**Windows：** 解压 zip 包后，将 `winget-source-builder.exe` 和 `WinGetUtil.dll` 放到 PATH 目录下，或直接在当前目录使用。

**Linux/macOS：** 解压归档文件，打包功能需要依赖 `makemsix` 工具（构建方法参考 [开发指南](docs/en/development.md)）。

### 从源码构建

```powershell
git clone https://github.com/SpecterShell/winget-source-builder.git
cd winget-source-builder
git -c submodule.recurse=false submodule update --init winget-cli msix-packaging
cargo build --release
```

详细的环境配置说明请查看 [开发指南](docs/en/development.md)。

## 快速开始

工作流分为两个阶段：**构建**（准备内容）和 **发布**（打包输出）。

```powershell
# 步骤 1：构建源索引
winget-source-builder build `
  --repo-dir ./manifests `
  --state-dir ./state `
  --index-version v2

# 步骤 2：发布 MSIX 包
winget-source-builder publish `
  --state-dir ./state `
  --out-dir ./publish `
  --packaging-assets-dir ./packaging
```

命令执行完成后，你将得到以下输出：

- `publish/source2.msix` — 主源安装包（v2 索引格式）
- `publish/manifests/` — 托管的合并清单文件
- `publish/packages/` — 版本数据附属文件（仅 v2 格式包含）

用户即可通过以下命令添加你的软件源：

```powershell
winget source add --name mysource --type Microsoft.PreIndexed.Package --arg https://your-domain.com/path/to/source/
```

## 常见工作流

### 首次使用配置

刚接触 WinGet 源搭建？可以先阅读 [使用指南](docs/en/usage.md)，获取完整的操作教程，包括：

- 清单仓库的目录结构配置
- 打包资源创建（AppxManifest.xml、图标）
- 首次构建的完整流程

### 日常操作（添加/更新包）

在仓库中添加或更新清单后，执行以下操作：

```powershell
# 直接运行构建命令，将自动检测变更并增量更新
winget-source-builder build `
  --repo-dir ./manifests `
  --state-dir ./state `
  --index-version v2

# 查看具体变更内容
winget-source-builder diff --repo-dir ./manifests --state-dir ./state
```

构建工具会对比文件哈希与上一次的状态，仅重新处理发生变更的版本。

### 发布版本

准备好发布新版本时执行：

```powershell
# 基础发布（无签名）
winget-source-builder publish `
  --state-dir ./state `
  --out-dir ./publish `
  --packaging-assets-dir ./packaging

# 带代码签名的发布（Windows 平台）
winget-source-builder publish `
  --state-dir ./state `
  --out-dir ./publish `
  --packaging-assets-dir ./packaging `
  --sign-pfx-file ./cert.pfx `
  --sign-password-env CERT_PASSWORD
```

### 查看变更内容

构建前可以查看与上一次发布状态的差异：

```powershell
winget-source-builder diff --repo-dir ./manifests --state-dir ./state
```

或获取完整的状态报告：

```powershell
winget-source-builder status --repo-dir ./manifests --state-dir ./state
```

## 更多文档

- **[使用指南](docs/en/usage.md)** — 分步教程，涵盖首次构建、增量构建原理和常见任务处理
- **[CLI 参考手册](docs/en/cli-reference.md)** — 完整的命令文档与示例
- **[架构设计](docs/en/architecture.md)** — 底层工作原理：哈希模型、状态管理和构建流水线
- **[开发指南](docs/en/development.md)** — 源码构建、测试运行和贡献指南
- **[贡献说明](docs/en/contributing.md)** — CI/CD 工作流和发布流程说明

## 许可证

采用 MIT 许可证，详情请查看 [LICENSE](LICENSE) 文件。
