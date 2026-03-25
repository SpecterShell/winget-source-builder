# winget-source-builder

[English](README.md) | [简体中文](README.zh-CN.md)

`winget-source-builder` 是一個面向第三方倉庫的靜態 WinGet 來源建置工具。它依據檔案狀態而不是 Git commit 追蹤變更，維護可增量更新的內部狀態庫，並輸出包含 `source.msix` 或 `source2.msix`、所需 sidecar 檔案以及託管合併清單的發佈目錄。

面向使用者的訊息現在透過 `locales/` 下的外部語系檔管理。新增語系不需要再修改 Rust 原始碼。

## 功能

- 以 SQLite 狀態庫為基礎的檔案狀態增量建置。
- 使用 Rust 平行執行掃描、雜湊、合併與差異計算。
- 內容定址的託管清單路徑與 `versionData.mszyml`。
- 相容 WinGet 的 `source.msix` 與 `source2.msix` 輸出。
- 核心已保留格式抽象，未來可透過新增 writer 支援新的 catalog 版本。

## 需求

- 完整 WinGetUtil 路徑以及 `v2` sidecar 產生需要 Windows 10/11。
- 執行時需要 `winget-source-builder.exe` 同目錄下的 `WinGetUtil.dll`。Windows 建置會預設從倉庫內建的 `winget-cli` 子模組自動產生它。
- 需要 Windows SDK 的 `makeappx.exe` 或 `makemsix`。非 Windows 建置會預設從倉庫內建的 `msix-packaging` 子模組建出 `makemsix`。
- 從原始碼倉庫執行時需要 Rust stable，並執行 `git submodule update --init --recursive`。
- 被索引的來源倉庫需要包含 `packaging/`，例如 `winget-source-template` 提供的模板結構。

## 快速開始

從原始碼倉庫建置：

```powershell
git submodule update --init --recursive
cargo run -- build `
  --repo C:\path\to\source-repo\manifests `
  --state C:\path\to\builder-state `
  --out C:\path\to\publish-root `
  --lang zh-TW `
  --backend rust `
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

- `--format v1` 時輸出 `source.msix`，`--format v2` 時輸出 `source2.msix`
- `packages/<PackageIdentifier>/<hash8>/versionData.mszyml` 只會在 `--format v2` 產生
- `manifests/...`

狀態目錄：

- `state.sqlite`
- `validation-queue.json`
- 使用 WinGetUtil backend 時會有 `writer/mutable-v1.db` 或 `writer/mutable-v2.db`

建議參考 `winget-source-template` 的 workflow 模式：使用 `robinraju/release-downloader` 從 GitHub Releases 下載預先建好的 builder，並在模板倉庫自己的 workflow 中直接執行。

## 文件

- [使用說明](docs/zh-TW/usage.md)
- [架構說明](docs/zh-TW/architecture.md)
- [開發與 CI](docs/zh-TW/development.md)
