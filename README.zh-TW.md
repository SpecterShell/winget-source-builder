# winget-source-builder

[简体中文](README.zh-CN.md) | [English](README.md)

[![CI 狀態](https://github.com/SpecterShell/winget-source-builder/actions/workflows/ci.yml/badge.svg)](https://github.com/SpecterShell/winget-source-builder/actions/workflows/ci.yml)
[![授權條款](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![最新版本](https://img.shields.io/github/v/release/SpecterShell/winget-source-builder)](https://github.com/SpecterShell/winget-source-builder/releases)

> 從你的清單儲存庫建置 WinGet 相容的來源索引 — 快速、增量、開箱即可發佈。

`winget-source-builder` 幫助你架設私有的 WinGet 軟體套件倉庫。如果你維護了一套軟體清單集合，希望使用者可以透過 `winget source add` 指令從你的來源安裝軟體，本工具可以幫你產生所需的索引與安裝套件。

**解決的痛點：** 建立合法的 WinGet 來源需要完成複雜的索引生成、雜湊計算和 MSIX 打包流程，手動操作不僅容易出錯，而且效率極低。本工具實現了全流程自動化，開箱即用。

**工作原理：** 建置工具會掃描你的 YAML 清單，透過 SHA256 雜湊追蹤檔案變更，維護增量狀態資料庫。每次執行僅處理發生變更的內容，後續建置幾乎可以瞬間完成。最終輸出可直接部署的 MSIX 套件（`source.msix` 或 `source2.msix`）以及託管清單檔案。

**適用族群：** 軟體套件倉庫維護者、軟體發佈廠商，以及需要替代微軟社群倉庫、架設私有或公共 WinGet 來源的企業組織。

## 安裝方式

### 下載預建置二進位檔

從 [GitHub 發佈頁面](https://github.com/SpecterShell/winget-source-builder/releases) 獲取對應平台的最新版本。

**Windows：** 解壓縮 zip 檔後，將 `winget-source-builder.exe` 和 `WinGetUtil.dll` 放到 PATH 目錄下，或直接在當前目錄使用。

**Linux/macOS：** 解壓縮封存檔，打包功能需要依賴 `makemsix` 工具（建置方法請參考 [開發指南](docs/en/development.md)）。

### 從原始碼建置

```powershell
git clone https://github.com/SpecterShell/winget-source-builder.git
cd winget-source-builder
git -c submodule.recurse=false submodule update --init winget-cli msix-packaging
cargo build --release
```

詳細的環境設定說明請檢視 [開發指南](docs/en/development.md)。

## 快速開始

工作流程分為兩個階段：**建置**（準備內容）和 **發佈**（打包輸出）。

```powershell
# 步驟 1：建置來源索引
winget-source-builder build `
  --repo-dir ./manifests `
  --state-dir ./state `
  --index-version v2

# 步驟 2：發佈 MSIX 套件
winget-source-builder publish `
  --state-dir ./state `
  --out-dir ./publish `
  --packaging-assets-dir ./packaging
```

指令執行完成後，你將得到以下輸出：

- `publish/source2.msix` — 主來源安裝套件（v2 索引格式）
- `publish/manifests/` — 託管的合併清單檔案
- `publish/packages/` — 版本資料附屬檔案（僅 v2 格式包含）

使用者即可透過以下指令新增你的軟體來源：

```powershell
winget source add --name mysource --argument https://your-domain.com/source2.msix
```

## 常見工作流程

### 首次使用設定

剛接觸 WinGet 來源架設？可以先閱讀 [使用指南](docs/en/usage.md)，獲取完整的操作教學，包含：

- 清單儲存庫的目錄結構設定
- 打包資源建立（AppxManifest.xml、圖示）
- 首次建置的完整流程

### 日常操作（新增/更新套件）

在儲存庫中新增或更新清單後，執行以下操作：

```powershell
# 直接執行建置指令，將自動偵測變更並增量更新
winget-source-builder build `
  --repo-dir ./manifests `
  --state-dir ./state `
  --index-version v2

# 檢視具體變更內容
winget-source-builder diff --repo-dir ./manifests --state-dir ./state
```

建置工具會比對檔案雜湊與上一次的狀態，僅重新處理發生變更的版本。

### 發佈版本

準備好發佈新版本時執行：

```powershell
# 基礎發佈（無簽章）
winget-source-builder publish `
  --state-dir ./state `
  --out-dir ./publish `
  --packaging-assets-dir ./packaging

# 帶程式碼簽章的發佈（Windows 平台）
winget-source-builder publish `
  --state-dir ./state `
  --out-dir ./publish `
  --packaging-assets-dir ./packaging `
  --sign-pfx-file ./cert.pfx `
  --sign-password-env CERT_PASSWORD
```

### 檢視變更內容

建置前可以檢視與上一次發佈狀態的差異：

```powershell
winget-source-builder diff --repo-dir ./manifests --state-dir ./state
```

或取得完整的狀態報告：

```powershell
winget-source-builder status --repo-dir ./manifests --state-dir ./state
```

## 更多文件

- **[使用指南](docs/en/usage.md)** — 分步教學，涵蓋首次建置、增量建置原理和常見任務處理
- **[CLI 參考手冊](docs/en/cli-reference.md)** — 完整的指令文件與範例
- **[架構設計](docs/en/architecture.md)** — 底層工作原理：雜湊模型、狀態管理和建置流水線
- **[開發指南](docs/en/development.md)** — 原始碼建置、測試執行和貢獻指南
- **[貢獻說明](docs/en/contributing.md)** — CI/CD 工作流程和發佈程序說明

## 授權條款

採用 MIT 授權條款，詳情請檢視 [LICENSE](LICENSE) 檔案。
