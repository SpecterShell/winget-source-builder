# CLI 参考手册

本文档是 `winget-source-builder` 所有命令、选项和退出码的完整参考。

## 目录

- [全局选项](#全局选项)
- [命令组](#命令组)
  - [核心工作流](#核心工作流)
  - [仓库管理](#仓库管理)
  - [检查与调试](#检查与调试)
  - [维护操作](#维护操作)
- [退出码](#退出码)

## 全局选项

这些选项适用于大多数命令：

| 选项 | 描述 |
|--------|-------------|
| `--lang <locale>` | 覆盖显示语言（例如 `en`、`zh-CN`、`zh-TW`） |
| `--dry-run` | 模拟执行，不实际修改内容 |
| `--force` | 覆盖现有数据，忽略安全检查 |
| `--json` | 输出机器可读的 JSON 格式（适用于报告类命令） |

### 索引版本选择

许多命令支持 `--index-version` 参数选择源格式：

- `--index-version v1` — 旧版格式（`source.msix`）
- `--index-version v2` — 现代格式（`source2.msix`，推荐）

### 显示版本冲突策略

对于修改状态的命令，你可以控制 ARP 版本冲突的处理方式：

| 策略 | 行为 |
|----------|----------|
| `latest` | 保留最新版本（默认） |
| `oldest` | 保留最旧版本 |
| `strip-all` | 移除所有冲突的显示版本 |
| `error` | 检测到冲突时终止执行 |

## 命令组

### 核心工作流

日常使用的命令：`build` 和 `publish`。

#### `build`

**用途：** 扫描仓库、识别变更、更新状态并暂存可发布的构建版本。

**使用场景：** 在添加、更新或删除清单后运行，是构建→发布工作流的第一步。

```powershell
winget-source-builder build `
  --repo-dir <dir> `
  --state-dir <dir> `
  [--backend <wingetutil|rust>] `
  [--index-version <v1|v2>] `
  [--package-id <id>...] `
  [--version-dir <dir>...] `
  [--force] `
  [--dry-run] `
  [--no-validation-queue] `
  [--display-version-conflict-strategy <latest|oldest|strip-all|error>]
```

**选项：**

| 选项 | 描述 |
|--------|-------------|
| `--repo-dir` | **必填** 清单仓库路径 |
| `--state-dir` | **必填** 状态目录路径 |
| `--backend` | 索引操作后端：`wingetutil`（仅 Windows）或 `rust`（默认） |
| `--index-version` | 源格式版本：`v1` 或 `v2`（默认：`v2`） |
| `--package-id` | 仅构建指定包，可重复使用 |
| `--version-dir` | 仅构建指定版本目录，可重复使用 |
| `--force` | 忽略现有状态，强制重建所有内容 |
| `--dry-run` | 显示将发生的变更，不更新状态 |
| `--no-validation-queue` | 跳过生成 validation-queue.json |
| `--display-version-conflict-strategy` | ARP 版本冲突处理策略 |

**示例：**

```powershell
# 全仓库构建
winget-source-builder build --repo-dir ./manifests --state-dir ./state

# 使用 Rust 后端的 v2 格式增量构建
winget-source-builder build `
  --repo-dir ./manifests `
  --state-dir ./state `
  --backend rust `
  --index-version v2

# 仅构建指定包
winget-source-builder build `
  --repo-dir ./manifests `
  --state-dir ./state `
  --package-id Vendor.App1 `
  --package-id Vendor.App2

# 强制重建所有内容
winget-source-builder build `
  --repo-dir ./manifests `
  --state-dir ./state `
  --force
```

#### `publish`

**用途：** 将暂存的构建打包为 MSIX 并写入最终发布目录。

**使用场景：** 在 `build` 执行完成且准备好部署时运行，用于生成可分发的文件。

```powershell
winget-source-builder publish `
  --state-dir <dir> `
  --out-dir <dir> `
  --packaging-assets-dir <dir> `
  [--build-id <id>] `
  [--force] `
  [--dry-run] `
  [--sign-pfx-file <file>] `
  [--sign-password <value>] `
  [--sign-password-env <ENV>] `
  [--timestamp-url <url>]
```

**选项：**

| 选项 | 描述 |
|--------|-------------|
| `--state-dir` | **必填** 状态目录路径 |
| `--out-dir` | **必填** 最终输出路径 |
| `--packaging-assets-dir` | **必填** 包含 `AppxManifest.xml` 和 `Assets/` 目录的打包资源路径 |
| `--build-id` | 发布指定构建版本，而非最新的暂存版本 |
| `--force` | 即使输出目录与跟踪状态不一致也强制覆盖 |
| `--dry-run` | 显示将写入的内容，不实际创建文件 |
| `--sign-pfx-file` | 用于代码签名的 PFX 证书路径 |
| `--sign-password` | PFX 文件的密码（推荐使用更安全的 `--sign-password-env`） |
| `--sign-password-env` | 存储 PFX 密码的环境变量名称 |
| `--timestamp-url` | 时间戳服务器 URL（仅 Windows 支持） |

**示例：**

```powershell
# 基础发布（无签名）
winget-source-builder publish `
  --state-dir ./state `
  --out-dir ./publish `
  --packaging-assets-dir ./packaging

# 带代码签名的发布
winget-source-builder publish `
  --state-dir ./state `
  --out-dir ./publish `
  --packaging-assets-dir ./packaging `
  --sign-pfx-file ./cert.pfx `
  --sign-password-env CERT_PASSWORD

# 发布指定的历史构建版本
winget-source-builder publish `
  --state-dir ./state `
  --out-dir ./publish `
  --packaging-assets-dir ./packaging `
  --build-id 42
```

---

### 仓库管理

无需全量重建即可操作状态的命令。

#### `add`

**用途：** 增量添加指定版本到工作状态。

**使用场景：** 当你只需添加单个版本而无需扫描整个仓库时使用，比全量 `build` 更高效。

```powershell
winget-source-builder add `
  --repo-dir <dir> `
  --state-dir <dir> `
  (--version-dir <dir>... | --manifest-file <file>... | --package-id <id> --version <ver>) `
  [--backend <wingetutil|rust>] `
  [--index-version <v1|v2>] `
  [--force] `
  [--dry-run] `
  [--no-validation-queue] `
  [--display-version-conflict-strategy <latest|oldest|strip-all|error>]
```

**示例：**

```powershell
# 通过版本目录添加
winget-source-builder add `
  --repo-dir ./manifests `
  --state-dir ./state `
  --version-dir ./manifests/v/Vendor/App/1.2.3

# 通过包 ID 和版本添加
winget-source-builder add `
  --repo-dir ./manifests `
  --state-dir ./state `
  --package-id Vendor.App `
  --version 1.2.3

# 添加单个清单文件
winget-source-builder add `
  --repo-dir ./manifests `
  --state-dir ./state `
  --manifest-file ./manifests/v/Vendor/App/1.2.3/Vendor.App.yaml
```

#### `remove` / `delete`

**用途：** 从工作状态中增量移除指定版本。

**使用场景：** 当你需要删除某个版本而无需全量重建时使用。`delete` 是 `remove` 的完全别名。

```powershell
winget-source-builder remove `
  --repo-dir <dir> `
  --state-dir <dir> `
  (--version-dir <dir>... | --manifest-file <file>... | --package-id <id> --version <ver>) `
  [--backend <wingetutil|rust>] `
  [--index-version <v1|v2>] `
  [--force] `
  [--dry-run] `
  [--no-validation-queue] `
  [--display-version-conflict-strategy <latest|oldest|strip-all|error>]
```

**示例：**

```powershell
# 通过包 ID 和版本移除
winget-source-builder remove `
  --repo-dir ./manifests `
  --state-dir ./state `
  --package-id Vendor.App `
  --version 1.0.0

# 通过版本目录移除
winget-source-builder remove `
  --repo-dir ./manifests `
  --state-dir ./state `
  --version-dir ./manifests/v/Vendor/App/1.0.0
```

#### `diff`

**用途：** 对比当前仓库内容与工作状态的差异。

**使用场景：** 在运行 `build` 前查看变更内容，在 CI 中用于判断是否需要执行构建。

```powershell
winget-source-builder diff `
  --repo-dir <dir> `
  --state-dir <dir> `
  [--package-id <id>...] `
  [--version-dir <dir>...] `
  [--json]
```

**示例：**

```powershell
# 人类可读格式的差异输出
winget-source-builder diff --repo-dir ./manifests --state-dir ./state

# 机器可读的 JSON 格式差异（适用于 CI）
winget-source-builder diff `
  --repo-dir ./manifests `
  --state-dir ./state `
  --json > changes.json

# 仅对比指定包的差异
winget-source-builder diff `
  --repo-dir ./manifests `
  --state-dir ./state `
  --package-id Vendor.App
```

#### `status`

**用途：** 显示当前状态摘要、构建指针，可选包含差异信息。

**使用场景：** 快速查看仓库状态概览，无需执行全量差异对比。

```powershell
winget-source-builder status `
  --state-dir <dir> `
  [--repo-dir <dir>] `
  [--json]
```

**示例：**

```powershell
# 快速状态概览
winget-source-builder status --state-dir ./state

# 状态信息包含待处理变更
winget-source-builder status `
  --state-dir ./state `
  --repo-dir ./manifests

# 输出 JSON 格式用于脚本处理
winget-source-builder status --state-dir ./state --json
```

---

### 检查与调试

用于查看数据和验证一致性的命令。

#### `list-builds`

**用途：** 显示状态数据库中的近期构建记录。

```powershell
winget-source-builder list-builds `
  --state-dir <dir> `
  [--limit <n>] `
  [--status <running|staged|published|failed>] `
  [--json]
```

| 选项 | 描述 |
|--------|-------------|
| `--limit` | 最多显示的构建数量（默认：20） |
| `--status` | 按构建状态过滤 |

**示例：**

```powershell
# 显示最近 10 次构建
winget-source-builder list-builds --state-dir ./state --limit 10

# 仅显示已发布的构建
winget-source-builder list-builds `
  --state-dir ./state `
  --status published
```

#### `show`

**用途：** 查看状态中的构建、包、版本或安装程序哈希详情。

```powershell
# 显示构建详情
winget-source-builder show build --state-dir <dir> <build-id> [--json]

# 显示包详情
winget-source-builder show package --state-dir <dir> <package-id> [--json]

# 显示版本详情
winget-source-builder show version `
  --state-dir <dir> `
  (--version-dir <dir> | --package-id <id> --version <ver>) `
  [--json]

# 显示安装程序详情
winget-source-builder show installer --state-dir <dir> <installer-hash> [--json]
```

**示例：**

```powershell
# 显示包信息
winget-source-builder show package --state-dir ./state Vendor.App

# 以 JSON 格式显示版本信息
winget-source-builder show version `
  --state-dir ./state `
  --package-id Vendor.App `
  --version 1.2.3 `
  --json

# 显示构建详情
winget-source-builder show build --state-dir ./state 42
```

#### `verify`

**用途：** 对照跟踪状态校验暂存或已发布的输出。

**使用场景：** 在部署前后验证输出完整性。

```powershell
# 校验暂存构建
winget-source-builder verify staged `
  --state-dir <dir> `
  [--build-id <id>] `
  [--json]

# 校验已发布输出
winget-source-builder verify published `
  --state-dir <dir> `
  --out-dir <dir> `
  [--json]
```

**示例：**

```powershell
# 校验暂存构建
winget-source-builder verify staged --state-dir ./state

# 校验指定的已发布输出
winget-source-builder verify published `
  --state-dir ./state `
  --out-dir ./publish
```

#### `hash`

**用途：** 打印仓库目标的内容哈希和每个安装程序的哈希。

**使用场景：** 调试哈希不匹配问题或验证清单内容。

```powershell
winget-source-builder hash `
  --repo-dir <dir> `
  (--version-dir <dir> | --package-id <id> --version <ver>) `
  [--json]
```

**示例：**

```powershell
# 显示某个版本的哈希值
winget-source-builder hash `
  --repo-dir ./manifests `
  --package-id Vendor.App `
  --version 1.2.3

# 输出 JSON 格式
winget-source-builder hash `
  --repo-dir ./manifests `
  --version-dir ./manifests/v/Vendor/App/1.2.3 `
  --json
```

#### `merge`

**用途：** 以标准格式打印仓库目标的合并后清单。

**使用场景：** 调试多文件清单合并问题或查看最终合并输出。

```powershell
winget-source-builder merge `
  --repo-dir <dir> `
  (--version-dir <dir> | --package-id <id> --version <ver>) `
  [--output-file <file>] `
  [--json]
```

**示例：**

```powershell
# 打印合并后的清单到标准输出
winget-source-builder merge `
  --repo-dir ./manifests `
  --package-id Vendor.App `
  --version 1.2.3

# 保存到文件
winget-source-builder merge `
  --repo-dir ./manifests `
  --package-id Vendor.App `
  --version 1.2.3 `
  --output-file ./merged.yaml
```

---

### 维护操作

用于清理和诊断的命令。

#### `clean`

**用途：** 移除衍生数据以释放空间或重置状态。

**使用场景：** 定期执行以回收磁盘空间，或排查状态问题时使用。

```powershell
winget-source-builder clean `
  --state-dir <dir> `
  [--staging] `
  [--builds] `
  [--validation-queue] `
  [--published-tracking] `
  [--backend-cache] `
  [--all] `
  [--keep-last <n>] `
  [--older-than <duration>] `
  [--dry-run] `
  [--force]
```

| 选项 | 描述 |
|--------|-------------|
| `--staging` | 清理暂存构建目录 |
| `--builds` | 清理构建历史 |
| `--validation-queue` | 清理验证队列文件 |
| `--published-tracking` | 清理已发布构建跟踪数据 |
| `--backend-cache` | 清理后端特定缓存 |
| `--all` | 选择所有可清理数据（工作状态除外） |
| `--keep-last` | 清理构建历史时保留最近 N 条记录 |
| `--older-than` | 仅移除早于指定时长的内容（例如 `7d`、`24h`） |

**示例：**

```powershell
# 清理旧的暂存目录
winget-source-builder clean --state-dir ./state --staging

# 仅保留最近 5 次构建记录
winget-source-builder clean `
  --state-dir ./state `
  --builds `
  --keep-last 5

# 清理工作状态外的所有内容
winget-source-builder clean --state-dir ./state --all

# 预览将清理的内容
winget-source-builder clean `
  --state-dir ./state `
  --all `
  --dry-run
```

#### `doctor`

**用途：** 检查运行环境、打包资源、后端/索引兼容性和状态健康度。

**使用场景：** 排查问题的第一步，或作为 CI 中的前置检查。

```powershell
winget-source-builder doctor `
  [--repo-dir <dir>] `
  [--state-dir <dir>] `
  [--packaging-assets-dir <dir>] `
  [--backend <wingetutil|rust>] `
  [--index-version <v1|v2>] `
  [--json]
```

**示例：**

```powershell
# 基础健康检查
winget-source-builder doctor

# 带路径的完整检查
winget-source-builder doctor `
  --repo-dir ./manifests `
  --state-dir ./state `
  --packaging-assets-dir ./packaging

# 输出 JSON 格式用于 CI
winget-source-builder doctor --json > health-check.json
```

---

## 退出码

| 编码 | 含义 |
|------|---------|
| `0` | 执行成功 |
| `1` | 通用错误 |
| `2` | 参数或用法无效 |
| `3` | 仓库或状态未找到 |
| `4` | 后端不可用 |
| `5` | 验证失败 |
| `6` | 检测到输出目录漂移（发布时） |
| `7` | 签名失败 |
| `8` | 检测到状态损坏 |
