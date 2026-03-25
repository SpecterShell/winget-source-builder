# 使用说明

## 前置条件

- 完整构建流程需要 Windows 10/11。
- 运行时需要 `winget-source-builder.exe` 同目录下的 `WinGetUtil.dll`。Windows 构建会默认从仓库内置的 `winget-cli` 子模块自动生成它。
- 需要 Windows SDK 的 `makeappx.exe`，或设置 `MAKEAPPX_EXE`。
- 需要一个按 WinGet 清单结构组织的清单仓库。
- 源仓库根目录还需要包含 `packaging/msix/`。
- 从源码仓库运行时需要 Rust stable，并执行 `git submodule update --init --recursive`。只有在想覆盖内置 `winget-cli` 子模块时，才需要设置 `WINGET_CLI_ROOT`。

## 命令

构建静态源目录：

```powershell
cargo run -- build `
  --repo C:\path\to\repo `
  --state C:\path\to\state `
  --out C:\path\to\out `
  --lang zh-CN `
  --format v2
```

从打包产物运行：

```powershell
.\winget-source-builder.exe build `
  --repo C:\path\to\repo `
  --state C:\path\to\state `
  --out C:\path\to\out `
  --lang zh-CN `
  --format v2
```

## 环境变量

- `WINGET_CLI_ROOT`：`winget-cli` 源码仓库的绝对路径，用于在编译时引导 `WinGetUtil.dll`。
- `MAKEAPPX_EXE`：`makeappx.exe` 的绝对路径。
- `WINGET_SOURCE_BUILDER_WORKSPACE_ROOT`：覆盖默认的工作区根目录，用于定位 `packaging/msix/`。如果 `--repo` 已经指向源模板仓库内的目录，通常无需手动设置。
- `WINGET_SOURCE_BUILDER_LANG`：构建进度和摘要输出的运行时语言。只要 `locales/` 下存在对应语言文件，就可以使用，例如 `en` 或 `zh-CN`。

## 输出目录

- `source2.msix`：供 WinGet v2 客户端使用的 catalog 包。
- `packages/<PackageIdentifier>/<hash8>/versionData.mszyml`：包级 sidecar 数据。
- `manifests/...`：catalog 引用的内容寻址合并清单。

## 状态目录

- `state.sqlite`：增量状态库。
- `validation-queue.json`：安装器重新验证任务列表。
- `writer/mutable-v2.db`：持久化的 WinGetUtil 可变数据库。
- `staging/`：每次构建的临时工作目录。

## 增量行为

- 通过文件新增、删除与内容哈希来检测变化。
- 仅元数据变更时，会重新发布受影响包，但不会强制重新做安装器验证。
- 影响安装器的变更会写入 `validation-queue.json`。
- 如果 `--out` 下的上一次发布目录丢失，更新与删除操作无法复用旧的托管清单，需要走一次新的全量构建流程。
