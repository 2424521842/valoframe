# Windows 发布说明

本文定义 Windows 安装包的发布边界和可复现流程。当前默认产物是 NSIS 安装器；MSI 尚未作为受支持渠道配置或验证。

## 发布渠道

### 内部未签名 RC

内部 RC 只用于仓库负责人或同一法律主体控制的受限测试设备上的安装、升级和卸载验证，不是公开发布：

- 可以暂时不做 Authenticode 签名，但必须在分发说明中明确“未签名”，并预期 Windows SmartScreen/信誉提示。
- FFmpeg 对应源码、第三方许可证和项目许可尚未闭合时，不得向主体外的测试者分发；任何内部传递也必须经过实际许可负责人确认。
- 不得上传公共下载页、包管理器或自动更新端点。
- 不启用自动更新，不把内部 RC 当作后续公开版本的可信更新源。
- 每个 RC 仍需记录版本、Git 提交、构建环境、安装器 SHA-256，以及所含 FFmpeg 的版本、来源和 SHA-256。

### 公开发布

当前仓库不能据此文档直接宣称“可公开发布”。在 `WINDOWS_RELEASE_CHECKLIST.md` 的公开发布阻断项全部关闭前，只能生成内部 RC。

## 固定的安装器契约

`src-tauri/tauri.conf.json` 明确规定：

- 默认只生成 NSIS 安装器。
- 安装范围为 `currentUser`，普通用户无需机器级安装权限。
- 禁止安装比现有版本更低的版本；需要回退时发布修复后的更高补丁版本。
- WebView2 使用静默的 `downloadBootstrapper` 安装模式。目标机器缺少 WebView2 时，安装过程需要联网；离线场景必须作为单独发行渠道重新设计和验证。
- 安装器包含简体中文和英文，默认由系统语言选择，无法匹配时回退到列表中的简体中文。
- `src-tauri/resources/` 整体映射到安装后的 `$RESOURCE/`；其中 `resources/bin/ffmpeg.exe` 因而落在 `$RESOURCE/bin/ffmpeg.exe`。该相对路径是运行时代码与打包配置之间的契约。

目录级映射让受版本控制的许可证/来源元数据可以保证资源根存在，因此普通 `cargo check/test` 不要求先下载大体积 FFmpeg。发布构建仍必须由静态检查确认实际 `ffmpeg.exe` 已 staged 且与受审 manifest 一致；目录存在不等于发布资源齐备。

配置没有填入尚未确定的 publisher、证书、许可证文件、更新端点或更新公钥。不得用占位值绕过公开发布阻断。

## 版本和身份

每次 RC 或公开发布前，确认以下版本完全一致：

- `package.json` 的 `version`
- `src-tauri/Cargo.toml` 的 `package.version`
- `src-tauri/tauri.conf.json` 的 `version`

公开首发前必须审核并冻结产品名称、合法 publisher 和应用 identifier。公开发布后更改 identifier、安装范围或安装器渠道，可能被 Windows 视为不同产品或破坏升级路径，必须通过干净虚拟机验证。

## FFmpeg 发布材料

安装器包含 FFmpeg 之前，发布负责人必须为实际二进制保留：

- 精确版本、下载来源、下载时间和 SHA-256；
- 构建配置、启用的组件/编解码器和适用许可证结论；
- 软件物料清单（SBOM）或等价组件清单；
- 对应源代码的长期镜像或满足所选构建许可证要求的书面提供方式；
- 随安装包分发的第三方声明文本。

不要仅凭“FFmpeg”名称推断许可证。许可证义务取决于实际构建及其启用组件；无法追溯的二进制不得进入公开安装包。

## 内部 RC 构建

在干净工作区中使用仓库锁文件安装依赖，并把 Rust/Tauri 输出指向隔离目录，避免污染或复用默认的 `src-tauri/target`：

```powershell
npm ci
npm test
npm run build

$env:CARGO_TARGET_DIR = Join-Path $PWD '.tmp\windows-release-target'
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo clippy --manifest-path src-tauri/Cargo.toml --locked --all-targets --all-features -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml --locked --all-targets --all-features

npm run release:ffmpeg:prepare:internal
npm run release:ffmpeg:verify:internal

npm run release:bundle:windows:internal
```

构建前必须由发布静态检查确认 `src-tauri/resources/bin/ffmpeg.exe` 已 staged，且与本次发布记录/manifest 的哈希一致。隔离构建的安装器位于 `$env:CARGO_TARGET_DIR\release\bundle\nsis\`。`--no-sign` 只适用于内部 RC；公开发布不得沿用该参数。

`scripts/release/check-bundle.ps1` 还必须同时接收本次 Tauri 构建生成的 `release\nsis\...\installer.nsi` 和完整 7-Zip 的 `7z.exe`。门禁会先校验 PE overlay 上的 NSIS first header，再把 `installer.nsi` 的 `Section Install` 静态绑定到本次主程序、`bin\ffmpeg.exe`、LGPL/GPL 文本、`BUILD-INFO.json` 与 `SOURCE-OFFER.md`；随后只把这六个目标从最终安装器受控解包到一次性目录。五个资源必须逐字节匹配；Tauri 会在打包期间把主程序中唯一的 `__TAURI_BUNDLE_TYPE_VAR_UNK` 改为同偏移的 `..._NSS`。内部未签名模式使用 `strict-unsigned` 比较，整文件除这三个 marker 字节外不得有任何差异。公开模式使用 `authenticode-aware` 比较：外部 UNK staging 文件必须 `NotSigned` 且无证书表；内嵌 NSS 文件的证书表必须从 `Align8(staging file length)` 开始、只允许 0–7 个零 padding、以合法 WIN_CERTIFICATE 项精确覆盖到 EOF。只在 NSS→UNK 后同时归零 PE checksum 和 security-directory entry，再比较 staging 文件长度内包括合法 overlay 在内的全部字节；内嵌 NSS 主程序和安装器的 Authenticode 状态都必须为 `Valid`。报告明确区分两种比较模式并留存 raw/canonical 哈希及证书表边界。

CI 将已通过检查的六个 NSIS 载荷保留到 `RUNNER_TEMP` 下的预先为空目录。移动前后都重新验证临时根目录、父链和输出目录没有 reparse point，移动后再按最终报告重新枚举并复核恰好六个文件。启动烟测重新读取该 JSON 报告，逐项核对路径、大小和 SHA-256，并只使用报告中的 `rawEmbeddedSha256` 授权真实 NSS 主程序；不得现场计算一个新哈希再自授权。未显式请求输出时，静态检查仍会删除一次性解包目录。

手工 `Windows internal release readiness` workflow 还会生成一个保留 30 天的 `vhm-internal-rc-evidence-<run id>-<attempt>` 证据 artifact。它只上传 JSON 元数据，不上传未签名安装器或六个解包载荷，内容包括：bundle gate 原始报告、重新核对后的六文件 SHA-256 清单、安装器与 staging 主程序的大小/SHA-256/`NotSigned` 状态、GitHub run 与 Node/npm/Python/Rust/Cargo/Tauri/PowerShell 工具链元数据、公开发布反向门禁结果，以及通过后的启动烟测报告。workflow 失败时仍尝试上传已经生成的证据；缺失的后续报告表示门禁尚未走到该阶段，不能解释为通过。

该 workflow 的输入契约固定为 `--no-sign` 内部 RC，并在进入公开反向门禁前再次要求 staging 主程序和安装器均为 `NotSigned`。公开模式调用不会传递 `-AllowUnsignedInternalRc`：它必须因为尚未关闭的 FFmpeg redistribution 阻断或未签名 Authenticode 阻断而失败；意外通过或任何非预期错误都会使 workflow 失败。证据 artifact 的存在和内部 RC 门禁通过都不构成公开发布批准。

该检查不会运行安装器，只证明最终压缩包中存在与本次输入一致的六个关键文件；它不证明安装、升级、卸载、WebView2 引导或应用运行行为。下面的干净虚拟机矩阵仍是发布必需证据，不能由静态解包替代。

若既有代码尚未达到严格 Clippy 零告警，应记录实际告警并先修复，不能把跳过检查变成公开发布惯例。

## 验证和留档

对最终安装器，而不是中间可执行文件，完成以下工作：

1. 记录文件名、大小和 SHA-256。
2. 在从未安装过本应用的标准用户 Windows 虚拟机执行安装、首次启动、卸载。
3. 分别覆盖已安装/未安装 WebView2，以及允许/禁止联网的场景。
4. 从上一公开版本升级，确认数据库、来源配置、标签和用户素材引用保持正确，用户原始视频不被修改。
   同时按 [数据库恢复指南](./DATABASE_RECOVERY.md) 演练迁移前在线备份、未来 schema 拒绝、损坏库拒绝和从最近备份恢复；恢复输入与结果须进入 RC 证据记录。
5. 验证降级被拒绝，并使用“更高补丁版本”演练回退方案。
6. 检查安装目录实际存在 `bin/ffmpeg.exe`，缩略图生成走打包资源而非系统 `PATH`。
7. 卸载后确认应用安装文件被移除，而用户数据库和素材的保留/删除行为符合已发布的数据策略。
8. 留存测试矩阵、日志、安装器、校验和、SBOM、第三方声明、签名验证结果和发布说明。

公开发布还必须在干净虚拟机上验证 Authenticode 签名和时间戳，且签名主体与已批准的 publisher 一致。

### 安全烟测根目录

Windows 上 Tauri 通过 Known Folder API 解析应用数据目录；仅在子进程中覆盖 `APPDATA` 或 `LOCALAPPDATA` **不能**安全隔离真实数据库和缩略图缓存。发布烟测必须使用运行时提供的窄化开关 `VHM_RELEASE_SMOKE_ROOT`，并满足全部条件：

- 值为已经存在的绝对路径，规范化后的末级目录名以 `vhm-release-smoke-` 开头；
- 根目录本身不是符号链接或 Windows reparse point，启动前只能包含 `.vhm-release-smoke-root` 这一个普通文件；其内容去除首尾空白后精确为 `vhm-release-smoke-root-v1`；
- 根目录与真实应用 data/cache 目录互不等同、互不为父子目录；
- 配置窗口使用 `create: false`，只有完成上述验证后才手动创建；烟测 WebView2 profile 明确写入 `<root>/webview2`；
- 数据库写入 `<root>/data`，缩略图缓存写入 `<root>/cache/thumbnails`；`TEMP`/`TMP`、进程输出日志和禁止真实扫描的哨兵路径位于 root 外、由脚本 marker-gated 的 sibling；
- 不覆盖 `APPDATA`、`LOCALAPPDATA`、`USERPROFILE`、`HOME`、`HOMEDRIVE` 或 `HOMEPATH`：这些值不是 Windows Known Folder 的可靠隔离边界，伪造后还可能令 `SHGetKnownFolderPath` 返回 `PATH_NOT_FOUND`；也不设置 `WEBVIEW2_USER_DATA_FOLDER`，避免它覆盖 Rust builder 的显式 `<root>/webview2`；
- 主进程必须先以 suspended 状态创建、加入启用 `KILL_ON_JOB_CLOSE` 的 Windows Job，再恢复执行；只向子进程继承受限的 stdin/stdout/stderr 句柄，失败报告保留有界输出；
- CI 启动烟测的可执行文件和资源必须来自 bundle gate 已验证并保留的 NSIS payload，不得回退到构建目录中的 UNK staging 主程序；
- 新数据库必须是 schema v13、`requiredTableCount = 16`，包含空的 `clip_trash_snapshots` 回收身份目录和 `clip_delete_intents` 删除意图日志，并在 JSON 报告中记录 `database.trashSnapshotCount = 0` 与 `database.deleteIntentCount = 0`；
- 同一 smoke root 再启动第二实例时，首窗口先进入最小化状态；`runtime.singleInstance.verified` 必须为 `true`、`secondInstanceExitCode` 和 `secondInstanceJobActiveProcessesAfterExit` 必须为 `0`、`onlyPrimaryNamedRootAfterHandoff`、`primaryWindowMinimizedBeforeHandoff`、`primaryWindowHandlePreserved`、`primaryWindowVisibleAfterHandoff` 必须为 `true`，且 `primaryWindowMinimizedAfterHandoff` 必须为 `false`；报告同时保留首/次 PID、进程/窗口清单及 `focusVerification`，但前台焦点切换只作为 best-effort 证据，不得把操作系统拒绝抢焦点误判为单实例失败；
- 窗口门禁只接受主进程在隐藏前唯一可见、优先无 owner 且有标题的顶层窗口，并记录 HWND、PID、标题和 class；`WM_CLOSE` 后既要验证该 HWND 消失，也要验证主进程以 0 退出、Job 内 active process 为 0；
- Python 只读检查器在 `PRAGMA quick_check` 前注册与 Rust `db.rs` 一致的 `VHM_CLIP_NAME` collation，否则带自然名称索引的健康数据库会被误判为不可检查。

任一应用侧烟测根目录前置校验失败时，应用应在创建窗口、数据库或缓存前拒绝以烟测模式启动；启动后的数据库、窗口、进程与真实目录检查则必须失败关闭。烟测工具只能清理自己创建且再次通过同一名称、marker 和路径边界校验的父目录；不得用通配符清理，也不得把修改 `APPDATA` 描述为隔离措施。安装器级烟测结束后，还要确认真实应用 data/cache 及 WebView2 profile 未被创建或修改。

## 自动更新

自动更新当前不是发布契约的一部分。启用之前必须单独完成：

- 引入并验证 Tauri updater 插件及最小权限；
- 生成更新签名密钥对，离线备份私钥，把公钥写入配置；
- 确定 HTTPS 更新端点和 `latest.json` 托管、缓存及回滚策略；
- 在 CI/发布环境中以机密方式注入私钥和密码；
- 验证篡改包、错误签名、断网、跨版本升级和失败恢复。

不得把 Authenticode 与 Tauri 更新签名视为同一件事：公开安装器需要 Windows 代码签名，更新包还需要 updater 自己的签名链。

## 公开发布前的外部决策

以下内容必须由项目负责人或法律/发布负责人提供，仓库不能自行猜测：

- 真实 publisher/法律主体和稳定 identifier；
- 项目许可证、EULA（若需要）及第三方声明；
- 正式品牌图标和安装器视觉资产；
- FFmpeg 二进制来源、许可证审核、SBOM 与源代码镜像/提供方式；
- Authenticode 证书、签名服务和可信时间戳策略；
- updater 密钥托管、HTTPS 端点和发布权限；
- 支持的 Windows 版本、架构、在线/离线安装策略和升级兼容窗口。
