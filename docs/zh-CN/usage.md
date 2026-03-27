# 使用指南

本指南将指导你使用 `winget-source-builder` 创建和维护 WinGet 兼容的软件源。阅读完成后，你将理解「构建→发布」的两阶段工作流，以及如何利用增量构建能力提升效率。

## 目录

- [概述](#概述)
- [前置准备](#前置准备)
- [首次构建](#首次构建)
- [理解增量构建](#理解增量构建)
- [常见任务](#常见任务)
- [环境变量](#环境变量)
- [故障排查](#故障排查)

## 概述

`winget-source-builder` 的工作流分为两个独立阶段：

1. **构建阶段** — 扫描清单仓库、计算哈希、识别变更并准备暂存目录
2. **发布阶段** — 将暂存目录打包为 `source.msix`（v1）或 `source2.msix`（v2）并生成最终输出

这种分离设计支持发布前的构建校验，也适合高频构建、低频发布的增量工作流场景。

## 前置准备

开始前你需要准备：

- **WinGet 格式的清单仓库** — 按包标识符和版本组织的 YAML 清单（例如目录结构 `manifests/v/Vendor/App/1.0.0/`）
- **可写的状态目录** — 用于存储增量状态、暂存构建和验证队列
- **打包资源**（发布阶段需要） — 用于 MSIX 打包的 `AppxManifest.xml` 和 `Assets/` 目录
- **平台依赖：**
  - Windows：`WinGetUtil.dll`（已随发布包捆绑）
  - Linux/macOS：用于 MSIX 打包的 `makemsix` 工具

你可以从 [winget-source-template](https://github.com/SpecterShell/winget-source-template) 获取包含示例清单和打包资源的模板仓库。

## 首次构建

### 步骤 1：初始化配置

创建状态目录，所有构建状态都会存储在此处：

```powershell
mkdir ./state
```

构建工具会在此目录下自动生成以下文件：

- `state.sqlite` — 主状态数据库
- `validation-queue.json` — 待处理的安装程序验证队列
- `staging/` — 可发布的暂存构建目录

### 步骤 2：执行构建命令

`build` 命令会分析清单仓库并准备发布所需的所有内容：

```powershell
winget-source-builder build `
  --repo-dir ./manifests `
  --state-dir ./state `
  --backend rust `
  --index-version v2
```

执行过程：

1. **扫描** — 发现并读取所有 YAML 清单文件
2. **哈希计算** — 为文件内容计算哈希用于变更检测
3. **合并** — 将多文件清单合并为标准格式
4. **差异对比** — 与上一次状态对比识别变更
5. **暂存** — 在 `state/staging/` 目录生成可发布的文件树

`--index-version v2` 参数会生成现代源格式（`source2.msix`），仅当需要兼容旧版 WinGet 客户端时使用 `v1`。

### 步骤 3：检查构建结果

构建完成后，查看状态确认结果：

```powershell
winget-source-builder status --state-dir ./state
```

输出会展示：

- 工作状态中的包和版本数量
- 最新的待发布暂存构建
- 仓库中检测到的待处理变更

### 步骤 4：发布

现在生成最终的 MSIX 包：

```powershell
winget-source-builder publish `
  --state-dir ./state `
  --out-dir ./publish `
  --packaging-assets-dir ./packaging
```

输出目录将包含：

- `source2.msix` — 已签名（或未签名）的 MSIX 包
- `manifests/` — 供直接下载的托管合并清单
- `packages/` — v2 格式特有的版本数据附属文件

将这些文件部署到你的 Web 服务器后，用户即可添加你的软件源：

```powershell
winget source add --name mysource --argument https://your-domain.com/source2.msix
```

## 理解增量构建

本工具的核心优势之一是增量构建能力，工作原理如下：

### 首次构建 vs 后续构建

**首次构建：**

- 扫描所有清单文件
- 为每个文件计算哈希
- 处理所有版本
- 耗时较长（大型仓库可能需要数分钟）

**后续构建：**

- 对比文件哈希与存储的历史状态
- 仅处理发生变更的版本
- 通常几秒内即可完成

### 触发全量重建的场景

部分变更需要处理的内容超出单个修改文件：

| 变更类型 | 重处理范围 |
|-------------|----------------------|
| 单个清单编辑 | 仅对应版本 |
| 安装程序 URL 变更 | 对应版本 + 验证队列条目 |
| 包名称变更 | 包的所有版本（包级元数据） |
|  schema 变更 | 可能需要使用 `--force` 全量重建 |

### 强制干净构建

如果怀疑状态损坏或需要完全重置：

```powershell
winget-source-builder build `
  --repo-dir ./manifests `
  --state-dir ./state `
  --index-version v2 `
  --force
```

`--force` 参数会忽略现有状态，强制重建所有内容。

## 常见任务

### 添加新包版本

1. 将清单文件添加到仓库（例如 `manifests/v/Vendor/App/1.2.3/`）
2. 运行构建命令识别变更
3. 准备就绪后执行发布

如果需要快速测试单个版本而不扫描整个仓库：

```powershell
winget-source-builder add `
  --repo-dir ./manifests `
  --state-dir ./state `
  --version-dir ./manifests/v/Vendor/App/1.2.3
```

### 移除包

移除指定版本：

```powershell
winget-source-builder remove `
  --repo-dir ./manifests `
  --state-dir ./state `
  --package-id Vendor.App `
  --version 1.0.0
```

如果需要移除整个包（所有版本），可以为每个版本重复执行上述命令，或使用 `clean` 重置状态。

### 检查仓库状态

获取快速概览：

```powershell
winget-source-builder status --state-dir ./state
```

查看详细的待处理变更：

```powershell
winget-source-builder diff `
  --repo-dir ./manifests `
  --state-dir ./state
```

获取机器可读输出（适用于 CI 场景）：

```powershell
winget-source-builder diff --json > changes.json
```

### 发布前验证

校验暂存构建是否正确：

```powershell
winget-source-builder verify staged --state-dir ./state
```

校验已发布输出：

```powershell
winget-source-builder verify published `
  --state-dir ./state `
  --out-dir ./publish
```

### 代码签名

**Windows 环境**（使用 `signtool.exe`）：

```powershell
winget-source-builder publish `
  --state-dir ./state `
  --out-dir ./publish `
  --packaging-assets-dir ./packaging `
  --sign-pfx-file ./signing.pfx `
  --sign-password-env WINGET_SOURCE_SIGN_PASSWORD `
  --timestamp-url http://timestamp.digicert.com
```

**Linux/macOS 环境**（使用 `makemsix` + `openssl`）：

```powershell
winget-source-builder publish `
  --state-dir ./state `
  --out-dir ./publish `
  --packaging-assets-dir ./packaging `
  --sign-pfx-file ./signing.pfx `
  --sign-password-env WINGET_SOURCE_SIGN_PASSWORD
```

注意：时间戳 URL 目前仅支持 Windows 平台。

### 查看包信息

查看状态数据库中的包详情：

```powershell
# 显示包详情
winget-source-builder show package --state-dir ./state Vendor.App

# 显示指定版本详情
winget-source-builder show version `
  --state-dir ./state `
  --package-id Vendor.App `
  --version 1.2.3

# 输出 JSON 格式用于脚本处理
winget-source-builder show package Vendor.App --json
```

### 清理空间

状态目录会随时间增长，可清理旧构建释放空间：

```powershell
# 仅保留最近 5 次构建
winget-source-builder clean `
  --state-dir ./state `
  --builds `
  --keep-last 5

# 移除旧的暂存目录
winget-source-builder clean `
  --state-dir ./state `
  --staging

# 清理工作状态外的所有内容
winget-source-builder clean `
  --state-dir ./state `
  --all
```

### 问题诊断

运行诊断命令检查运行环境：

```powershell
winget-source-builder doctor `
  --repo-dir ./manifests `
  --state-dir ./state `
  --packaging-assets-dir ./packaging
```

检查内容包括：

- 所需工具是否可用
- 打包资源是否有效
- 后端兼容性
- 状态数据库健康度

## 环境变量

以下环境变量可修改默认行为，适用于默认配置不满足需求的场景：

| 变量 | 用途 |
|----------|---------|
| `WINGET_CLI_ROOT` | 自定义 `winget-cli` 检出路径，用于构建自定义 `WinGetUtil.dll` |
| `MSIX_PACKAGING_ROOT` | 自定义 `msix-packaging` 检出路径（使用 Mozilla 分支可支持签名） |
| `MAKEAPPX_EXE` | 显式指定 `makeappx.exe` 路径（Windows） |
| `MAKEMSIX_EXE` | 显式指定 `makemsix` 路径（Linux/macOS） |
| `OPENSSL` | 显式指定 `openssl` 二进制路径 |
| `WINGET_SOURCE_BUILDER_WORKSPACE_ROOT` | 打包资源自动发现的 fallback 工作区根路径 |
| `WINGET_SOURCE_BUILDER_LANG` | CLI 消息的运行时区域设置（例如 `en`、`zh-CN`） |

## 故障排查

### 构建失败，提示 "backend unavailable"

**问题：** `wingetutil` 后端仅支持 Windows 且需要 `WinGetUtil.dll`。

**解决方案：** 非 Windows 平台使用 `--backend rust`，Windows 平台确保 `WinGetUtil.dll` 与可执行文件位于同一目录。

### 发布失败，提示 "output directory drift"

**问题：** 输出目录包含来自不同构建的文件。

**解决方案：** 使用 `--force` 强制覆盖，或先清理输出目录。

### 增量构建未检测到变更

**问题：** 文件时间戳变更但内容未变（或相反）。

**解决方案：** 构建工具使用 SHA256 内容哈希而非时间戳检测变更，确认文件内容确实发生了修改。

### 构建后暂存内容丢失

**问题：** 暂存目录被删除或损坏。

**解决方案：** 运行 `build --force` 重新生成暂存文件树。

### Linux/macOS 上 MSIX 签名失败

**问题：** 默认 `makemsix` 不支持签名。

**解决方案：** 将 `MSIX_PACKAGING_ROOT` 设置为 Mozilla 支持签名的 `msix-packaging` 分支，并确保已安装 `openssl`。

### 清单合并错误

**问题：** 多文件清单存在字段冲突。

**解决方案：** 使用 `merge` 命令调试：

```powershell
winget-source-builder merge `
  --repo-dir ./manifests `
  --package-id Vendor.App `
  --version 1.2.3
```

该命令会输出合并结果，不会修改状态。