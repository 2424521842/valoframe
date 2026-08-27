# Windows 发布说明

本文定义 Windows 安装包的发布边界和可复现流程。当前默认产物是 NSIS 安装器；MSI 尚未作为受支持渠道配置或验证。

当前发布路线为：用户手动安装 `v0.2.1`，再用 `v0.2.2` 完成第一次应用内更新。v0.2.0/v0.2.1 的旧版本计划保留为历史工程资料，不再作为个人开发者稳定发布的严格审批门禁。

## 发布渠道

### 内部未签名 RC

内部 RC 只用于仓库负责人或同一法律主体控制的受限测试设备上的安装、升级和卸载验证，不是公开发布：

- 可以暂时不做 Authenticode 签名，但必须在分发说明中明确“未签名”，并预期 Windows SmartScreen/信誉提示。
- FFmpeg 对应源码和第三方许可证尚未闭合时，不得向主体外的测试者分发；任何内部传递也必须经过实际许可负责人确认。
- 不得上传公共下载页、包管理器或自动更新端点。
- 不启用自动更新，不把内部 RC 当作后续公开版本的可信更新源。
- 每个 RC 仍需记录版本、Git 提交、构建环境、安装器 SHA-256，以及所含 FFmpeg 的版本、来源和 SHA-256。

### 公开发布

个人开发者稳定发布使用 `personal-community-stable` profile，由规范的 `vMAJOR.MINOR.PATCH` tag 触发。Tauri updater 签名、版本单调递增、下载地址/安装器版本绑定、512 MiB 上限、最终 bundle 内容、启动烟测、最小 FFmpeg 的精确对应源码/许可证附件和 draft 远端复核是强制门禁。旧的品牌、独立法律审批、完整 VM 矩阵和 Authenticode policy 继续作为未来增强检查，但不阻止个人 updater 发布。该 profile 只记录负责人对免费个人社区渠道的决定，不把“非商业”写成 MIT 附加限制，也不声称 Riot Games、腾讯、FFmpeg 项目或独立律师批准。

没有 Authenticode 的安装器可能显示“未知发布者”或 SmartScreen 提示；发布说明和下载页面必须明确披露。Community Beta 仍不得冒充稳定更新版本。

### 未签名 Community Beta

`Unsigned community beta` workflow 是给免费社区工具准备的独立 GitHub Prerelease 通道。它必须从默认分支手动触发，绑定完整 40 位源码 SHA，并输入 `UNSIGNED-COMMUNITY-BETA <tag> <SHA>`，只接受 `v<version>-beta.<序号>` 标签。该通道：

- 保留当前清单中的游戏图片和项目图标，并绑定发布负责人的 beta 渠道决定；
- Rust 中保留同源 updater 运行时，但不嵌入公钥、不启用检查，也不生成 `latest.json` 或更新签名；
- 明确公开安装器、内嵌主程序均为 `NotSigned`，并在发布说明中提示 Unknown Publisher/SmartScreen；
- 从固定 FFmpeg commit 构建零外部库的最小 LGPL 版本，在 Windows x64 复核后才进入安装包；
- 将 FFmpeg 二进制/构建证据、许可证和精确对应源码与安装器一起上传；
- 运行独立 `-AllowUnsignedCommunityBeta` bundle profile，同时反向证明严格 public preflight 与默认 bundle gate 仍然阻断；
- 只创建 GitHub Prerelease，绝不覆盖既有 Release；仅当失败上传留下的标签仍精确指向本次批准 SHA 时允许安全重试；
- 要求 `community-beta-publish` 环境至少配置 required reviewer，并把 deployment branch 限定到默认分支；个人仓库可由负责人自己完成发布前的第二次确认。

具体下载提示和操作步骤见 [`COMMUNITY_BETA.md`](./COMMUNITY_BETA.md)。该通道是一次明确披露限制的社区测试决定，不会修改 `release/public-release-policy.json`，也不会伪造 Authenticode、时间戳、正式 VM 或法律审批。

## 固定的安装器契约

`src-tauri/tauri.conf.json` 明确规定：

- 默认只生成 NSIS 安装器。
- 安装范围为 `currentUser`，普通用户无需机器级安装权限。
- 禁止安装比现有版本更低的版本；需要回退时发布修复后的更高补丁版本。
- WebView2 使用静默的 `downloadBootstrapper` 安装模式。目标机器缺少 WebView2 时，安装过程需要联网；离线场景必须作为单独发行渠道重新设计和验证。
- 安装器包含简体中文和英文，默认由系统语言选择，无法匹配时回退到列表中的简体中文。
- `src-tauri/resources/` 整体映射到安装后的 `$RESOURCE/`；其中 `resources/bin/ffmpeg.exe` 因而落在 `$RESOURCE/bin/ffmpeg.exe`。该相对路径是运行时代码与打包配置之间的契约。

目录级映射让受版本控制的许可证/来源元数据可以保证资源根存在，因此普通 `cargo check/test` 不要求先下载大体积 FFmpeg。发布构建仍必须由静态检查确认实际 `ffmpeg.exe` 已 staged 且与受审 manifest 一致；目录存在不等于发布资源齐备。

稳定更新端点已经固定，正式公钥在构建时从仓库 Variable `VALOFRAME_UPDATER_PUBLIC_KEY` 注入；未注入公钥的开发构建保持 updater 未配置。仓库不写入真实私钥、公钥占位值或虚构审批信息。

## 版本和身份

每次 RC 或公开发布前，确认以下版本完全一致：

- `package.json` 与 `package-lock.json` 顶层/root package 的 `version`
- `src-tauri/Cargo.toml` 的 `package.version` 与 `src-tauri/Cargo.lock` 中 `valorant-highlight-manager` package 的版本
- `src-tauri/tauri.conf.json` 的 `version`

稳定标签必须精确为 `vMAJOR.MINOR.PATCH`。`v0.2.1` 是手动安装起点，`v0.2.2` 是首次 OTA 验收版本。存在对应 `release/notes/vX.Y.Z.md` 时使用该说明，否则 workflow 生成默认说明；不得只修改某一个 manifest，也不得改写第三方依赖中碰巧相同的版本。

个人社区首发仍会冻结产品名称和应用 identifier，避免破坏升级路径。合法 publisher、Authenticode 主体与完整干净 VM 矩阵属于未来严格发行加固；若以后启用或更改 identifier、安装范围、publisher 或安装器渠道，应通过干净虚拟机验证。

## FFmpeg 发布材料

安装器包含 FFmpeg 之前，发布负责人必须为实际二进制保留：

- 精确版本、下载来源、下载时间和 SHA-256；
- 构建配置、启用的组件/编解码器和适用许可证结论；
- 软件物料清单（SBOM）或等价组件清单；
- 对应源代码的长期镜像或满足所选构建许可证要求的书面提供方式；
- 随安装包分发的第三方声明文本。

不要仅凭“FFmpeg”名称推断许可证。许可证义务取决于实际构建及其启用组件；无法追溯的二进制不得进入公开安装包。

当前固定的 BtbN `win64-lgpl` 二进制继续只用于内部 RC，不进入 Community Beta 或个人社区稳定版。它实际启用了大量本应用不需要的外部组件，因此仓库另设最小构建链：从固定 FFmpeg commit 交叉编译只保留 `file/mov`、H.264/HEVC parser+软件 decoder、`scale/mjpeg/image2`，同时生成精确源码包、构建参数、工具链和两种无用户内容合成 MP4 的逐项缩略图烟测证据。固定 commit 的内置 `av1` decoder 只提供硬件加速路径，不能作为无 GPU 环境的软件解码承诺，因此个人稳定版仍索引和保留 AV1 MP4，但不提供 AV1 缩略图；系统播放器回退的实际播放能力取决于系统解码组件。Windows 门禁会复核 `SHA256SUMS.txt`、`BUILD-METADATA.json`、12 位固定 commit 标识，并直接解析 PE import table；实际导入必须与交叉工具链 `objdump` 结果一致且仅命中固定的 Windows 系统 DLL allowlist。候选报告固定为 `passed-candidate-not-promoted` 且 `promotionAuthorized = false`；Community Beta 或 `personal-community-stable` workflow 只能在各自发布负责人渠道决定、源码 sidecar、许可证正文和专用 bundle 门禁同时成立时使用它。个人社区证据会如实标记 MinGW/工具链运行时许可复核与目标市场编解码器专利复核尚未完成，不得写成已审批；未来严格发行仍保留代表性真实素材回归、固定软件 AV1 decoder、Authenticode、VM 及完整审阅要求。

`scripts/release/generate-compliance-evidence.mjs` 从 `package-lock.json`、Windows x64 Cargo 解析图和 FFmpeg manifest 生成两份 npm SPDX 2.3、Windows Cargo SPDX 2.3、FFmpeg 组件快照、第三方声明、去重后的许可证全文、索引、阻断摘要和逐文件 SHA-256 manifest。可在不存在输出目录时运行 `npm run release:compliance:generate`。发布归档漏带正文的精确依赖由 `third_party/licenses/license-text-overrides.json` 补充：manifest 固定版本、锁文件 checksum/integrity、上游提交、SPDX 正文覆盖、正文大小和 SHA-256，生成器只在包内正文确实为空时离线应用，并拒绝未使用、重复、越界、symlink/junction、未进入 Git index、版本/SPDX/VCS/哈希不一致的 override。`license-text-override-approvals.json` 使用绑定组件与正文哈希的独立结构化记录。严格 profile 会把缺少逐条人工复核记录保留为 blocker；`personal-community-stable` 只把这些人工复核和 FFmpeg 专利/独立法审状态列为 advisory，但许可证正文缺失、锁文件/SPDX/VCS/哈希不一致、`selectors` 的 MPL-2.0 源码形式记录缺失、FFmpeg 对应源码缺失或外部/GPL/nonfree 组件出现仍会硬失败。技术证据不能被自动写成“已批准”。

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

node .\scripts\release\generate-compliance-evidence.mjs `
  --output src-tauri\resources\licenses\third-party `
  --offline

npm run release:bundle:windows:internal
```

构建前必须由发布静态检查确认 `src-tauri/resources/bin/ffmpeg.exe` 已 staged，且与本次发布记录/manifest 的哈希一致。隔离构建的安装器位于 `$env:CARGO_TARGET_DIR\release\bundle\nsis\`。`--no-sign` 只适用于内部 RC 和明确披露限制的 Community Beta；严格正式公开发布不得沿用该参数。

`scripts/release/check-bundle.ps1` 还必须同时接收本次 Tauri 构建生成的 `release\nsis\...\installer.nsi` 和完整 7-Zip 的 `7z.exe`。门禁会先校验 PE overlay 上的 NSIS first header，再把 `installer.nsi` 的 `Section Install` 静态绑定到本次主程序、`bin\ffmpeg.exe`、项目 MIT 许可证及范围说明、FFmpeg 许可文件以及 `COMPLIANCE-MANIFEST.json` 声明的全部第三方材料；随后只把这些目标从最终安装器受控解包到一次性目录。当前快照为 17 个文件，但数量由 manifest 驱动，不再硬编码。根 `LICENSE`/`LICENSE-SCOPE.txt`、公开 policy 中获批的 SHA-256 与安装目录对应文件必须逐字节一致。所有资源必须逐字节匹配；Tauri 会在打包期间把主程序中唯一的 `__TAURI_BUNDLE_TYPE_VAR_UNK` 改为同偏移的 `..._NSS`。内部未签名模式使用 `strict-unsigned` 比较，整文件除这三个 marker 字节外不得有任何差异。公开模式使用 `authenticode-aware` 比较：外部 UNK staging 文件必须 `NotSigned` 且无证书表；内嵌 NSS 文件的证书表必须从 `Align8(staging file length)` 开始、只允许 0–7 个零 padding、以合法 WIN_CERTIFICATE 项精确覆盖到 EOF。只在 NSS→UNK 后同时归零 PE checksum 和 security-directory entry，再比较 staging 文件长度内包括合法 overlay 在内的全部字节。内嵌 NSS 主程序和安装器不仅必须为 `Valid`，还必须与公开 policy 中获批的 publisher subject 和证书 thumbprint 精确一致、存在时间戳证书，并额外通过微软签名的 `signtool verify /pa /all /v`。报告明确区分两种比较模式并留存 raw/canonical 哈希、证书表边界、签名者、时间戳和 signtool 证据。

CI 将已通过检查的全部 NSIS 载荷保留到 `RUNNER_TEMP` 下的预先为空目录。移动前后都重新验证临时根目录、父链和输出目录没有 reparse point，移动后再按最终报告重新枚举并复核文件数量及每项哈希。启动烟测重新读取该 JSON 报告，逐项核对路径、大小和 SHA-256，并只使用报告中的 `rawEmbeddedSha256` 授权真实 NSS 主程序；不得现场计算一个新哈希再自授权。未显式请求输出时，静态检查仍会删除一次性解包目录。

手工 `Windows internal release readiness` workflow 还会生成一个保留 30 天的 `vhm-internal-rc-evidence-<run id>-<attempt>` 证据 artifact。它不上传未签名安装器、staging 主程序或受控解包载荷；除原有 JSON 报告外，还上传生成的 SPDX、第三方声明和许可证文本。内容包括：bundle gate 原始报告、重新核对后的动态载荷 SHA-256 清单、安装器与 staging 主程序的大小/SHA-256/`NotSigned` 状态、工具链元数据、`public-release-preflight.json`、公开 bundle 反向门禁结果，以及通过后的启动烟测报告。workflow 失败时仍尝试上传已经生成的证据；缺失的后续报告表示门禁尚未走到该阶段，不能解释为通过。

该 workflow 的输入契约固定为 `--no-sign` 内部 RC，并在进入公开反向门禁前再次要求 staging 主程序和安装器均为 `NotSigned`。公开模式调用不会传递 `-AllowUnsignedInternalRc`：它必须因为尚未关闭的 FFmpeg、第三方合规、公开签名 policy 或未签名 Authenticode 阻断而失败；这些预期域使用稳定前缀进入白名单，意外通过或任何其他错误都会使 workflow 失败。证据 artifact 的存在和内部 RC 门禁通过都不构成公开发布批准。

该检查不会运行安装器，只证明最终压缩包中存在与本次输入一致的主程序、FFmpeg 和全部声明的合规文件；它不证明安装、升级、卸载、WebView2 引导或应用运行行为。个人社区发布另外执行隔离启动烟测；完整安装/升级/卸载、WebView2 引导和干净虚拟机矩阵仍是未来严格发行证据，不能由静态解包替代。

`release/public-release-policy.json` 与 `scripts/release/public-release-preflight.ps1` 保留为严格公开发行的未来加固检查。它们可以报告第三方材料、品牌、publisher、identifier、代码签名和 VM 等未完成项，但不再被个人开发者 `stable-release` workflow 当作 updater 发布阻断。Tauri updater 签名仍不可跳过。

旧的干净 VM 与数据安全归档流程继续作为未来严格发行的可选证据机制。它要求归档在 checkout 外安全展开、manifest 绑定精确提交并逐项校验 SHA-256，但个人 updater 发布不再要求通过受保护 Environment 注入该归档。首次 OTA 的当前验收路线固定为 `v0.1.0-beta.1` 手动升级到 `v0.2.1`，再由 `v0.2.1` 签名升级到 `v0.2.2`。

未来严格发行的机器可读 evidence allowlist 可继续使用以下历史记录；它们不属于当前个人 updater workflow 的阻断项：

- `same-source-subdirectory-rename-auto-reconnect-user-state-preserved`：同一授权来源内子目录改名后自动重连，并保持 clip ID、收藏、标签、备注和评审状态。
- `source-root-relocation-user-state-preserved`：用户预览并提交新根后恢复播放，保持全部索引状态，且完整同步成功前不伪造扫描新鲜度。
- `kill-death-timeline-icons-tooltips-accessibility-and-seek`：本人击杀/本人死亡数量、红色准星/紫色骷髅、tooltip、无障碍名称和点击跳转秒数一致。
- `signed-updater-v0.2.1-to-v0.2.2-schema-v18-user-state-preserved`：从手动安装的 v0.2.1 经签名 updater 升级，随后确认 schema v18 与用户状态完整。
- `index-only-removal-source-media-sha256-unchanged`：单条和批量仅移除索引后复核原视频 SHA-256 不变，并记录部分失败；该项属于 data-safety manifest。

历史材料可以继续保留，当前路线使用 `v0.1.0-beta.1-manual-upgrade-to-v0.2.1` 和通用 `signed-updater-upgrade-to-higher-patch` 场景。

若既有代码尚未达到严格 Clippy 零告警，应记录实际告警并先修复，不能把跳过检查变成公开发布惯例。

## 验证和留档

对最终安装器，而不是中间可执行文件，完成以下工作：

1. 记录文件名、大小和 SHA-256。
2. 在从未安装过本应用的标准用户 Windows 虚拟机执行安装、首次启动、卸载。
3. 分别覆盖已安装/未安装 WebView2，以及允许/禁止联网的场景。
4. 首次 OTA 单独验证手动安装 `v0.2.1` 后通过签名 updater 升级到 `v0.2.2`；升级后确认 schema v16、来源配置、clip ID、收藏、标签、备注、评审、回收/删除 intent 和用户素材引用保持正确，用户原始视频不被修改。
   同时按 [数据库恢复指南](./DATABASE_RECOVERY.md) 演练 v13/v14/v15→v16 迁移前在线备份、未来 schema 拒绝、损坏库拒绝和从最近备份恢复；恢复输入与结果须进入 RC 证据记录。
5. 验证降级被拒绝，并使用“更高补丁版本”演练回退方案。
6. 检查安装目录实际存在 `bin/ffmpeg.exe`，缩略图生成走打包资源而非系统 `PATH`。
7. 卸载后确认应用安装文件被移除，而用户数据库和素材的保留/删除行为符合已发布的数据策略。
8. 留存测试矩阵、日志、安装器、校验和、SBOM、第三方声明、签名验证结果和发布说明。

若未来启用 Authenticode，应在干净虚拟机验证签名、时间戳和 publisher；当前个人发布允许没有 Authenticode，但必须验证并披露 Windows 的未知发布者提示。

### 安全烟测根目录

Windows 上 Tauri 通过 Known Folder API 解析应用数据目录；仅在子进程中覆盖 `APPDATA` 或 `LOCALAPPDATA` **不能**安全隔离真实数据库和缩略图缓存。发布烟测必须使用运行时提供的窄化开关 `VHM_RELEASE_SMOKE_ROOT`，并满足全部条件：

- 值为已经存在的绝对路径，规范化后的末级目录名以 `vhm-release-smoke-` 开头；
- 根目录本身不是符号链接或 Windows reparse point，启动前只能包含 `.vhm-release-smoke-root` 这一个普通文件；其内容去除首尾空白后精确为 `vhm-release-smoke-root-v1`；
- 根目录与真实应用 data/cache 目录互不等同、互不为父子目录；
- 配置窗口使用 `create: false`，只有完成上述验证后才手动创建；烟测 WebView2 profile 明确写入 `<root>/webview2`；
- 数据库写入 `<root>/data`，缩略图缓存写入 `<root>/cache/thumbnails`；`TEMP`/`TMP`、进程输出日志和禁止真实扫描的哨兵路径位于 root 外、由脚本 marker-gated 的 sibling；
- 不覆盖 `APPDATA`、`LOCALAPPDATA`、`USERPROFILE`、`HOME`、`HOMEDRIVE` 或 `HOMEPATH`：这些值不是 Windows Known Folder 的可靠隔离边界，伪造后还可能令 `SHGetKnownFolderPath` 返回 `PATH_NOT_FOUND`；启动脚本先拒绝继承外部提供的非空 `WEBVIEW2_USER_DATA_FOLDER`，再为首、次实例统一注入受控的 `<root>/webview2`。该环境变量在任何 Tauri setup 之前隔离 WebView2，Rust `WebviewWindowBuilder.data_directory(<root>/webview2)` 继续作为应用侧第二道约束；报告和 workflow 必须验证两者精确一致；
- 主进程必须先以 suspended 状态创建、加入启用 `KILL_ON_JOB_CLOSE` 的 Windows Job，再恢复执行；只向子进程继承受限的 stdin/stdout/stderr 句柄，失败报告保留有界输出；
- CI 启动烟测的可执行文件和资源必须来自 bundle gate 已验证并保留的 NSIS payload，不得回退到构建目录中的 UNK staging 主程序；
- 新数据库必须是 schema v16、`requiredTableCount = 16`，包含来源类型/扫描模式/扫描根、卡片审核、clips 三段文件身份、`clip_events.killed_is_me` 和 `scan_runs.summary_available` 字段，以及空的 `clip_trash_snapshots` 回收身份目录和 `clip_delete_intents` 删除意图日志；JSON 报告记录 `database.trashSnapshotCount = 0` 与 `database.deleteIntentCount = 0`；
- 同一 smoke root 再启动第二实例时，首窗口先进入最小化状态；`runtime.singleInstance.verified` 必须为 `true`、`secondInstanceExitCode` 和 `secondInstanceJobActiveProcessesAfterExit` 必须为 `0`、`onlyPrimaryNamedRootAfterHandoff`、`primaryWindowMinimizedBeforeHandoff`、`primaryWindowHandlePreserved`、`primaryWindowVisibleAfterHandoff` 必须为 `true`，且 `primaryWindowMinimizedAfterHandoff` 必须为 `false`；报告同时保留首/次 PID、进程/窗口清单及 `focusVerification`，但前台焦点切换只作为 best-effort 证据，不得把操作系统拒绝抢焦点误判为单实例失败；
- 窗口门禁只接受主进程在隐藏前唯一可见、优先无 owner 且有标题的顶层窗口，并记录 HWND、PID、标题和 class；`WM_CLOSE` 后既要验证该 HWND 消失，也要验证主进程以 0 退出、Job 内 active process 为 0；
- Python 只读检查器在 `PRAGMA quick_check` 前注册与 Rust `db.rs` 一致的 `VHM_CLIP_NAME` collation，否则带自然名称索引的健康数据库会被误判为不可检查。

任一应用侧烟测根目录前置校验失败时，应用应在创建窗口、数据库或缓存前拒绝以烟测模式启动；启动后的数据库、窗口、进程与真实目录检查则必须失败关闭。烟测工具只能清理自己创建且再次通过同一名称、marker 和路径边界校验的父目录；不得用通配符清理，也不得把修改 `APPDATA` 描述为隔离措施。安装器级烟测结束后，还要确认真实应用 data/cache 及 WebView2 profile 未被创建或修改。

## 自动更新

现有应用内稳定更新工程保持不变，详细契约和密钥配置见[应用内更新](./APP_UPDATES.md)：

- 后端固定 `https://github.com/2424521842/valoframe/releases/latest/download/latest.json`，前端不能替换 endpoint 或公钥；
- 设置/关于页提供每日一次且会在焦点恢复时补查的非阻塞自动检查、无限频的手动检查、发布说明、确认下载、进度/验签/取消、失败直重试、显式放弃会话和确认安装，并在主导航显示可用或待安装状态；
- Tauri updater 原始 plugin commands 不向窗口 capability 暴露；运行时严格绑定规范版本、GitHub 仓库/tag 下载地址与已验签 ZIP 内安装器版本，并限制下载/解压大小；Windows 只有确认安装器启动成功后才退出，启动失败保留已验证包供重试；
- 扫描、永久删除、视频导出、来源根重新定位与安装由 Rust 关键任务门禁互斥；
- Community Beta 不嵌入公钥且不生成 updater 资产；稳定构建从仓库 Variable/Secrets 注入公钥、私钥和私钥密码；
- `stable-release` workflow 由规范稳定 tag 自动触发，拒绝版本错配、倒退、重复 Release 和缺失密钥；先创建并重新下载验证不可见 draft 的全部资产，公开后二次验证失败则把精确 Release 恢复为 draft；
- `lanzou-sync` workflow 只在上述稳定发布成功后运行，从公开 Release 重新下载版本化安装器和 `SHA256SUMS.txt`，核对哈希后同步到固定蓝奏云文件夹，并把文件夹链接、提取码、文件名和 SHA-256 写入该版本发布说明；镜像失败不回滚已验证的 GitHub Release；
- `v0.1.0-beta.1` 用户必须手动安装一次 `v0.2.1`；只有嵌入正确公钥并指向稳定端点的该版本，才能应用内升级到 `v0.2.2`。Community Beta、内部 RC、prerelease 或未配置公钥的构建不具备该资格。

发布前必须配置仓库 Secrets `TAURI_SIGNING_PRIVATE_KEY`、`TAURI_SIGNING_PRIVATE_KEY_PASSWORD` 和 Variable `VALOFRAME_UPDATER_PUBLIC_KEY`，并保留一份加密离线私钥备份。缺少任一配置时，稳定 workflow 在构建前失败。

蓝奏云自动镜像需要一次性配置 Repository Secret `LANZOU_COOKIE` 和 Repository Variable `LANZOU_FOLDER_ID`。前者应是登录 `pc.woozooo.com` 后浏览器请求中的单行 Cookie，至少包含 `ylogin` 与 `phpdisk_info`；后者必须是已经开启分享的非根目录数字 ID。Cookie 只能保存到 Actions Secret，不得写入仓库、Issue、日志或发布说明。Cookie 失效时只需更新 Secret；可从 Actions 手动运行 `Sync stable release to Lanzou Cloud` 并指定已经发布的稳定 tag 进行补传。上传器保留蓝奏云官方 100 MiB 和扩展名限制，不启用分片、伪装或关闭 TLS 校验；同名文件只有带有相同 SHA-256 描述标记时才视为幂等成功，否则拒绝覆盖。

不得把 Authenticode 与 Tauri 更新签名视为同一件事：更新包必须具有 Tauri/Minisign 签名；Windows Authenticode 当前可选，只用于减少未知发布者和 SmartScreen 信誉提示。

## 未来严格公开发行检查

以下检查继续由 policy 跟踪，不得伪造为已完成；它们不阻止当前个人开发者稳定 updater 发布，但在品牌商业化、扩大分发或要求 Authenticode 时应逐项关闭：

1. 第三方声明、SBOM、许可证正文/override 及人工审批。
2. 产品名称和品牌审批。
3. 真实 publisher/法律主体及证书主体审批。
4. 稳定 application identifier 审批。
5. 游戏素材覆盖公开源码、应用展示、安装器、下载和宣传渠道的分发权。
6. 图标覆盖 Windows 安装器与公开下载的分发权。
7. Riot Games、腾讯和《无畏契约》的商标归属及非官方/非赞助声明。
8. FFmpeg 二进制来源、许可/专利审核、SBOM 与精确源码镜像/提供方式。
9. Authenticode 证书、获批 subject/thumbprint 与签名服务。
10. 可信 HTTPS 时间戳服务与验证策略。
11. Windows 10/11 干净 VM 安装、升级、降级拒绝、卸载和真实媒体证据。
12. updater 启用决定、密钥托管、公钥引用、稳定 HTTPS endpoint 和发布权限。
13. 来源媒体只读、永久删除、应用数据边界与卸载保留策略的数据安全证据。

`release/public-release-policy.json` 保留这些状态供诊断和未来严格发行使用。当前个人社区稳定发布的硬门禁以 updater 签名、版本/地址绑定、自动化测试、实际 bundle/启动检查、最小 LGPL FFmpeg 的二进制与精确对应源码/许可证附件，以及远端资产复核为准；渠道决定固定在 `release/approvals/personal-community-stable-v0.2.1.json`。
