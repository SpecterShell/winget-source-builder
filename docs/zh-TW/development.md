# 開發與 CI

## 本機開發

建議的本機檢查指令：

```powershell
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
cargo test --verbose
```

在 Windows 上，`build.rs` 會在編譯時把 `WinGetUtil.dll` 放到產生的可執行檔旁邊。它會依序嘗試：

- `WINGET_CLI_ROOT` 或倉庫內建的 `winget-cli` 子模組，再呼叫 `scripts/build-wingetutil.ps1`

在 Linux 和 macOS 上，`build.rs` 會在編譯時把 `makemsix` 放到產生的可執行檔旁邊。它會依序嘗試：

- `MSIX_PACKAGING_ROOT` 或倉庫內建的 `msix-packaging` 子模組，再呼叫 `scripts/build-makemsix.sh`

建置流程不再接受 DLL 路徑覆寫，也不再掃描兄弟目錄中的舊 `WinGetUtil.dll` 輸出，更不相容歷史遺留的執行期搜尋路徑。乾淨工作區應依賴內建子模組或顯式設定 `WINGET_CLI_ROOT`。

## 測試覆蓋

- Rust 單元測試涵蓋多檔 manifest 合併與安裝程式雜湊過濾。
- Windows 端對端測試會建置 `tests/data/e2e-repo` 內的示例倉庫。
- 當 `makemsix` 可用時，Rust `v1` 端對端測試也可以在 Linux 和 macOS 上執行。
- 如果缺少對應 backend 所需的執行期或封裝依賴，端對端測試會自動跳過。
- i18n 執行期測試涵蓋 locale 正規化、回退行為，以及從 `locales/` 載入翻譯。

## 在地化

- CLI 面向使用者的訊息由 `rust-i18n` 提供。
- 翻譯字串放在 `locales/` 中，而不是硬編碼在 Rust 原始碼裡。
- 新增一種語系通常只需要新增語系檔，除非程式新增了新的訊息鍵。

## GitHub Actions

倉庫內建兩個 workflow：

- `ci.yml`
  - 執行 `cargo fmt --all --check`
  - 執行 `cargo clippy --all-targets --all-features -- -D warnings`
  - 在 Linux、macOS、Windows 上執行 `cargo test --verbose`
  - 在 test/build 作業中檢出子模組，讓 `build.rs` 自動準備 `WinGetUtil.dll` 和 `makemsix`
  - 產生一個 Windows x64 workflow artifact
- `release.yml`
  - 在 `v*` tag 上觸發
  - 以 release 模式建置 Rust CLI
  - 在編譯過程中由 `build.rs` 準備 `WinGetUtil.dll`
  - 封裝 Windows x64 發佈 zip，並上傳到 GitHub Release

下游倉庫應在自己的 workflow 中直接下載本倉庫發佈的 Windows release 產物，例如使用 `robinraju/release-downloader`，而不是再依賴本倉庫提供的可重用 Action。

## 發佈封裝內容

Windows 發佈 zip 內包含：

- `winget-source-builder.exe`
- `WinGetUtil.dll`

Rust 主程式需要從被索引的來源模板倉庫讀取 `packaging/`，而不是從 builder 發佈封裝讀取。
