# Windows 发布检查清单

本清单区分内部 RC、个人开发者稳定 updater 发布和未来严格公开发行。当前路线是手动安装 `v0.2.1`，再用 `v0.2.2` 验证第一次应用内更新。

## 1. 变更范围和版本

- [ ] 发布工作区干净，或所有未提交改动均已逐项审阅并记录。
- [ ] `package.json`、`package-lock.json` 顶层/root package、`src-tauri/Cargo.toml`、Cargo.lock 中项目 package 及 `src-tauri/tauri.conf.json` 均精确为 `0.2.1`；没有改写第三方依赖的同名版本值。
- [ ] `release/notes/v0.2.1.md` 包含版本与用户可见变更，并如实说明尚未完成的外部门禁；Git 提交、构建时间和构建机/工具链版本由 stable workflow 的 manifest 与证据附件绑定。
- [ ] 已确认默认产物仅为 NSIS；未把未配置、未测试的 MSI 宣称为支持渠道。
- [ ] 产品名称、identifier、安装范围和升级策略与上一版本兼容。
- [ ] 按 `DATABASE_RECOVERY.md` 完成 v13/v14/v15→v16（`pre-v*-to-v16`）原子升级/在线备份、未来 schema 拒绝、损坏库拒绝、最近备份恢复及恢复后再次升级；保留输入库和结果证据。

## 2. 配置静态检查

- [ ] `bundle.targets` 为 `["nsis"]`。
- [ ] `bundle.windows.nsis.installMode` 为 `currentUser`。
- [ ] `bundle.windows.allowDowngrades` 为 `false`。
- [ ] WebView2 模式为静默 `downloadBootstrapper`，发布说明写明缺少运行时时的联网要求。
- [ ] NSIS 语言包含 `SimpChinese` 和 `English`。
- [ ] 资源目录映射把 `resources/` 安装到 `$RESOURCE/`，从而把 staged 的 `resources/bin/ffmpeg.exe` 安装到 `$RESOURCE/bin/ffmpeg.exe`。
- [ ] 日常 Rust 门禁只依赖受版本控制的资源根/元数据；发布静态检查会另外拒绝未 staged 的 FFmpeg。
- [ ] `check-bundle.ps1` 接收本次构建生成的唯一 `installer.nsi`，并把 `Section Install` 精确绑定到主程序、FFmpeg、FFmpeg 许可文件和 `COMPLIANCE-MANIFEST.json` 声明的全部第三方材料。
- [ ] 完整 `7z.exe` 把最终安装器识别为 NSIS 3 Unicode；manifest 驱动的全部资源与输入逐字节一致。内部 `strict-unsigned` 主程序只允许唯一 Tauri marker 从 UNK 变为 NSS；公开 `authenticode-aware` 主程序还只允许 checksum、security-directory、Align8 零 padding 和 EOF WIN_CERTIFICATE 表发生合法签名变化。
- [ ] 若启用 Authenticode，外部 UNK staging 主程序为 `NotSigned`、内嵌 NSS 主程序和安装器均为 `Valid`，并完成证书表、时间戳和 `signtool` 检查；当前个人发布可跳过此项。
- [ ] 配置中没有占位 publisher、伪造许可证、测试证书、虚构更新公钥或更新端点。
- [ ] 本地 Tauri schema 校验通过。

## 3. 代码质量门禁

- [ ] `npm ci` 成功且只使用 `package-lock.json`。
- [ ] `npm run assets:verify` 通过，42 张 PNG 的完整结构、大小、SHA-256、精确文件集合及负责人声明记录绑定均与清单一致。
- [ ] `npm test` 全部通过。
- [ ] `npm run build` 通过。
- [ ] `cargo fmt --manifest-path src-tauri/Cargo.toml --check` 通过。
- [ ] `cargo clippy --manifest-path src-tauri/Cargo.toml --locked --all-targets --all-features -- -D warnings` 通过。
- [ ] `cargo test --manifest-path src-tauri/Cargo.toml --locked --all-targets --all-features` 通过，手工/ignored 测试另有记录。
- [ ] 扫描新鲜度/终态数量、同源重连/歧义、来源根预览与提交、仅移除索引、本人击杀/死亡时间轴和 schema v16 路径去重定向测试全部通过。
- [ ] 构建使用隔离的 `CARGO_TARGET_DIR`，没有依赖或改写默认 `src-tauri/target`。

## 4. FFmpeg 和第三方材料

- [ ] `src-tauri/resources/bin/ffmpeg.exe` 存在，且不是占位文件、下载错误页或 Git LFS 指针。
- [ ] 二进制架构与目标安装器一致，能够执行并报告预期版本。
- [ ] SHA-256 与受审 manifest/发布记录一致。
- [ ] 已记录原始下载 URL、下载时间、版本和构建参数。
- [ ] 已根据实际启用组件完成许可证审核，没有根据项目名猜测 LGPL/GPL 状态。
- [ ] SBOM 或等价组件清单已生成并归档。
- [ ] npm/Cargo SPDX、FFmpeg 组件快照、第三方声明、许可证全文和 SHA-256 manifest 均由锁文件重新生成；`personal-community-stable` 必须达到 100% 正文覆盖并固定每项 override 的锁文件/VCS/正文哈希，`selectors` 的 MPL-2.0 源码形式链接必须随声明提供。逐项独立法审属于未来严格发行，当前如实列为 advisory。
- [ ] 最小自建 FFmpeg 候选的 checksum/build metadata、12 位 commit、PE 实际 imports 与 objdump 结果已一致，imports 只命中 Windows 系统 DLL allowlist；H.264、HEVC、AV1 三种合成 MP4 均完成受控缩略图烟测，且对应源码包与最终二进制在同一 Release 提供。代表性真实高光视频回归属于未来严格发行加固，不阻止个人社区版。
- [ ] 对应源代码镜像或书面提供方式可长期访问并匹配该二进制。
- [x] 项目自有代码已选择 MIT，根许可证已纳入仓库，并记录当前不另设重复 EULA。
- [ ] 第三方声明和缺失许可文本处理已由锁文件、上游提交与哈希固定并纳入发布材料；`personal-community-stable` 如实披露尚未完成的逐项人工/独立法审，未来严格发行再要求全部批准。

## 5. 内部未签名 RC

- [ ] 使用 `npm run tauri -- build --bundles nsis --ci --no-sign` 生成隔离产物。
- [ ] 安装器文件名、大小和 SHA-256 已记录。
- [ ] 已归档 bundle gate 报告，其中包含 `installer.nsi`、7-Zip 及 manifest 驱动受控解包载荷的 SHA-256；报告边界明确写明没有执行安装器。
- [ ] GitHub run 已保留 30 天的 `vhm-internal-rc-evidence-<run id>-<attempt>` 证据 artifact；其中包含 bundle/payload/installer/toolchain、`public-release-preflight.json`、`public-release-gate.json`、启动烟测以及 SPDX/第三方许可材料。
- [ ] 证据 artifact 不含未签名安装器、staging 主程序或受控解包载荷；仅包含报告和许可/SBOM 文本，run URL、run attempt 和 artifact 到期日已写入 RC 记录。
- [ ] `installer-sha256.json` 明确记录 `internal-only`、`unsigned`、安装器与 staging 主程序的 `NotSigned` 状态和 SHA-256；`toolchain-metadata.json` 的 commit/run 和工具版本与本次构建一致。
- [ ] `public-release-gate.json` 为 `blocked-as-required`，公开模式没有传入 `-AllowUnsignedInternalRc`；意外通过、已签名输入或非预期错误均使 workflow 失败，不能把内部证据 artifact 当作公开发布批准。
- [ ] RC 中的游戏素材集合指纹为 `26c4c77a5a13d3ca1a84f4616b0cba1f251462882a0e86f9592d5fc8ef2e1c13`，并记录负责人声明 ID `game-content-rights-owner-attestation-2026-07-20`；人工确认本次测试通道与参与主体符合授权原件，且记录不把操作假设当成批准。
- [ ] 输出目录移动前后均复核临时根目录/父链无 reparse，移动后文件数量和每项哈希再次匹配最终报告；自动启动烟测读取该 JSON 逐项复核，并以报告的 `rawEmbeddedSha256` 授权 NSS 主程序，不现场自算哈希。
- [ ] 分发说明醒目标注“内部、未签名、可能触发 SmartScreen”。
- [ ] 仅在仓库负责人或同一法律主体控制的设备上测试；FFmpeg/第三方许可未闭合时没有向主体外测试者分发，也没有公开上传或接入自动更新。
- [ ] 在标准用户干净虚拟机完成安装、启动、缩略图生成和卸载。
- [ ] 确认安装目录内存在 `bin/ffmpeg.exe`，运行时不依赖系统 `PATH`。
- [ ] 在 WebView2 已存在和不存在两种镜像上验证；后者验证下载成功及断网失败提示。
- [ ] 记录安装/运行日志、截图和已知问题。

### 烟测隔离安全门

- [ ] 没有把覆盖 `APPDATA`/`LOCALAPPDATA` 当作隔离；Windows Known Folder 可能忽略这些子进程环境变量。
- [ ] `VHM_RELEASE_SMOKE_ROOT` 是已存在的绝对路径，规范化后的末级目录名以 `vhm-release-smoke-` 开头。
- [ ] 根目录本身不是 reparse point，且启动前只含一个普通 `.vhm-release-smoke-root` 文件，内容精确为 `vhm-release-smoke-root-v1`。
- [ ] 烟测根目录与真实应用 data/cache 目录互不等同且互不为父子目录。
- [ ] 配置窗口不会自动创建；验证 smoke root 后才手动创建，WebView2 data directory 位于 `<root>/webview2`。
- [ ] 烟测数据库位于 `<root>/data`，缩略图位于 `<root>/cache/thumbnails`；只有 `TEMP`/`TMP`、进程日志和哨兵路径使用 root 外的 marker-gated sibling。
- [ ] 子进程不伪造 `APPDATA`、`LOCALAPPDATA`、`USERPROFILE`、`HOME`、`HOMEDRIVE`、`HOMEPATH`；脚本拒绝继承外部 `WEBVIEW2_USER_DATA_FOLDER`，随后为首、次实例统一注入受控的 `<root>/webview2`，并与 Rust builder 的显式路径及烟测报告逐项核对。
- [ ] 主进程 suspended 创建并在 resume 前加入 `KILL_ON_JOB_CLOSE` Job；stdout/stderr 可诊断，失败清理后 Job 内无遗留进程。
- [ ] 只选择隐藏前主 PID 唯一可见的真实顶层窗口；记录 HWND/PID/title/class，`WM_CLOSE` 后窗口消失、主进程 exit 0、Job active process 为 0。
- [ ] SQLite 只读检查在 `quick_check` 前注册与 Rust 一致的 `VHM_CLIP_NAME` collation，并验证 schema v16、16 张必需表、clips 三段稳定身份、`clip_events.killed_is_me`、`scan_runs.summary_available`、空的 `clip_trash_snapshots`/`clip_delete_intents`、`trashSnapshotCount = 0`、`deleteIntentCount = 0`、空自定义标签目录及空扫描/素材计数。
- [ ] 第二实例使用与首实例相同的 smoke root；首窗口在交接前被最小化，`runtime.singleInstance.verified = true`、`secondInstanceExitCode = 0`、`secondInstanceJobActiveProcessesAfterExit = 0`，且 `onlyPrimaryNamedRootAfterHandoff`、`primaryWindowMinimizedBeforeHandoff`、`primaryWindowHandlePreserved`、`primaryWindowVisibleAfterHandoff` 均为 `true`，`primaryWindowMinimizedAfterHandoff = false`。报告保留首/次 PID 与进程/窗口清单，并把 `focusVerification` 作为 best-effort 证据而不是硬通过条件。
- [ ] 非法相对路径、错误目录名、缺失/错误 marker、路径重叠和符号链接场景均会 fail closed。
- [ ] 清理逻辑只删除自己创建并重新通过名称、marker 和路径边界校验的目录，不使用通配符删除。
- [ ] 烟测前后核对真实应用 data/cache/WebView2 profile，确认未创建、未写入、未删除真实用户数据。

完成本节后，产物可用于内部 RC；稳定 updater 发布还必须完成下一节。

## 6. 个人开发者稳定 updater 必做项

- [ ] `package.json`/lockfile、Cargo manifest/lockfile 与 `tauri.conf.json` 版本完全一致，tag 精确为 `vMAJOR.MINOR.PATCH` 且高于现有稳定版本。
- [ ] GitHub 仓库 Secrets 已配置 `TAURI_SIGNING_PRIVATE_KEY`、`TAURI_SIGNING_PRIVATE_KEY_PASSWORD`；Variable 已配置 `VALOFRAME_UPDATER_PUBLIC_KEY`。仓库、日志和 Release 中没有私钥。
- [ ] 私钥和密码已有可读取的加密离线备份；没有把“计划备份”误勾为已经完成。
- [ ] Tauri 正确签名通过，单字节篡改和错误公钥均失败；下载 URL 绑定 `2424521842/valoframe` 的对应 tag，ZIP 内安装器版本一致且包不超过 512 MiB。
- [ ] workflow 创建 draft，重新下载验证安装器、`.nsis.zip`、`.sig` 和 `latest.json` 后才发布；公开后二次验证失败能恢复为 draft。
- [ ] `v0.2.1` 由用户手动安装；发布 `v0.2.2` 后完成发现、下载、验签、确认安装和重启验收。
- [ ] 覆盖离线、取消、重试、篡改包、错误公钥、安装器启动失败和禁止降级，确认旧安装和用户数据保持可用。
- [ ] 未启用 Authenticode 时，下载页和发布说明明确提示 Windows 可能显示“未知发布者”或 SmartScreen。

## 7. 未来严格公开发行检查（可选）

以下历史 policy 条目用于商业化、扩大分发和代码签名等未来加固，不阻止当前个人 updater 发布，也不得被虚构为已完成：

- [ ] 真实法律主体/publisher 已确定，并与证书签名主体一致。
- [ ] 稳定 identifier 已审核并冻结；对既有安装的升级影响已验证。
- [x] 项目自有代码的 MIT 许可证及“不另设重复 EULA”决定已经负责人批准。
- [ ] 第三方声明、许可证全文缺口和相关合规结论已经负责人/法律批准。
- [ ] 默认 Tauri/Vite 图标及其他占位品牌资产已替换为获批的正式资产。
- [ ] “瓦刻 / VALOFRAME”名称的商标检索、可用性与分发区域已由负责人/法律确认；应用、安装器、网站和商店文案使用同一批准名称。
- [x] 仓库负责人已确认有权将 `VALOFRAME_正规图标格式包.zip` 及其派生资产复制、修改并发布到公开 GitHub 仓库；确认日期与 SHA-256 见 `src-tauri/icons/README.md`。
- [ ] 安装器、应用商店或商业分发所需的设计源文件、合同或许可证已另行归档并通过法律与品牌审核。
- [x] 仓库负责人已声明清单锁定的 42 张游戏图片取得授权；逐文件哈希、集合指纹、待审字段与 verifier 已纳入 policy，但没有把具体渠道写成已核实事实。
- [ ] 人工取得并核对可验证的授权原始证据，确认权利链、被授权主体、地域、期限、公开源码、应用内展示、GitHub 宣传、安装器和 Release 下载等实际渠道，并把审阅结论、manifest 和哈希重新固定到严格 policy；完成前不得宣称未来严格企业式发行已获第三方或独立法律批准。版本级负责人决定可以授权 `personal-community-stable` 精确渠道，但不产生上述第三方批准声明。
- [ ] Riot Games、腾讯及《无畏契约》相关商标归属和非官方/非赞助/非认可声明已经批准，并纳入 README、About/许可页和公开发布材料。
- [ ] FFmpeg provenance、SHA-256、许可证结论、SBOM 和源代码镜像/提供方式齐全。
- [ ] Authenticode 证书和可信时间戳服务已配置；policy 固定 publisher subject 和证书 thumbprint，最终安装器与内嵌主程序的链、时间戳和 `signtool` 验证成功。
- [ ] 若选择 Authenticode，发布流程不会输出或上传未完成代码签名的安装器。
- [x] updater Rust 服务、最小 capability、设置/关于 UI、主导航可发现状态、每日/焦点恢复/手动检查、进度/验签/取消、失败直重试与显式放弃已实现；关键任务门禁覆盖扫描、永久删除、视频导出和来源根重定位；运行时严格绑定规范版本、固定 GitHub HTTPS 仓库/tag 与已验签 ZIP 内安装器版本，限制下载/解压大小，并在 Windows 安装器启动失败时保留应用和已验证包。
- [x] 稳定 updater workflow 在 draft 阶段证明正确签名通过、单字节篡改和错误公钥失败；Community Beta 继续拒绝公钥、`.sig` 和 `latest.json`。
- [ ] 扩展 Windows 10/11、DPI、真实 NVIDIA/Tracker、来源恢复、索引清理、时间轴和数据安全证据矩阵。
- [ ] 支持的 Windows 版本、CPU 架构、WebView2 在线/离线策略已公布。
- [ ] 从每个受支持的上一公开版本完成真实升级测试。
- [ ] 降级被正确拒绝，且“发布更高补丁版本”的回退演练完成。
- [ ] 安装、升级、卸载均不会修改或删除用户原始视频；仅应用内回收站的显式永久删除会删除所选视频，数据库保留策略符合文档。

这些可选条目未完成时不得宣称“严格企业级发行已验收”，但个人开发者稳定 updater 可以在第 6 节全部通过后发布。

## 8. 虚拟机发布矩阵

| 场景 | 最低要求 | 结果/证据 |
| --- | --- | --- |
| 干净安装 | 标准用户、WebView2 已安装 | |
| 引导安装 WebView2 | 标准用户、WebView2 缺失、允许联网 | |
| WebView2 下载失败 | WebView2 缺失、禁止联网 | |
| 同版本重装 | 已安装同版本 | |
| 正常升级 | 每个受支持的上一公开版本 | |
| v0.2.1 数据迁移 | v13/v14/v15→v16；备份、clip ID/用户状态和删除安全数据保持 | |
| 来源恢复 | 同源子目录改名自动重连；用户预览/提交新根；reparse、重叠、歧义与关键任务冲突失败关闭 | |
| 索引清理 | missing/来源不可用素材单条与混合批量；原视频大小、mtime、身份不变 | |
| 时间轴 | 真实形状 ACLOS fixture；本人击杀/死亡图标、数量、tooltip、无障碍名称、跳转和越界隐藏 | |
| 禁止降级 | 当前版本安装后运行更低版本安装器 | |
| 卸载 | 验证安装文件与用户数据策略 | |
| FFmpeg 运行时 | 无系统 FFmpeg，仅使用 `$RESOURCE/bin/ffmpeg.exe` | |
| 用户素材安全 | 安装、升级、卸载前后校验原始素材 | |

## 9. 最终产物与发布后检查

- [ ] 最终 NSIS 安装器的 SHA-256、大小和签名状态已二次核对。
- [ ] 若启用 Authenticode，其链和时间戳在干净机器上验证通过；未启用时已验证并披露未知发布者提示。
- [ ] 发布说明列出系统要求、WebView2 联网行为、数据迁移和已知问题。
- [ ] 安装器、符号/调试材料、SBOM、源代码材料、第三方声明、游戏素材 manifest、可核验的人工范围审阅记录和测试证据已归档；授权原件如含敏感信息，只留在受控法律档案。
- [ ] 下载页和更新元数据指向同一已验证文件及版本。
- [ ] `latest.json` 只含 `windows-x86_64` 稳定版本，URL 指向同一非 draft、非 prerelease Release 的已验证 `.nsis.zip`，签名与下载字节重新验证通过。
- [ ] 发布后从公开入口重新下载并校验哈希、签名、安装和首次启动。
- [ ] 已准备撤回发布、关闭更新端点和发布更高补丁版本的响应流程。
