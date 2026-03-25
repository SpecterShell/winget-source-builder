# 架構說明

## 整體設計

專案實作為一個 Rust CLI，並在同一個二進位內保留一層很薄的原生互操作邊界。

- Rust 負責掃描、雜湊、manifest 合併/解析、正規化、差異計算、狀態管理、發佈規劃、WinGetUtil 互操作，以及 `source2.msix` 封裝。
- `WinGetUtil.dll` 仍然是可變索引寫入的相容後端，但現在由 Rust 在執行期直接載入。
- MSIX 靜態資源放在來源模板倉庫的 `packaging/msix/` 下，而不是放在本建置器倉庫裡。

## 建置流程

1. 掃描倉庫並對變更的 YAML 檔案計算雜湊。
2. 只對髒的版本目錄重新產生合併 manifest 快照。
3. 計算：
   - `version_content_sha256`
   - `version_installer_sha256`
   - `published_manifest_sha256`
4. 將髒版本與上一次成功狀態做差異比對。
5. 只重建受影響套件的 sidecar。
6. 對 WinGetUtil 的可變資料庫套用 add/remove 操作。
7. 產生 staging 發佈樹並輸出 `source2.msix`。
8. 只有整個流程成功後，才提交新的輸出與狀態。

## 狀態庫

狀態庫是一個 SQLite 資料庫，記錄：

- 目前檔案快照
- 目前版本快照
- 目前套件快照
- 已發佈檔案清單
- 每次建置的版本/套件差異紀錄

因此建置器不依賴 Git commit 形狀，而是比較「目前倉庫狀態」與「上一次成功發佈狀態」。

## 雜湊模型

- `raw_file_hash`：只用於掃描快取。
- `version_content_sha256`：語意層級的 manifest 身分，用於判斷是否重新發佈。
- `version_installer_sha256`：安裝程式相關身分，用於決定是否重新驗證。
- `published_manifest_sha256`：託管合併 manifest 的精確位元組雜湊。
- package publish hash：`versionData.mszyml` 精確位元組雜湊。

`Commands`、`Protocols` 與 `FileExtensions` 不參與安裝程式雜湊，但仍參與完整內容雜湊。

## 輸出契約

V1 會發佈：

- `source2.msix`
- `packages/.../versionData.mszyml`
- `manifests/...`

核心層已將 catalog 格式處理放在抽象之後，未來若有新的來源格式，可以透過新增 writer 來支援。
