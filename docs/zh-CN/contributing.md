# 贡献指南

本文档涵盖 CI/CD 工作流、发布流程，以及下游项目使用本工具的指导。

## 目录

- [CI/CD 工作流](#cicd-工作流)
- [发布流程](#发布流程)
- [发布包内容](#发布包内容)
- [下游项目使用指南](#下游项目使用指南)
- [版本管理](#版本管理)

## CI/CD 工作流

项目使用 GitHub Actions 实现持续集成和部署。

### CI 工作流（`ci.yml`）

每次向 `main` 分支推送代码或提交 PR 时触发。

**执行内容：**

1. **代码质量检查**
   - `cargo fmt --all --check` — 确保代码格式一致
   - `cargo clippy --all-targets --all-features -- -D warnings` — 代码 lint 检查

2. **测试**
   - 在以下平台运行 `cargo test --verbose`：
     - Ubuntu（最新版）
     - macOS（最新版）
     - Windows（最新版）
   - 初始化 WinGetUtil 和 makemsix 子模块
   - 平台依赖不可用时测试会自动跳过

3. **构建产物**
   - 生成以下平台的发布二进制文件：
     - Linux x86_64
     - macOS（通用二进制）
     - Windows x86_64
     - Windows aarch64
   - 作为工作流产物上传（保留 90 天）

**查看结果：**

- 访问 [Actions 页面](https://github.com/SpecterShell/winget-source-builder/actions)
- 点击对应的工作流运行记录
- 从摘要页面下载产物

### 发布工作流（`release.yml`）

推送匹配 `v*` 格式的标签时触发（例如 `v1.2.3`）。

**执行内容：**

1. **构建阶段**
   - 检出代码和子模块
   - 为所有平台构建发布二进制文件
   - 预置 WinGetUtil.dll（Windows）和 makemsix（Linux/macOS）

2. **打包阶段**
   - 创建平台特定的归档文件：
     - Windows：`.zip` 文件
     - Linux/macOS：`.tar.gz` 文件
   - 包含对应的辅助二进制文件

3. **发布阶段**
   - 创建或更新 GitHub Release
   - 上传所有平台包
   - 从提交记录生成发布说明

**创建发布：**

```powershell
# 打版本标签
git tag v1.2.3

# 推送标签（触发发布工作流）
git push origin v1.2.3
```

## 发布流程

### 版本号规则

本项目遵循 [语义化版本规范](https://semver.org/)：

- **主版本号（MAJOR）** — 包含不兼容变更（CLI 变更、输出格式变更）
- **次版本号（MINOR）** — 新增功能，向后兼容
- **修订号（PATCH）** —  Bug 修复，向后兼容

### 发布检查清单

创建发布前请确认：

- [ ] 更新 `CHANGELOG.md` 中的发布说明
- [ ] 确认 `main` 分支所有测试通过
- [ ] 若未自动更新，需手动修改 `Cargo.toml` 中的版本号
- [ ] 确认文档已同步更新
- [ ] 创建并推送版本标签

### 预发布版本

用于测试或 Beta 版本：

```powershell
# 创建预发布版本
git tag v1.3.0-beta.1
git push origin v1.3.0-beta.1
```

GitHub Releases 会自动将这类版本标记为预发布。

## 发布包内容

### Windows 包

```
winget-source-builder-x86_64-pc-windows-msvc.zip
├── winget-source-builder.exe    # 主可执行文件
├── WinGetUtil.dll               # Windows 后端依赖库
└── LICENSE                      # 许可证文件
```

### Linux 包

```
winget-source-builder-x86_64-unknown-linux-gnu.tar.gz
├── winget-source-builder        # 主可执行文件
├── makemsix                     # MSIX 打包工具
└── LICENSE                      # 许可证文件
```

### macOS 包

```
winget-source-builder-universal-apple-darwin.tar.gz
├── winget-source-builder        # 主可执行文件（通用二进制）
├── makemsix                     # MSIX 打包工具
└── LICENSE                      # 许可证文件
```

### 注意事项

- **Windows：** `WinGetUtil.dll` 必须与可执行文件位于同一目录
- **Linux/macOS：** `makemsix` 必须位于同一目录或 PATH 环境变量中
- 打包的运行时构建默认依赖同级目录下的辅助二进制文件

## 下游项目使用指南

如果你在自己的 CI/CD 流水线中使用 `winget-source-builder`，可参考以下方式高效集成。

### 下载发布包

**推荐：使用发布下载器 Action**

```yaml
# GitHub Actions 示例
- name: 下载 winget-source-builder
  uses: robinraju/release-downloader@v1
  with:
    repository: SpecterShell/winget-source-builder
    tag: v1.0.0  # 固定到指定版本
    fileName: winget-source-builder-x86_64-pc-windows-msvc.zip
    extract: true
```

**PowerShell 示例：**

```powershell
$version = "1.0.0"
$url = "https://github.com/SpecterShell/winget-source-builder/releases/download/v$version/winget-source-builder-x86_64-pc-windows-msvc.zip"

Invoke-WebRequest -Uri $url -OutFile builder.zip
Expand-Archive -Path builder.zip -DestinationPath ./builder
```

### 固定版本

为保证构建可复现，请始终固定到特定版本：

```yaml
# 推荐：固定版本
- uses: robinraju/release-downloader@v1
  with:
    tag: v1.2.3

# 不推荐：使用最新版本（可能意外中断）
- uses: robinraju/release-downloader@v1
  with:
    latest: true
```

### 缓存配置

为加快 CI 速度，可缓存下载的二进制文件：

```yaml
- name: 缓存 winget-source-builder
  uses: actions/cache@v3
  with:
    path: ./builder
    key: builder-${{ runner.os }}-${{ env.BUILDER_VERSION }}

- name: 未命中缓存时下载
  if: steps.cache.outputs.cache-hit != 'true'
  uses: robinraju/release-downloader@v1
  with:
    tag: ${{ env.BUILDER_VERSION }}
    fileName: winget-source-builder-x86_64-pc-windows-msvc.zip
    extract: true
```

### CI 集成示例

```yaml
name: 构建源

on:
  push:
    paths:
      - 'manifests/**'

env:
  BUILDER_VERSION: '1.0.0'

jobs:
  build:
    runs-on: windows-latest
    steps:
      - uses: actions/checkout@v4

      - name: 下载构建工具
        uses: robinraju/release-downloader@v1
        with:
          repository: SpecterShell/winget-source-builder
          tag: v${{ env.BUILDER_VERSION }}
          fileName: winget-source-builder-x86_64-pc-windows-msvc.zip
          extract: true

      - name: 构建源
        run: |
          ./winget-source-builder.exe build `
            --repo-dir ./manifests `
            --state-dir ./state `
            --index-version v2

      - name: 发布源
        run: |
          ./winget-source-builder.exe publish `
            --state-dir ./state `
            --out-dir ./publish `
            --packaging-assets-dir ./packaging

      - name: 上传产物
        uses: actions/upload-artifact@v3
        with:
          name: source
          path: ./publish/
```

### 状态持久化

为实现 CI 运行间的增量构建，请持久化状态目录：

```yaml
- name: 恢复状态缓存
  uses: actions/cache@v3
  with:
    path: ./state
    key: builder-state-${{ github.run_id }}
    restore-keys: |
      builder-state-

- name: 增量构建
  run: |
    ./winget-source-builder.exe build `
      --repo-dir ./manifests `
      --state-dir ./state `
      --index-version v2
```

## 版本管理

### 兼容性保证

同一主版本内：

- CLI 参数保持向后兼容
- 输出格式保持稳定
- 状态数据库迁移自动执行

重大变更（主版本号升级）会在发布说明中记录，并提供迁移指南。

### 弃用策略

移除功能前的流程：

1. 在文档中标记功能为弃用
2. 在 CLI 输出中添加警告
3. 至少保留一个带警告的次版本
4. 在下一个主版本中正式移除

### API 稳定性

本工具是 CLI 应用，而非库，对外公开接口包括：

- 命令行参数
- 退出码
- JSON 输出格式（使用 `--json` 时）
- 环境变量

内部 API 可能随时变更，不另行通知。
