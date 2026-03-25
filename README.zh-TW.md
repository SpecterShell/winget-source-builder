# winget-source-builder

[English](README.md) | [简体中文](README.zh-CN.md)

`winget-source-builder` 是一個以 Windows 為主的靜態 WinGet 來源建置工具，目標是讓第三方倉庫也能產生可靜態託管的 WinGet 來源。它依據檔案狀態而不是 Git commit 追蹤變更，維護可增量更新的內部狀態庫，並輸出包含 `source2.msix`、套件 sidecar 與託管合併清單的發佈目錄。

面向使用者的訊息現在透過 `locales/` 下的外部語系檔管理。新增語系不需要再修改 Rust 原始碼。

## 功能

- 以 SQLite 狀態庫為基礎的檔案狀態增量建置。
- 使用 Rust 平行執行掃描、雜湊、合併與差異計算。
- 內容定址的託管清單路徑與 `versionData.mszyml`。
- 相容 WinGet 的 `source2.msix` 輸出。
- 核心已保留格式抽象，未來可透過新增 writer 支援新的 catalog 版本。

## 需求

- 完整建置流程需要 Windows 10/11。
- 執行時需要 `winget-source-builder.exe` 同目錄下的 `WinGetUtil.dll`。Windows 建置會預設從倉庫內建的 `winget-cli` 子模組自動產生它。
- 需要 Windows SDK 的 `makeappx.exe`，或使用 `MAKEAPPX_EXE` 指向它。
- 從原始碼倉庫執行時需要 Rust stable，並執行 `git submodule update --init --recursive`。
- 被索引的來源倉庫需要包含 `packaging/msix/`，例如 `winget-source-template` 提供的模板結構。

## 快速開始

從原始碼倉庫建置：

```powershell
git submodule update --init --recursive
cargo run -- build `
  --repo C:\path\to\source-repo\manifests `
  --state C:\path\to\builder-state `
  --out C:\path\to\publish-root `
  --lang zh-TW `
  --format v2
```

從已封裝的 Windows 產物執行：

```powershell
.\winget-source-builder.exe build `
  --repo C:\path\to\source-repo\manifests `
  --state C:\path\to\builder-state `
  --out C:\path\to\publish-root `
  --lang zh-TW `
  --format v2
```

輸出目錄：

- `source2.msix`
- `packages/<PackageIdentifier>/<hash8>/versionData.mszyml`
- `manifests/...`

狀態目錄：

- `state.sqlite`
- `validation-queue.json`
- `writer/mutable-v2.db`

## GitHub Action

本倉庫同時提供一個可重用的 GitHub Action，入口位於 [action.yml](https://github.com/SpecterShell/winget-source-builder/blob/main/action.yml)。

建議的消費方是一個包含下列目錄的來源模板倉庫：

- `manifests/`
- `packaging/msix/`

目錄結構與 workflow 範例可參考 `winget-source-template`。

## 文件

- [使用說明](docs/zh-TW/usage.md)
- [架構說明](docs/zh-TW/architecture.md)
- [開發與 CI](docs/zh-TW/development.md)
