# 架构说明

## 总体设计

项目实现为一个 Rust CLI，并在同一二进制内保留一层很薄的原生互操作边界。

- Rust 负责扫描、哈希、清单合并/解析、规范化、差异计算、状态管理、发布规划、WinGetUtil 互操作，以及 `source2.msix` 打包。
- `WinGetUtil.dll` 仍然是可变索引写入的兼容性后端，但现在由 Rust 在运行时直接加载。
- MSIX 静态资源放在源模板仓库的 `packaging/msix/` 下，而不是放在本构建器仓库里。

## 构建流程

1. 扫描仓库并对变化的 YAML 文件计算哈希。
2. 仅对脏的版本目录重新生成合并清单快照。
3. 计算：
   - `version_content_sha256`
   - `version_installer_sha256`
   - `published_manifest_sha256`
4. 将脏版本与上一次成功状态做差异比较。
5. 只重建受影响包的 sidecar。
6. 对 WinGetUtil 的可变数据库应用 add/remove 操作。
7. 生成 staging 发布树并产出 `source2.msix`。
8. 只有整个流程成功后，才提交新的输出与状态。

## 状态库

状态库是一个 SQLite 数据库，记录：

- 当前文件快照
- 当前版本快照
- 当前包快照
- 已发布文件清单
- 每次构建的版本/包差异记录

因此构建器不依赖 Git 提交形态，而是比较“当前仓库状态”和“上一次成功发布状态”。

## 哈希模型

- `raw_file_hash`：只用于扫描缓存。
- `version_content_sha256`：语义级清单标识，用于决定是否重新发布。
- `version_installer_sha256`：安装器相关标识，用于决定是否重新验证。
- `published_manifest_sha256`：托管合并清单的精确字节哈希。
- package publish hash：`versionData.mszyml` 精确字节哈希。

`Commands`、`Protocols` 与 `FileExtensions` 不参与安装器哈希，但仍参与完整内容哈希。

## 输出契约

V1 发布内容：

- `source2.msix`
- `packages/.../versionData.mszyml`
- `manifests/...`

核心层已经把 catalog 格式处理放在抽象后面，未来如果有新的源格式，可以通过新增 writer 来适配。
