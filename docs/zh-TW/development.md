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

建置流程不再接受 DLL 路徑覆寫，也不再掃描兄弟目錄中的舊 `WinGetUtil.dll` 輸出，更不相容歷史遺留的執行期搜尋路徑。乾淨工作區應依賴內建子模組或顯式設定 `WINGET_CLI_ROOT`。

## 測試覆蓋

- Rust 單元測試涵蓋多檔 manifest 合併與安裝程式雜湊過濾。
- Windows 端對端測試會建置 `tests/data/e2e-repo` 內的示例倉庫。
- 如果機器上沒有 `WinGetUtil.dll` 或 `makeappx.exe`，端對端測試會自動跳過。
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
  - 在 Windows 上檢出子模組，讓 `build.rs` 自動準備 `WinGetUtil.dll`
  - 產生一個 Windows x64 workflow artifact
- `release.yml`
  - 在 `v*` tag 上觸發
  - 以 release 模式建置 Rust CLI
  - 在編譯過程中由 `build.rs` 準備 `WinGetUtil.dll`
  - 封裝 Windows x64 發佈 zip，並上傳到 GitHub Release
- `action.yml`
  - 將本倉庫暴露為可重用的 GitHub Action
  - 檢出 action 原始碼與子模組
  - 在 Windows runner 上建置 Rust CLI，並對來源模板倉庫執行建置

## 發佈封裝內容

Windows 發佈 zip 內包含：

- `winget-source-builder.exe`
- `WinGetUtil.dll`
- `action.yml`
- `LICENSE`
- `AGENTS.md`
- 三種語言的 README
- `docs/` 目錄

Rust 主程式需要從被索引的來源模板倉庫讀取 `packaging/msix/`，而不是從 builder 發佈封裝讀取。
