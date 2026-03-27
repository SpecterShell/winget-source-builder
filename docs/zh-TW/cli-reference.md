# CLI 參考

`winget-source-builder` 的完整命令、選項和結束碼參考。

## 目錄

- [全域選項](#全域選項)
- [命令群組](#命令群組)
  - [核心工作流程](#核心工作流程)
  - [倉庫管理](#倉庫管理)
  - [檢查與偵錯](#檢查與偵錯)
  - [維護](#維護)
- [結束碼](#結束碼)

## 全域選項

這些選項適用於大多數命令：

| 選項 | 說明 |
|------|------|
| `--lang <地區設定>` | 覆寫顯示語言（例如 `en`、`zh-CN`、`zh-TW`） |
| `--dry-run` | 顯示將要執行的操作而不實際變更 |
| `--force` | 覆寫現有資料，忽略安全檢查 |
| `--json` | 輸出機器可讀的 JSON（用於報告類命令） |

### 索引版本選擇

許多命令接受 `--index-version` 來選擇來源格式：

- `--index-version v1` — 舊版格式（`source.msix`）
- `--index-version v2` — 現代格式（`source2.msix`，推薦）

### 顯示版本衝突策略

對於修改狀態的命令，你可以控制如何處理 ARP 版本衝突：

| 策略 | 行為 |
|------|------|
| `latest` | 保留最新版本（預設） |
| `oldest` | 保留最舊版本 |
| `strip-all` | 移除所有衝突的顯示版本 |
| `error` | 如果偵測到衝突則失敗 |

## 命令群組

### 核心工作流程

日常使用的命令：`build` 和 `publish`。

#### `build`

**用途：** 掃描倉庫、識別變更、更新狀態並暫存可發佈的建置。

**何時使用：** 在新增、更新或移除清單後執行。這是建置 → 發佈工作流程的第一步。

```powershell
winget-source-builder build `
  --repo-dir <目錄> `
  --state-dir <目錄> `
  [--backend <wingetutil|rust>] `
  [--index-version <v1|v2>] `
  [--package-id <識別碼>...] `
  [--version-dir <目錄>...] `
  [--force] `
  [--dry-run] `
  [--no-validation-queue] `
  [--display-version-conflict-strategy <latest|oldest|strip-all|error>]
```

**選項：**

| 選項 | 說明 |
|------|------|
| `--repo-dir` | **必要。** 清單倉庫路徑 |
| `--state-dir` | **必要。** 狀態目錄路徑 |
| `--backend` | 索引操作後端：`wingetutil`（僅 Windows）或 `rust`（預設） |
| `--index-version` | 來源格式版本：`v1` 或 `v2`（預設：`v2`） |
| `--package-id` | 限制建置到特定套件，可重複 |
| `--version-dir` | 限制建置到特定版本目錄，可重複 |
| `--force` | 忽略現有狀態並重建所有內容 |
| `--dry-run` | 顯示會變什麼而不更新狀態 |
| `--no-validation-queue` | 跳過產生 validation-queue.json |
| `--display-version-conflict-strategy` | 如何處理 ARP 版本衝突 |

**範例：**

```powershell
# 完整倉庫建置
winget-source-builder build --repo-dir ./manifests --state-dir ./state

# 使用 rust 後端和 v2 格式的增量建置
winget-source-builder build `
  --repo-dir ./manifests `
  --state-dir ./state `
  --backend rust `
  --index-version v2

# 僅建置特定套件
winget-source-builder build `
  --repo-dir ./manifests `
  --state-dir ./state `
  --package-id Vendor.App1 `
  --package-id Vendor.App2

# 強制重建所有內容
winget-source-builder build `
  --repo-dir ./manifests `
  --state-dir ./state `
  --force
```

#### `publish`

**用途：** 將暫存的建置封裝成 MSIX 並寫入最終發佈樹。

**何時使用：** 在 `build` 後執行，當你準備好部署時。這會建立可分發的檔案。

```powershell
winget-source-builder publish `
  --state-dir <目錄> `
  --out-dir <目錄> `
  --packaging-assets-dir <目錄> `
  [--build-id <id>] `
  [--force] `
  [--dry-run] `
  [--sign-pfx-file <檔案>] `
  [--sign-password <值>] `
  [--sign-password-env <環境變數>] `
  [--timestamp-url <url>]
```

**選項：**

| 選項 | 說明 |
|------|------|
| `--state-dir` | **必要。** 狀態目錄路徑 |
| `--out-dir` | **必要。** 最終輸出路徑 |
| `--packaging-assets-dir` | **必要。** 包含 `AppxManifest.xml` 和 `Assets/` 的目錄 |
| `--build-id` | 發佈特定建置而非最新的暫存建置 |
| `--force` | 即使輸出目錄與追蹤狀態不同也覆寫 |
| `--dry-run` | 顯示會寫入什麼而不建立檔案 |
| `--sign-pfx-file` | 程式碼簽章 PFX 憑證路徑 |
| `--sign-password` | PFX 檔案密碼（為了安全請使用 `--sign-password-env`） |
| `--sign-password-env` | 包含 PFX 密碼的環境變數 |
| `--timestamp-url` | 時間戳伺服器 URL（僅 Windows） |

**範例：**

```powershell
# 基礎發佈（未簽章）
winget-source-builder publish `
  --state-dir ./state `
  --out-dir ./publish `
  --packaging-assets-dir ./packaging

# 帶程式碼簽章的發佈
winget-source-builder publish `
  --state-dir ./state `
  --out-dir ./publish `
  --packaging-assets-dir ./packaging `
  --sign-pfx-file ./cert.pfx `
  --sign-password-env CERT_PASSWORD

# 發佈特定歷史建置
winget-source-builder publish `
  --state-dir ./state `
  --out-dir ./publish `
  --packaging-assets-dir ./packaging `
  --build-id 42
```

---

### 倉庫管理

用於修改狀態而不完全重建的命令。

#### `add`

**用途：** 將特定版本增量新增到工作狀態。

**何時使用：** 當你想新增單個版本而不掃描整個倉庫時。對於針對性更新比 `build` 更快。

```powershell
winget-source-builder add `
  --repo-dir <目錄> `
  --state-dir <目錄> `
  (--version-dir <目錄>... | --manifest-file <檔案>... | --package-id <識別碼> --version <版本>) `
  [--backend <wingetutil|rust>] `
  [--index-version <v1|v2>] `
  [--force] `
  [--dry-run] `
  [--no-validation-queue] `
  [--display-version-conflict-strategy <latest|oldest|strip-all|error>]
```

**範例：**

```powershell
# 按版本目錄新增
winget-source-builder add `
  --repo-dir ./manifests `
  --state-dir ./state `
  --version-dir ./manifests/v/Vendor/App/1.2.3

# 按套件識別碼和版本新增
winget-source-builder add `
  --repo-dir ./manifests `
  --state-dir ./state `
  --package-id Vendor.App `
  --version 1.2.3

# 新增單個清單檔案
winget-source-builder add `
  --repo-dir ./manifests `
  --state-dir ./state `
  --manifest-file ./manifests/v/Vendor/App/1.2.3/Vendor.App.yaml
```

#### `remove` / `delete`

**用途：** 從工作狀態中增量移除特定版本。

**何時使用：** 當你需要移除版本而不重建所有內容時。`delete` 是 `remove` 的完全別名。

```powershell
winget-source-builder remove `
  --repo-dir <目錄> `
  --state-dir <目錄> `
  (--version-dir <目錄>... | --manifest-file <檔案>... | --package-id <識別碼> --version <版本>) `
  [--backend <wingetutil|rust>] `
  [--index-version <v1|v2>] `
  [--force] `
  [--dry-run] `
  [--no-validation-queue] `
  [--display-version-conflict-strategy <latest|oldest|strip-all|error>]
```

**範例：**

```powershell
# 按套件識別碼和版本移除
winget-source-builder remove `
  --repo-dir ./manifests `
  --state-dir ./state `
  --package-id Vendor.App `
  --version 1.0.0

# 按版本目錄移除
winget-source-builder remove `
  --repo-dir ./manifests `
  --state-dir ./state `
  --version-dir ./manifests/v/Vendor/App/1.0.0
```

#### `diff`

**用途：** 比較目前倉庫內容與工作狀態。

**何時使用：** 在執行 `build` 前查看發生了什麼變化。在 CI 中有助於決定是否需要建置。

```powershell
winget-source-builder diff `
  --repo-dir <目錄> `
  --state-dir <目錄> `
  [--package-id <識別碼>...] `
  [--version-dir <目錄>...] `
  [--json]
```

**範例：**

```powershell
# 人類可讀的差異
winget-source-builder diff --repo-dir ./manifests --state-dir ./state

# 用於 CI 的機器可讀差異
winget-source-builder diff `
  --repo-dir ./manifests `
  --state-dir ./state `
  --json > changes.json

# 僅差異特定套件
winget-source-builder diff `
  --repo-dir ./manifests `
  --state-dir ./state `
  --package-id Vendor.App
```

#### `status`

**用途：** 顯示目前狀態摘要、建置指標和可選的差異資訊。

**何時使用：** 快速取得倉庫狀態概覽而不進行完整差異比較。

```powershell
winget-source-builder status `
  --state-dir <目錄> `
  [--repo-dir <目錄>] `
  [--json]
```

**範例：**

```powershell
# 快速狀態概覽
winget-source-builder status --state-dir ./state

# 在狀態中包含待處理變更
winget-source-builder status `
  --state-dir ./state `
  --repo-dir ./manifests

# 用於指令碼輸出的 JSON
winget-source-builder status --state-dir ./state --json
```

---

### 檢查與偵錯

用於檢視資料和驗證一致性的命令。

#### `list-builds`

**用途：** 從狀態資料庫顯示最近的建置記錄。

```powershell
winget-source-builder list-builds `
  --state-dir <目錄> `
  [--limit <n>] `
  [--status <running|staged|published|failed>] `
  [--json]
```

| 選項 | 說明 |
|------|------|
| `--limit` | 顯示的最大建置數量（預設：20） |
| `--status` | 按建置狀態篩選 |

**範例：**

```powershell
# 顯示最近 10 個建置
winget-source-builder list-builds --state-dir ./state --limit 10

# 僅顯示已發佈的建置
winget-source-builder list-builds `
  --state-dir ./state `
  --status published
```

#### `show`

**用途：** 從狀態檢查建置、套件、版本或安裝程式雜湊。

```powershell
# 顯示建置詳情
winget-source-builder show build --state-dir <目錄> <建置-id> [--json]

# 顯示套件詳情
winget-source-builder show package --state-dir <目錄> <套件-識別碼> [--json]

# 顯示版本詳情
winget-source-builder show version `
  --state-dir <目錄> `
  (--version-dir <目錄> | --package-id <識別碼> --version <版本>) `
  [--json]

# 顯示安裝程式詳情
winget-source-builder show installer --state-dir <目錄> <安裝程式-雜湊> [--json]
```

**範例：**

```powershell
# 顯示套件資訊
winget-source-builder show package --state-dir ./state Vendor.App

# 將版本顯示為 JSON
winget-source-builder show version `
  --state-dir ./state `
  --package-id Vendor.App `
  --version 1.2.3 `
  --json

# 顯示建置詳情
winget-source-builder show build --state-dir ./state 42
```

#### `verify`

**用途：** 對照追蹤狀態檢查暫存或發佈的輸出。

**何時使用：** 在部署前或部署後確保輸出完整性。

```powershell
# 驗證暫存建置
winget-source-builder verify staged `
  --state-dir <目錄> `
  [--build-id <id>] `
  [--json]

# 驗證發佈輸出
winget-source-builder verify published `
  --state-dir <目錄> `
  --out-dir <目錄> `
  [--json]
```

**範例：**

```powershell
# 驗證暫存建置
winget-source-builder verify staged --state-dir ./state

# 驗證特定發佈輸出
winget-source-builder verify published `
  --state-dir ./state `
  --out-dir ./publish
```

#### `hash`

**用途：** 列印倉庫目標的內容雜湊和每安裝程式雜湊。

**何時使用：** 偵錯雜湊不符或驗證清單內容。

```powershell
winget-source-builder hash `
  --repo-dir <目錄> `
  (--version-dir <目錄> | --package-id <識別碼> --version <版本>) `
  [--json]
```

**範例：**

```powershell
# 顯示版本的雜湊
winget-source-builder hash `
  --repo-dir ./manifests `
  --package-id Vendor.App `
  --version 1.2.3

# 以 JSON 輸出
winget-source-builder hash `
  --repo-dir ./manifests `
  --version-dir ./manifests/v/Vendor/App/1.2.3 `
  --json
```

#### `merge`

**用途：** 以規範形式列印倉庫目標的合併清單。

**何時使用：** 偵錯多檔案清單合併或檢視最終合併輸出。

```powershell
winget-source-builder merge `
  --repo-dir <目錄> `
  (--version-dir <目錄> | --package-id <識別碼> --version <版本>) `
  [--output-file <檔案>] `
  [--json]
```

**範例：**

```powershell
# 將合併清單列印到 stdout
winget-source-builder merge `
  --repo-dir ./manifests `
  --package-id Vendor.App `
  --version 1.2.3

# 儲存到檔案
winget-source-builder merge `
  --repo-dir ./manifests `
  --package-id Vendor.App `
  --version 1.2.3 `
  --output-file ./merged.yaml
```

---

### 維護

用於清理和診斷的命令。

#### `clean`

**用途：** 移除衍生資料以釋放空間或重置狀態。

**何時使用：** 定期回收磁碟空間，或排解狀態問題時。

```powershell
winget-source-builder clean `
  --state-dir <目錄> `
  [--staging] `
  [--builds] `
  [--validation-queue] `
  [--published-tracking] `
  [--backend-cache] `
  [--all] `
  [--keep-last <n>] `
  [--older-than <時長>] `
  [--dry-run] `
  [--force]
```

| 選項 | 說明 |
|------|------|
| `--staging` | 清理暫存建置目錄 |
| `--builds` | 清理建置歷史 |
| `--validation-queue` | 清理驗證佇列檔案 |
| `--published-tracking` | 清理已發佈建置追蹤 |
| `--backend-cache` | 清理後端特定快取 |
| `--all` | 選擇所有可清理的資料（除工作狀態外） |
| `--keep-last` | 清理建置時保留 N 個最近的項目 |
| `--older-than` | 僅移除早於指定時長的項目（例如 `7d`、`24h`） |

**範例：**

```powershell
# 清理舊暫存目錄
winget-source-builder clean --state-dir ./state --staging

# 僅保留最後 5 個建置
winget-source-builder clean `
  --state-dir ./state `
  --builds `
  --keep-last 5

# 清理除工作狀態外的所有內容
winget-source-builder clean --state-dir ./state --all

# 預覽將要清理的內容
winget-source-builder clean `
  --state-dir ./state `
  --all `
  --dry-run
```

#### `doctor`

**用途：** 檢查環境、封裝資源、後端/索引相容性和狀態健康。

**何時使用：** 排解問題的第一步，或作為 CI 中的預檢。

```powershell
winget-source-builder doctor `
  [--repo-dir <目錄>] `
  [--state-dir <目錄>] `
  [--packaging-assets-dir <目錄>] `
  [--backend <wingetutil|rust>] `
  [--index-version <v1|v2>] `
  [--json]
```

**範例：**

```powershell
# 基礎健康檢查
winget-source-builder doctor

# 帶路徑的完整檢查
winget-source-builder doctor `
  --repo-dir ./manifests `
  --state-dir ./state `
  --packaging-assets-dir ./packaging

# 用於 CI 的 JSON 輸出
winget-source-builder doctor --json > health-check.json
```

---

## 結束碼

| 代碼 | 含義 |
|------|------|
| `0` | 成功 |
| `1` | 一般錯誤 |
| `2` | 引數或用法無效 |
| `3` | 未找到倉庫或狀態 |
| `4` | 後端不可用 |
| `5` | 驗證失敗 |
| `6` | 偵測到輸出目錄漂移（發佈時） |
| `7` | 簽章失敗 |
| `8` | 偵測到狀態損毀 |
