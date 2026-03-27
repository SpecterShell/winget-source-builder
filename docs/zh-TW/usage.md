# 使用說明

本指南將帶你了解如何使用 `winget-source-builder` 建立和維護相容 WinGet 的套件來源。閱讀完本指南後，你將理解兩階段工作流程（建置 → 發佈）以及如何利用增量建置的優勢。

## 目錄

- [概述](#概述)
- [前置條件](#前置條件)
- [你的第一次建置](#你的第一次建置)
- [理解增量建置](#理解增量建置)
- [常見任務](#常見任務)
- [環境變數](#環境變數)
- [故障排除](#故障排除)

## 概述

`winget-source-builder` 分為兩個不同的階段：

1. **建置階段** —— 掃描你的清單倉庫，計算雜湊，識別變更，並準備暫存樹
2. **發佈階段** —— 將暫存樹封裝成 `source.msix`（v1）或 `source2.msix`（v2）並寫入最終輸出

這種分離讓你可以在發佈前驗證建置，並支援增量工作流程 —— 你可以頻繁建置但只在準備好時才發佈。

## 前置條件

開始前，你需要：

- **WinGet 風格的清單倉庫** —— 按套件識別碼和版本組織的 YAML 清單（例如 `manifests/v/Vendor/App/1.0.0/`）
- **可寫的狀態目錄** —— 建置器在此追蹤增量狀態、暫存建置和驗證佇列
- **封裝資源**（用於發佈） —— 包含 `AppxManifest.xml` 和 `Assets/` 目錄的 MSIX 封裝資源
- **平台需求：**
  - Windows：`WinGetUtil.dll`（隨版本附帶）
  - Linux/macOS：`makemsix` 用於 MSIX 封裝

包含範例清單和封裝資源的範本倉庫可在 [winget-source-template](https://github.com/SpecterShell/winget-source-template) 取得。

## 你的第一次建置

### 第一步：初始設定

建立你的狀態目錄。這是所有建置器狀態的存放位置：

```powershell
mkdir ./state
```

建置器會在這裡建立幾個檔案：

- `state.sqlite` —— 主狀態資料庫
- `validation-queue.json` —— 待處理的安裝程式驗證
- `staging/` —— 準備發佈的暫存建置

### 第二步：執行建置命令

`build` 命令分析你的清單倉庫並為發佈做準備：

```powershell
winget-source-builder build `
  --repo-dir ./manifests `
  --state-dir ./state `
  --backend rust `
  --index-version v2
```

以下是工作流程：

1. **掃描** —— 發現所有 YAML 清單並讀取
2. **雜湊計算** —— 計算檔案內容雜湊用於變更偵測
3. **合併** —— 將多檔案清單合併為規範形式
4. **差異比較** —— 與之前的狀態對比找出變更
5. **暫存** —— 在 `state/staging/` 中建立可發佈的樹

`--index-version v2` 旗標產生現代來源格式（`source2.msix`）。只有需要相容舊版 WinGet 用戶端時才使用 `v1`。

### 第三步：檢查結果

建置後，檢查狀態：

```powershell
winget-source-builder status --state-dir ./state
```

這會顯示：

- 你的工作狀態中有多少套件和版本
- 最新的暫存建置（準備發佈）
- 倉庫中偵測到的任何待處理變更

### 第四步：發佈

現在建立最終的 MSIX 套件：

```powershell
winget-source-builder publish `
  --state-dir ./state `
  --out-dir ./publish `
  --packaging-assets-dir ./packaging
```

輸出目錄現在包含：

- `source2.msix` —— 簽章（或未簽章）的 MSIX 套件
- `manifests/` —— 可直接下載的託管合併清單
- `packages/` —— 版本資料 sidecar 檔案（僅 v2 格式）

將這些檔案部署到你的 Web 伺服器，然後使用者就可以加入你的來源：

```powershell
winget source add --name mysource --argument https://your-domain.com/source2.msix
```

## 理解增量建置

這個工具的主要優勢之一是增量建置。以下是運作原理：

### 首次建置 vs 後續建置

**首次建置：**

- 掃描所有清單
- 為每個檔案計算雜湊
- 處理每個版本
- 耗時較長（大型倉庫可能需要數分鐘）

**後續建置：**

- 比較檔案雜湊與儲存的狀態
- 只處理變更的版本
- 通常在幾秒內完成

### 什麼會觸發完整重建

某些變更需要重新處理超出變更檔案本身的內容：

| 變更類型 | 需要重新處理的內容 |
|---------|------------------|
| 單個清單編輯 | 僅該版本 |
| 安裝程式 URL 變更 | 版本 + 驗證佇列項目 |
| 套件名稱變更 | 所有版本（套件級中繼資料） |
| 架構變更 | 可能需要 `--force` |

### 強制全新建置

如果你懷疑狀態損毀或想重新開始：

```powershell
winget-source-builder build `
  --repo-dir ./manifests `
  --state-dir ./state `
  --index-version v2 `
  --force
```

`--force` 旗標會忽略現有狀態並重建所有內容。

## 常見任務

### 新增新套件版本

1. 將清單檔案加入倉庫（例如 `manifests/v/Vendor/App/1.2.3/`）
2. 執行建置以取得變更
3. 準備好後發佈

如需快速測試單個版本而不掃描整個倉庫：

```powershell
winget-source-builder add `
  --repo-dir ./manifests `
  --state-dir ./state `
  --version-dir ./manifests/v/Vendor/App/1.2.3
```

### 移除套件

要移除特定版本：

```powershell
winget-source-builder remove `
  --repo-dir ./manifests `
  --state-dir ./state `
  --package-id Vendor.App `
  --version 1.0.0
```

要移除整個套件（所有版本），對每個版本重複執行或使用 `clean` 重置狀態。

### 檢查倉庫狀態

快速概覽：

```powershell
winget-source-builder status --state-dir ./state
```

查看詳細的待處理變更：

```powershell
winget-source-builder diff `
  --repo-dir ./manifests `
  --state-dir ./state
```

機器可讀輸出（在 CI 中有用）：

```powershell
winget-source-builder diff --json > changes.json
```

### 發佈前驗證

驗證暫存建置是否正確：

```powershell
winget-source-builder verify staged --state-dir ./state
```

驗證已發佈的輸出：

```powershell
winget-source-builder verify published `
  --state-dir ./state `
  --out-dir ./publish
```

### 程式碼簽章

**在 Windows 上**（使用 `signtool.exe`）：

```powershell
winget-source-builder publish `
  --state-dir ./state `
  --out-dir ./publish `
  --packaging-assets-dir ./packaging `
  --sign-pfx-file ./signing.pfx `
  --sign-password-env WINGET_SOURCE_SIGN_PASSWORD `
  --timestamp-url http://timestamp.digicert.com
```

**在 Linux/macOS 上**（使用 `makemsix` + `openssl`）：

```powershell
winget-source-builder publish `
  --state-dir ./state `
  --out-dir ./publish `
  --packaging-assets-dir ./packaging `
  --sign-pfx-file ./signing.pfx `
  --sign-password-env WINGET_SOURCE_SIGN_PASSWORD
```

注意：時間戳 URL 目前僅在 Windows 上支援。

### 檢視套件資訊

檢查狀態資料庫中的套件：

```powershell
# 顯示套件詳情
winget-source-builder show package --state-dir ./state Vendor.App

# 顯示特定版本
winget-source-builder show version `
  --state-dir ./state `
  --package-id Vendor.App `
  --version 1.2.3

# 輸出 JSON 用於指令碼
winget-source-builder show package Vendor.App --json
```

### 清理

隨著時間推移，狀態目錄會增長。清理舊建置：

```powershell
# 只保留最後 5 個建置
winget-source-builder clean `
  --state-dir ./state `
  --builds `
  --keep-last 5

# 移除舊的暫存目錄
winget-source-builder clean `
  --state-dir ./state `
  --staging

# 核彈選項：清理除工作狀態外的所有內容
winget-source-builder clean `
  --state-dir ./state `
  --all
```

### 診斷問題

執行診斷命令檢查環境：

```powershell
winget-source-builder doctor `
  --repo-dir ./manifests `
  --state-dir ./state `
  --packaging-assets-dir ./packaging
```

這會檢查：

- 必需工具是否可用
- 封裝資源是否有效
- 後端相容性
- 狀態資料庫健康

## 環境變數

以下環境變數可修改行為。當預設值不適合你的設定時使用它們。

| 變數 | 用途 |
|------|------|
| `WINGET_CLI_ROOT` | 指向 `winget-cli` 檢出路徑，用於建置自訂 `WinGetUtil.dll` |
| `MSIX_PACKAGING_ROOT` | 指向 `msix-packaging` 檢出路徑，用於自訂 `makemsix`（需要簽章支援時請指向 Mozilla 的分支） |
| `MAKEAPPX_EXE` | `makeappx.exe` 的明確路徑（Windows） |
| `MAKEMSIX_EXE` | `makemsix` 的明確路徑（Linux/macOS） |
| `OPENSSL` | `openssl` 二進位檔的明確路徑 |
| `WINGET_SOURCE_BUILDER_WORKSPACE_ROOT` | 封裝資源自動探索的後備工作區根目錄 |
| `WINGET_SOURCE_BUILDER_LANG` | CLI 訊息的執行期語言地區（例如 `zh-TW`） |

## 故障排除

### 建置失敗顯示「後端不可用」

**問題：** `wingetutil` 後端需要 Windows 和 `WinGetUtil.dll`。

**解決方案：** 在非 Windows 平台上使用 `--backend rust`，或確保 `WinGetUtil.dll` 在可執行檔旁邊。

### 發佈失敗顯示「輸出目錄漂移」

**問題：** 輸出目錄包含來自不同建置的檔案。

**解決方案：** 使用 `--force` 覆寫，或先清理輸出目錄。

### 增量建置未偵測到變更

**問題：** 檔案時間戳變更但內容未變更（或相反）。

**解決方案：** 建置器使用 SHA256 內容雜湊，而非時間戳。如果你未看到預期的變更，請驗證檔案是否確實不同。

### 建置後暫存建置遺失

**問題：** 某些東西刪除或損毀了暫存目錄。

**解決方案：** 執行 `build --force` 重新建立暫存樹。

### Linux/macOS 上 MSIX 簽章失敗

**問題：** 預設的 `makemsix` 不支援簽章。

**解決方案：** 將 `MSIX_PACKAGING_ROOT` 設定為 Mozilla 的簽章支援分支 `msix-packaging`，並確保安裝了 `openssl`。

### 清單合併錯誤

**問題：** 多檔案清單有衝突的欄位。

**解決方案：** 使用 `merge` 命令偵錯：

```powershell
winget-source-builder merge `
  --repo-dir ./manifests `
  --package-id Vendor.App `
  --version 1.2.3
```

這會顯示合併後的輸出而不影響狀態。
