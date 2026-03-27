# 開發指南

本指南涵蓋從原始碼建置 `winget-source-builder`、執行測試和為專案貢獻程式碼。

## 目錄

- [前置條件](#前置條件)
- [從原始碼建置](#從原始碼建置)
- [專案結構](#專案結構)
- [執行測試](#執行測試)
- [在地化](#在地化)
- [貢獻程式碼](#貢獻程式碼)

## 前置條件

### 必要工具

- **Rust** — 最新穩定版本（推薦 1.70+）
  - 透過 [rustup](https://rustup.rs/) 安裝
- **Git** — 用於複製和子模組管理

### 平台特定需求

**Windows：**

- PowerShell 7+（用於執行建置指令碼）
- Visual Studio 2022 建置工具或完整 Visual Studio
  - 有關如何設定 Visual Studio，請參考 [winget-cli 的指南](https://github.com/microsoft/winget-cli/blob/master/doc/Developing.md)。
- Windows SDK

**Linux：**

- GCC 或 Clang 工具鏈
- CMake (3.15+)
- OpenSSL 開發標頭檔（用於簽章支援）

**macOS：**

- Xcode 命令列工具
- CMake (3.15+)
- OpenSSL（透過 Homebrew：`brew install openssl`）

### 複製倉庫

```powershell
git clone https://github.com/SpecterShell/winget-source-builder.git
cd winget-source-builder

# 初始化子模組（WinGetUtil 和 makemsix 必需）
git -c submodule.recurse=false submodule update --init winget-cli msix-packaging
```

子模組提供：

- `winget-cli/` — 建置 `WinGetUtil.dll` 的原始碼（Windows）
- `msix-packaging/` — 建置 `makemsix` 的原始碼（Linux/macOS）

## 從原始碼建置

### 標準建置

```powershell
# 偵錯建置（編譯更快，執行較慢）
cargo build

# 發布建置（編譯較慢，最佳化執行）
cargo build --release
```

首次建置將：

1. 編譯 Rust 程式碼
2. 在 Windows 上：從 `winget-cli` 子模組建置 `WinGetUtil.dll`
3. 在 Linux/macOS 上：從 `msix-packaging` 子模組建置 `makemsix`

### 建置輸出

**Windows：**

- `target/debug/winget-source-builder.exe`
- `target/debug/WinGetUtil.dll`（從建置複製）

**Linux/macOS：**

- `target/debug/winget-source-builder`
- `target/debug/makemsix`（從子模組建置）

### 自訂 WinGetUtil 位置

如果你有單獨的 `winget-cli` 檢出：

```powershell
$env:WINGET_CLI_ROOT = "C:\path\to\winget-cli"
cargo build --release
```

### 自訂 makemsix 位置

對於 Linux/macOS，如果你有單獨的 `msix-packaging` 檢出：

```powershell
$env:MSIX_PACKAGING_ROOT = "/path/to/msix-packaging"
cargo build --release
```

### 使用 Mozilla 的簽章支援 makemsix

預設的 `msix-packaging` 在非 Windows 平台上不支援簽章。要在 Linux/macOS 上獲得簽章支援：

```powershell
git clone https://github.com/mozilla/msix-packaging.git $env:MSIX_PACKAGING_ROOT
cargo build --release
```

## 專案結構

```
winget-source-builder/
├── src/
│   ├── main.rs           # CLI 進入點、命令路由
│   ├── adapter.rs        # 後端抽象層
│   ├── backend.rs        # 後端實作
│   ├── builder.rs        # 核心建置編排
│   ├── i18n.rs           # 國際化設定
│   ├── manifest.rs       # 清單解析和合併
│   ├── mszip.rs          # ZIP 壓縮工具
│   ├── progress.rs       # 進度報告
│   ├── state.rs          # 狀態資料庫操作
│   └── version.rs        # 版本比較和規範化
├── locales/              # 翻譯檔案
│   ├── en.yml
│   ├── zh-CN.yml
│   └── zh-TW.yml
├── scripts/              # 建置輔助指令碼
│   ├── build-wingetutil.ps1
│   └── build-makemsix.sh
├── docs/                 # 文件
│   └── zh-TW/
│       ├── usage.md
│       ├── cli-reference.md
│       ├── architecture.md
│       └── development.md
├── winget-cli/           # Git 子模組（Windows 後端）
├── msix-packaging/       # Git 子模組（跨平台封裝）
└── Cargo.toml
```

### 關鍵模組

| 模組 | 用途 |
|------|------|
| `builder.rs` | 編排建置流程：掃描、雜湊、差異、合併、索引 |
| `state.rs` | 增量狀態的 SQLite 資料庫操作 |
| `manifest.rs` | YAML 解析、多檔案合併、規範化 |
| `adapter.rs` | `wingetutil` 和 `rust` 後端的抽象 |
| `backend.rs` | 索引操作的後端實作 |
| `progress.rs` | 長時間操作的進度報告 |

## 執行測試

### 快速測試檢查

提交變更前，執行這些命令：

```powershell
# 格式檢查
cargo fmt --all --check

# 程式碼檢查
cargo clippy --all-targets --all-features -- -D warnings

# 執行所有測試
cargo test --verbose
```

### 測試類別

**單元測試：**

```powershell
# 僅執行單元測試
cargo test --lib --verbose
```

單元測試涵蓋：

- 清單合併邏輯
- 雜湊計算和過濾
- 版本比較
- 狀態資料庫操作

**端對端測試：**

```powershell
# 執行所有測試包括 e2e（需要平台依賴）
cargo test --verbose
```

E2E 測試：

- 建置 `tests/data/e2e-repo/` 中的夾具倉庫
- 測試完整的建置 → 發佈流程
- 驗證輸出完整性
- 如果平台依賴缺失則自動跳過

**平台特定說明：**

- **Windows：** E2E 測試使用 `wingetutil` 和 `rust` 後端執行
- **Linux/macOS：** E2E 測試僅使用 `rust` 後端執行（需要 `makemsix`）

### 測試夾具

`tests/data/e2e-repo/` 目錄包含 E2E 測試使用的範例清單倉庫。包括：

- 多檔案清單
- 各種安裝程式類型
- 合併和驗證的邊界情況

新增功能時，請考慮向此夾具新增測試案例。

## 在地化

專案使用 `rust-i18n` 進行國際化。翻譯儲存在 `locales/` 下的 YAML 檔案中。

### 新增語言

1. 建立新檔案：`locales/<地區設定>.yml`
2. 從 `locales/en.yml` 複製結構
3. 翻譯所有值
4. 使用 `WINGET_SOURCE_BUILDER_LANG=<地區設定>` 測試

### 翻譯檔案結構

```yaml
# locales/en.yml
hello: Hello
build:
  scanning: Scanning repository...
  complete: Build complete
error:
  not_found: File not found
```

### 在程式碼中使用翻譯

```rust
use rust_i18n::t;

println!("{}", t!("build.scanning"));
```

### 測試在地化

```powershell
# 測試英文（預設）
winget-source-builder --lang en status --state-dir ./state

# 測試繁體中文
winget-source-builder --lang zh-TW status --state-dir ./state

# 或透過環境變數
$env:WINGET_SOURCE_BUILDER_LANG = "zh-TW"
winget-source-builder status --state-dir ./state
```

## 貢獻程式碼

### 入門

1. 在 GitHub 上 fork 倉庫
2. 本機複製你的 fork
3. 為你的功能或修正建立新分支
4. 進行變更
5. 執行測試清單（格式、程式碼檢查、測試）
6. 提交並附帶清晰的說明
7. 推送並發起 Pull Request

### 程式碼風格

- 遵循 `rustfmt` 慣例（`cargo fmt`）
- 解決所有 `clippy` 警告
- 為公開 API 編寫文件註解
- 為新功能新增測試

### 提交訊息

使用清晰、描述性的提交訊息：

```
Add support for custom validation queues

- Add --validation-queue-dir option
- Update state tracking for queue files
- Add tests for queue persistence
```

### Pull Request 流程

1. 確保所有 CI 檢查通過
2. 如果需要，更新文件
3. 連結任何相關問題
4. 請求維護者審查

### 報告問題

報告錯誤時，請包含：

- 作業系統和版本
- Rust 版本（`rustc --version`）
- 建置器版本或提交雜湊
- 復現步驟
- 預期與實際行為
- 完整錯誤輸出（如果有 `--verbose`）

### CI/CD

專案使用 GitHub Actions 進行 CI：

- **ci.yml** — 在每次推送和 PR 時執行
  - 格式檢查
  - Clippy 程式碼檢查
  - Linux、macOS、Windows 上的測試

- **release.yml** — 在版本標籤（`v*`）上執行
  - 發布建置
  - 資源封裝
  - GitHub Release 建立

有關 CI/CD 和發布流程的更多詳情，請參閱[貢獻指南](contributing.md)。
