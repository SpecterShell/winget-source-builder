# 开发指南

本指南介绍如何从源码构建 `winget-source-builder`、运行测试，以及参与项目贡献。

## 目录

- [前置依赖](#前置依赖)
- [从源码构建](#从源码构建)
- [项目结构](#项目结构)
- [运行测试](#运行测试)
- [本地化](#本地化)
- [贡献指南](#贡献指南)

## 前置依赖

### 必备工具

- **Rust** — 最新稳定版（推荐 1.70+）
  - 通过 [rustup](https://rustup.rs/) 安装
- **Git** — 用于克隆仓库和子模块管理

### 平台特定依赖

**Windows：**

- PowerShell 7+（用于运行构建脚本）
- Visual Studio 2022 生成工具或完整 Visual Studio
  - 有关如何配置 Visual Studio，请参考 [winget-cli 的指南](https://github.com/microsoft/winget-cli/blob/master/doc/Developing.md)。
- Windows SDK

**Linux：**

- GCC 或 Clang 工具链
- CMake（3.15+）
- OpenSSL 开发头文件（用于支持签名）

**macOS：**

- Xcode 命令行工具
- CMake（3.15+）
- OpenSSL（通过 Homebrew 安装：`brew install openssl`）

### 克隆仓库

```powershell
git clone https://github.com/SpecterShell/winget-source-builder.git
cd winget-source-builder

# 初始化子模块（WinGetUtil 和 makemsix 依赖）
git -c submodule.recurse=false submodule update --init winget-cli msix-packaging
```

子模块提供：

- `winget-cli/` — 用于构建 `WinGetUtil.dll`（Windows）的源码
- `msix-packaging/` — 用于构建 `makemsix`（Linux/macOS）的源码

## 从源码构建

### 标准构建

```powershell
# 调试构建（编译更快，执行较慢）
cargo build

# 发布构建（编译较慢，执行优化）
cargo build --release
```

首次构建会执行：

1. 编译 Rust 代码
2. Windows 环境：从 `winget-cli` 子模块构建 `WinGetUtil.dll`
3. Linux/macOS 环境：从 `msix-packaging` 子模块构建 `makemsix`

### 构建产物

**Windows：**

- `target/debug/winget-source-builder.exe`
- `target/debug/WinGetUtil.dll`（从构建产物复制）

**Linux/macOS：**

- `target/debug/winget-source-builder`
- `target/debug/makemsix`（从子模块构建）

### 自定义 WinGetUtil 路径

如果你有独立的 `winget-cli` 检出目录：

```powershell
$env:WINGET_CLI_ROOT = "C:\path\to\winget-cli"
cargo build --release
```

### 自定义 makemsix 路径

Linux/macOS 环境下，如果你有独立的 `msix-packaging` 检出目录：

```powershell
$env:MSIX_PACKAGING_ROOT = "/path/to/msix-packaging"
cargo build --release
```

### 使用 Mozilla 支持签名的 makemsix

默认的 `msix-packaging` 在非 Windows 平台不支持签名。如需在 Linux/macOS 上使用签名功能：

```powershell
git clone https://github.com/mozilla/msix-packaging.git $env:MSIX_PACKAGING_ROOT
cargo build --release
```

## 项目结构

```
winget-source-builder/
├── src/
│   ├── main.rs           # CLI 入口，命令路由
│   ├── adapter.rs        # 后端抽象层
│   ├── backend.rs        # 后端实现
│   ├── builder.rs        # 核心构建编排
│   ├── i18n.rs           # 国际化配置
│   ├── manifest.rs       # 清单解析与合并
│   ├── mszip.rs          # ZIP 压缩工具
│   ├── progress.rs       # 进度报告
│   ├── state.rs          # 状态数据库操作
│   └── version.rs        # 版本比较与标准化
├── locales/              # 翻译文件
│   ├── en.yml
│   ├── zh-CN.yml
│   └── zh-TW.yml
├── scripts/              # 构建辅助脚本
│   ├── build-wingetutil.ps1
│   └── build-makemsix.sh
├── docs/                 # 文档
│   └── en/
│       ├── usage.md
│       ├── cli-reference.md
│       ├── architecture.md
│       └── development.md
├── winget-cli/           # Git 子模块（Windows 后端）
├── msix-packaging/       # Git 子模块（跨平台打包）
└── Cargo.toml
```

### 核心模块

| 模块 | 用途 |
|--------|---------|
| `builder.rs` | 编排构建流水线：扫描、哈希、差异对比、合并、索引生成 |
| `state.rs` | SQLite 数据库操作，实现增量状态管理 |
| `manifest.rs` | YAML 解析、多文件合并、标准化处理 |
| `adapter.rs` | `wingetutil` 和 `rust` 后端的抽象层 |
| `backend.rs` | 索引操作的后端实现 |
| `progress.rs` | 长时间操作的进度报告 |

## 运行测试

### 快速测试检查

提交变更前请运行以下命令：

```powershell
# 格式检查
cargo fmt --all --check

# Lint 检查
cargo clippy --all-targets --all-features -- -D warnings

# 运行所有测试
cargo test --verbose
```

### 测试分类

**单元测试：**

```powershell
# 仅运行单元测试
cargo test --lib --verbose
```

单元测试覆盖：

- 清单合并逻辑
- 哈希计算与过滤
- 版本比较
- 状态数据库操作

**端到端测试：**

```powershell
# 运行所有测试，包含端到端（需要平台依赖）
cargo test --verbose
```

端到端测试：

- 构建 `tests/data/e2e-repo/` 中的示例仓库
- 测试完整的构建→发布流水线
- 验证输出完整性
- 平台依赖不可用时自动跳过

**平台特定说明：**

- **Windows：** 端到端测试同时运行 `wingetutil` 和 `rust` 后端
- **Linux/macOS：** 端到端测试仅运行 `rust` 后端（需要 `makemsix`）

### 测试用例

`tests/data/e2e-repo/` 目录包含端到端测试使用的示例清单仓库，包含：

- 多文件清单
- 多种安装程序类型
- 合并与验证的边界场景

添加新功能时，建议向该示例仓库补充测试用例。

## 本地化

项目使用 `rust-i18n` 实现国际化，翻译文件存储在 `locales/` 目录下的 YAML 文件中。

### 添加新语言

1. 创建新文件：`locales/<locale>.yml`
2. 复制 `locales/en.yml` 的结构
3. 翻译所有字段
4. 通过 `WINGET_SOURCE_BUILDER_LANG=<locale>` 运行测试

### 翻译文件结构

```yaml
# locales/en.yml
hello: 你好
build:
  scanning: 正在扫描仓库...
  complete: 构建完成
error:
  not_found: 文件未找到
```

### 代码中使用翻译

```rust
use rust_i18n::t;

println!("{}", t!("build.scanning"));
```

### 测试本地化

```powershell
# 测试英文（默认）
winget-source-builder --lang en status --state-dir ./state

# 测试简体中文
winget-source-builder --lang zh-CN status --state-dir ./state

# 或通过环境变量设置
$env:WINGET_SOURCE_BUILDER_LANG = "zh-TW"
winget-source-builder status --state-dir ./state
```

## 贡献指南

### 参与流程

1. 在 GitHub 上 Fork 本仓库
2. 克隆你的 Fork 到本地
3. 为你的功能或修复创建新分支
4. 编写代码
5. 运行测试检查（格式、Lint、单元测试）
6. 提交代码，提交信息清晰描述变更
7. 推送分支并提交 Pull Request

### 代码风格

- 遵循 `rustfmt` 规范（执行 `cargo fmt`）
- 修复所有 `clippy` 警告
- 为公共 API 编写文档注释
- 为新功能添加测试用例

### 提交信息规范

使用清晰、描述性的提交信息：

```
添加自定义验证队列支持

- 新增 --validation-queue-dir 选项
- 更新队列文件的状态跟踪
- 添加队列持久化测试
```

### Pull Request 流程

1. 确保所有 CI 检查通过
2. 如有需要，更新相关文档
3. 关联相关的 Issue
4. 请求维护者评审

### 反馈问题

报告 Bug 时请包含以下信息：

- 操作系统及版本
- Rust 版本（`rustc --version`）
- 构建工具版本或提交哈希
- 复现步骤
- 预期行为与实际行为
- 完整错误输出（如有 `--verbose` 日志请一并提供）

### CI/CD 说明

项目使用 GitHub Actions 实现 CI：

- **ci.yml** — 每次推送和 PR 时运行
  - 格式检查
  - Clippy Lint 检查
  - Linux、macOS、Windows 平台测试
  - 各平台构建产物生成

- **release.yml** — 版本标签（`v*`）推送时运行
  - 发布构建
  - 资源打包
  - GitHub Release 创建

更多 CI/CD 和发布流程细节请参考 [贡献指南](contributing.md)。
