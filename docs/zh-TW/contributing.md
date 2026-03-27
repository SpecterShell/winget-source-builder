# 貢獻指南

本文件涵蓋 CI/CD 工作流程、發布流程，以及下游專案使用此工具的指南。

## 目錄

- [CI/CD 工作流程](#cicd-工作流程)
- [發布流程](#發布流程)
- [發布套件內容](#發布套件內容)
- [面向下游專案](#面向下游專案)
- [版本控制](#版本控制)

## CI/CD 工作流程

專案使用 GitHub Actions 進行持續整合和部署。

### CI 工作流程 (`ci.yml`)

在每次推送到 `main` 和 Pull Request 時觸發。

**它做什麼：**

1. **程式碼品質檢查**
   - `cargo fmt --all --check` — 確保格式一致
   - `cargo clippy --all-targets --all-features -- -D warnings` — 程式碼檢查

2. **測試**
   - 在以下平台執行 `cargo test --verbose`：
     - Ubuntu (latest)
     - macOS (latest)
     - Windows (latest)
   - 為 WinGetUtil 和 makemsix 初始化子模組
   - 平台依賴不可用時自動跳過測試

3. **建置成品**
   - 為以下平台產生發布二進位檔：
     - Linux x86_64
     - macOS (universal)
     - Windows x86_64
     - Windows aarch64
   - 上傳為工作流程成品（保留 90 天）

**檢視結果：**

- 前往 [Actions 標籤頁](https://github.com/SpecterShell/winget-source-builder/actions)
- 點擊一個工作流程執行
- 從摘要頁下載成品

### 發布工作流程 (`release.yml`)

推送匹配 `v*` 的標籤時觸發（例如 `v1.2.3`）。

**它做什麼：**

1. **建置階段**
   - 檢出程式碼和子模組
   - 為所有平台建置發布二進位檔
   - 提供 WinGetUtil.dll（Windows）和 makemsix（Linux/macOS）

2. **封裝階段**
   - 建立平台特定封存檔：
     - Windows：`.zip` 檔案
     - Linux/macOS：`.tar.gz` 檔案
   - 包含適當的輔助二進位檔

3. **發布階段**
   - 建立或更新 GitHub Release
   - 上傳所有平台套件
   - 從提交產生發布說明

**建立發布：**

```powershell
# 打標籤
git tag v1.2.3

# 推送標籤（觸發發布工作流程）
git push origin v1.2.3
```

## 發布流程

### 版本號規則

本專案遵循[語意化版本控制](https://semver.org/lang/zh-TW/)：

- **主版本** — 破壞性變更（CLI 變更、輸出格式變更）
- **次版本** — 新功能，向後相容
- **修補版本** — 錯誤修正，向後相容

### 發布檢查清單

建立發布前：

- [ ] 在 `CHANGELOG.md` 中新增發布說明
- [ ] 確保 `main` 上的所有測試通過
- [ ] 如果尚未完成，更新 `Cargo.toml` 中的版本
- [ ] 驗證文件是最新的
- [ ] 建立並推送版本標籤

### 預發布版本

用於測試版本：

```powershell
# 建立預發布版本
git tag v1.3.0-beta.1
git push origin v1.3.0-beta.1
```

GitHub Releases 會自動將這些標記為預發布。

## 發布套件內容

### Windows 套件

```
winget-source-builder-x86_64-pc-windows-msvc.zip
├── winget-source-builder.exe    # 主可執行檔
├── WinGetUtil.dll               # Windows 後端程式庫
└── LICENSE                      # 授權檔案
```

### Linux 套件

```
winget-source-builder-x86_64-unknown-linux-gnu.tar.gz
├── winget-source-builder        # 主可執行檔
├── makemsix                     # MSIX 封裝工具
└── LICENSE
```

### macOS 套件

```
winget-source-builder-universal-apple-darwin.tar.gz
├── winget-source-builder        # 主可執行檔（通用二進位檔）
├── makemsix                     # MSIX 封裝工具
└── LICENSE
```

### 說明

- **Windows：** `WinGetUtil.dll` 必須與可執行檔在同一目錄
- **Linux/macOS：** `makemsix` 必須在同一目錄或 PATH 中
- 封裝的執行期建置期望輔助二進位檔在相同目錄

## 面向下游專案

如果你在自己的 CI/CD 流程中使用 `winget-source-builder`，以下是如何有效整合。

### 下載發布

**推薦：使用發布下載器 action**

```yaml
# GitHub Actions 範例
- name: Download winget-source-builder
  uses: robinraju/release-downloader@v1
  with:
    repository: SpecterShell/winget-source-builder
    tag: v1.0.0  # 固定到特定版本
    fileName: winget-source-builder-x86_64-pc-windows-msvc.zip
    extract: true
```

**PowerShell 範例：**

```powershell
$version = "1.0.0"
$url = "https://github.com/SpecterShell/winget-source-builder/releases/download/v$version/winget-source-builder-x86_64-pc-windows-msvc.zip"

Invoke-WebRequest -Uri $url -OutFile builder.zip
Expand-Archive -Path builder.zip -DestinationPath ./builder
```

### 固定版本

為了可重現的建置，始終固定到特定版本：

```yaml
# 好的做法：固定版本
- uses: robinraju/release-downloader@v1
  with:
    tag: v1.2.3

# 避免：最新版本（可能意外中斷）
- uses: robinraju/release-downloader@v1
  with:
    latest: true
```

### 快取

為了加速 CI，快取下載的二進位檔：

```yaml
- name: Cache winget-source-builder
  uses: actions/cache@v3
  with:
    path: ./builder
    key: builder-${{ runner.os }}-${{ env.BUILDER_VERSION }}

- name: Download if not cached
  if: steps.cache.outputs.cache-hit != 'true'
  uses: robinraju/release-downloader@v1
  with:
    tag: ${{ env.BUILDER_VERSION }}
    fileName: winget-source-builder-x86_64-pc-windows-msvc.zip
    extract: true
```

### CI 整合範例

```yaml
name: Build Source

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

      - name: Download builder
        uses: robinraju/release-downloader@v1
        with:
          repository: SpecterShell/winget-source-builder
          tag: v${{ env.BUILDER_VERSION }}
          fileName: winget-source-builder-x86_64-pc-windows-msvc.zip
          extract: true

      - name: Build source
        run: |
          ./winget-source-builder.exe build `
            --repo-dir ./manifests `
            --state-dir ./state `
            --index-version v2

      - name: Publish source
        run: |
          ./winget-source-builder.exe publish `
            --state-dir ./state `
            --out-dir ./publish `
            --packaging-assets-dir ./packaging

      - name: Upload artifacts
        uses: actions/upload-artifact@v3
        with:
          name: source
          path: ./publish/
```

### 狀態持久化

為了在 CI 執行間實現增量建置，持久化狀態目錄：

```yaml
- name: Restore state cache
  uses: actions/cache@v3
  with:
    path: ./state
    key: builder-state-${{ github.run_id }}
    restore-keys: |
      builder-state-

- name: Build (incremental)
  run: |
    ./winget-source-builder.exe build `
      --repo-dir ./manifests `
      --state-dir ./state `
      --index-version v2
```

## 版本控制

### 相容性保證

在主版本內：

- CLI 參數保持向後相容
- 輸出格式保持穩定
- 狀態資料庫移轉自動進行

破壞性變更（主版本升級）將在發布說明中記錄，並附移轉指南。

### 棄用策略

在移除功能前：

1. 功能在文件中標記為已棄用
2. CLI 輸出中新增警告
3. 至少一個帶警告的次版本
4. 在下個主版本中移除

### API 穩定性

該工具是 CLI 應用程式，不是程式庫。公開介面是：

- 命令列參數
- 結束碼
- JSON 輸出格式（使用 `--json` 時）
- 環境變數

內部 API 可能在不通知的情況下變更。
