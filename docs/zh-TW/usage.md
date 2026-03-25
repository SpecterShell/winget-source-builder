# 使用說明

## 前置需求

- 完整建置流程需要 Windows 10/11。
- 執行時需要 `winget-source-builder.exe` 同目錄下的 `WinGetUtil.dll`。Windows 建置會預設從倉庫內建的 `winget-cli` 子模組自動產生它。
- 需要 Windows SDK 的 `makeappx.exe`，或設定 `MAKEAPPX_EXE`。
- 需要一個依 WinGet manifest 結構組織的 manifest 倉庫。
- 來源倉庫根目錄還需要包含 `packaging/`。
- 從原始碼倉庫執行時需要 Rust stable，並執行 `git submodule update --init --recursive`。只有在想覆寫內建 `winget-cli` 子模組時，才需要設定 `WINGET_CLI_ROOT`。

## 指令

建置靜態來源目錄：

```powershell
cargo run -- build `
  --repo C:\path\to\repo `
  --state C:\path\to\state `
  --out C:\path\to\out `
  --lang zh-TW `
  --format v2
```

從封裝產物執行：

```powershell
.\winget-source-builder.exe build `
  --repo C:\path\to\repo `
  --state C:\path\to\state `
  --out C:\path\to\out `
  --lang zh-TW `
  --format v2
```

## 環境變數

- `WINGET_CLI_ROOT`：`winget-cli` 原始碼倉庫的絕對路徑，用於在編譯時引導 `WinGetUtil.dll`。
- `MAKEAPPX_EXE`：`makeappx.exe` 的絕對路徑。
- `WINGET_SOURCE_BUILDER_WORKSPACE_ROOT`：覆寫預設工作區根目錄，用來定位 `packaging/`。如果 `--repo` 已經指向來源模板倉庫內的目錄，通常無需手動設定。
- `WINGET_SOURCE_BUILDER_LANG`：建置進度與摘要輸出的執行期語言。只要 `locales/` 下存在對應語系檔，就可以使用，例如 `en` 或 `zh-CN`。

## 輸出目錄

- `source2.msix`：提供 WinGet v2 用戶端使用的 catalog 套件。
- `packages/<PackageIdentifier>/<hash8>/versionData.mszyml`：套件層級的 sidecar 資料。
- `manifests/...`：catalog 參照的內容定址合併 manifest。

## 狀態目錄

- `state.sqlite`：增量狀態庫。
- `validation-queue.json`：安裝程式重新驗證工作項目。
- `writer/mutable-v2.db`：持久化的 WinGetUtil 可變資料庫。
- `staging/`：每次建置的暫存工作目錄。

## 增量行為

- 透過檔案新增、刪除與內容雜湊來偵測變更。
- 只有中繼資料變更時，會重新發佈受影響套件，但不會強制重新做安裝程式驗證。
- 影響安裝程式的變更會寫入 `validation-queue.json`。
- 如果 `--out` 下的前一次發佈樹遺失，更新與刪除操作就無法重用舊的託管 manifest，需要走新的全量建置流程。
