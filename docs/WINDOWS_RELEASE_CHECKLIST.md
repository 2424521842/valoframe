# Windows 发布检查清单

本清单分为“内部未签名 RC”和“公开发布”。内部 RC 的通过不等于公开发布获准。

## 1. 变更范围和版本

- [ ] 发布工作区干净，或所有未提交改动均已逐项审阅并记录。
- [ ] `package.json`、`src-tauri/Cargo.toml`、`src-tauri/tauri.conf.json` 版本一致。
- [ ] 发布说明包含 Git 提交、版本、构建时间和构建机/工具链版本。
- [ ] 已确认默认产物仅为 NSIS；未把未配置、未测试的 MSI 宣称为支持渠道。
- [ ] 产品名称、identifier、安装范围和升级策略与上一版本兼容。
- [ ] 按 `DATABASE_RECOVERY.md` 完成旧库原子升级/在线备份、未来 schema 拒绝、损坏库拒绝、最近备份恢复及恢复后再次升级；保留输入库和结果证据。

## 2. 配置静态检查

- [ ] `bundle.targets` 为 `["nsis"]`。
- [ ] `bundle.windows.nsis.installMode` 为 `currentUser`。
- [ ] `bundle.windows.allowDowngrades` 为 `false`。
- [ ] WebView2 模式为静默 `downloadBootstrapper`，发布说明写明缺少运行时时的联网要求。
- [ ] NSIS 语言包含 `SimpChinese` 和 `English`。
- [ ] 资源目录映射把 `resources/` 安装到 `$RESOURCE/`，从而把 staged 的 `resources/bin/ffmpeg.exe` 安装到 `$RESOURCE/bin/ffmpeg.exe`。
- [ ] 日常 Rust 门禁只依赖受版本控制的资源根/元数据；发布静态检查会另外拒绝未 staged 的 FFmpeg。
- [ ] `check-bundle.ps1` 接收本次构建生成的唯一 `installer.nsi`，并把 `Section Install` 精确绑定到主程序、FFmpeg 和四个合规文件。
- [ ] 完整 `7z.exe` 把最终安装器识别为 NSIS 3 Unicode；五个资源与输入逐字节一致。内部 `strict-unsigned` 主程序只允许唯一 Tauri marker 从 UNK 变为 NSS；公开 `authenticode-aware` 主程序还只允许 checksum、security-directory、Align8 零 padding 和 EOF WIN_CERTIFICATE 表发生合法签名变化。
- [ ] 公开模式要求外部 UNK staging 主程序为 `NotSigned`、内嵌 NSS 主程序和安装器均为 `Valid`；证书表边界、大小、条目和 EOF 覆盖有效。报告包含 embedded main 的 raw/canonical SHA-256、比较模式和签名状态。
- [ ] 配置中没有占位 publisher、伪造许可证、测试证书、虚构更新公钥或更新端点。
- [ ] 本地 Tauri schema 校验通过。

## 3. 代码质量门禁

- [ ] `npm ci` 成功且只使用 `package-lock.json`。
- [ ] `npm test` 全部通过。
- [ ] `npm run build` 通过。
- [ ] `cargo fmt --manifest-path src-tauri/Cargo.toml --check` 通过。
- [ ] `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings` 通过。
- [ ] `cargo test --manifest-path src-tauri/Cargo.toml` 通过，手工/ignored 测试另有记录。
- [ ] 构建使用隔离的 `CARGO_TARGET_DIR`，没有依赖或改写默认 `src-tauri/target`。

## 4. FFmpeg 和第三方材料

- [ ] `src-tauri/resources/bin/ffmpeg.exe` 存在，且不是占位文件、下载错误页或 Git LFS 指针。
- [ ] 二进制架构与目标安装器一致，能够执行并报告预期版本。
- [ ] SHA-256 与受审 manifest/发布记录一致。
- [ ] 已记录原始下载 URL、下载时间、版本和构建参数。
- [ ] 已根据实际启用组件完成许可证审核，没有根据项目名猜测 LGPL/GPL 状态。
- [ ] SBOM 或等价组件清单已生成并归档。
- [ ] 对应源代码镜像或书面提供方式可长期访问并匹配该二进制。
- [ ] 项目许可证/EULA（若需要）和第三方声明已获批准并纳入发布材料。

## 5. 内部未签名 RC

- [ ] 使用 `npm run tauri -- build --bundles nsis --ci --no-sign` 生成隔离产物。
- [ ] 安装器文件名、大小和 SHA-256 已记录。
- [ ] 已归档 bundle gate 报告，其中包含 `installer.nsi`、7-Zip 及六个受控解包载荷的 SHA-256；报告边界明确写明没有执行安装器。
- [ ] GitHub run 已保留 30 天的 `vhm-internal-rc-evidence-<run id>-<attempt>` 证据 artifact；其中包含 `vhm-internal-rc-report.json`、`verified-payload-sha256.json`、`installer-sha256.json`、`toolchain-metadata.json`、`public-release-gate.json` 和通过后的 `vhm-release-smoke-report.json`。
- [ ] 证据 artifact 只含 JSON 元数据，不含未签名安装器、staging 主程序或六个解包载荷；run URL、run attempt 和 artifact 到期日已写入 RC 记录。
- [ ] `installer-sha256.json` 明确记录 `internal-only`、`unsigned`、安装器与 staging 主程序的 `NotSigned` 状态和 SHA-256；`toolchain-metadata.json` 的 commit/run 和工具版本与本次构建一致。
- [ ] `public-release-gate.json` 为 `blocked-as-required`，公开模式没有传入 `-AllowUnsignedInternalRc`；意外通过、已签名输入或非预期错误均使 workflow 失败，不能把内部证据 artifact 当作公开发布批准。
- [ ] 输出目录移动前后均复核临时根目录/父链无 reparse，移动后恰好六个文件再次匹配最终报告；自动启动烟测读取该 JSON 逐项复核，并以报告的 `rawEmbeddedSha256` 授权 NSS 主程序，不现场自算哈希。
- [ ] 分发说明醒目标注“内部、未签名、可能触发 SmartScreen”。
- [ ] 仅在仓库负责人或同一法律主体控制的设备上测试；FFmpeg/项目许可未闭合时没有向主体外测试者分发，也没有公开上传或接入自动更新。
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
- [ ] 子进程不伪造 `APPDATA`、`LOCALAPPDATA`、`USERPROFILE`、`HOME`、`HOMEDRIVE`、`HOMEPATH`，也不以 `WEBVIEW2_USER_DATA_FOLDER` 覆盖 Rust builder 的显式路径。
- [ ] 主进程 suspended 创建并在 resume 前加入 `KILL_ON_JOB_CLOSE` Job；stdout/stderr 可诊断，失败清理后 Job 内无遗留进程。
- [ ] 只选择隐藏前主 PID 唯一可见的真实顶层窗口；记录 HWND/PID/title/class，`WM_CLOSE` 后窗口消失、主进程 exit 0、Job active process 为 0。
- [ ] SQLite 只读检查在 `quick_check` 前注册与 Rust 一致的 `VHM_CLIP_NAME` collation，并验证 schema v13、16 张必需表（含 `clip_trash_snapshots` 与 `clip_delete_intents`）、`trashSnapshotCount = 0`、`deleteIntentCount = 0`、空自定义标签目录及空扫描/素材计数。
- [ ] 第二实例使用与首实例相同的 smoke root；首窗口在交接前被最小化，`runtime.singleInstance.verified = true`、`secondInstanceExitCode = 0`、`secondInstanceJobActiveProcessesAfterExit = 0`，且 `onlyPrimaryNamedRootAfterHandoff`、`primaryWindowMinimizedBeforeHandoff`、`primaryWindowHandlePreserved`、`primaryWindowVisibleAfterHandoff` 均为 `true`，`primaryWindowMinimizedAfterHandoff = false`。报告保留首/次 PID 与进程/窗口清单，并把 `focusVerification` 作为 best-effort 证据而不是硬通过条件。
- [ ] 非法相对路径、错误目录名、缺失/错误 marker、路径重叠和符号链接场景均会 fail closed。
- [ ] 清理逻辑只删除自己创建并重新通过名称、marker 和路径边界校验的目录，不使用通配符删除。
- [ ] 烟测前后核对真实应用 data/cache/WebView2 profile，确认未创建、未写入、未删除真实用户数据。

完成本节后，产物最多可标记为“内部 RC”，不能勾选“公开发布批准”。

## 6. 公开发布阻断项

- [ ] 真实法律主体/publisher 已确定，并与证书签名主体一致。
- [ ] 稳定 identifier 已审核并冻结；对既有安装的升级影响已验证。
- [ ] 项目许可证、EULA（若需要）和第三方声明已经法律/负责人批准。
- [ ] 默认 Tauri/Vite 图标及其他占位品牌资产已替换为获批的正式资产。
- [ ] “瓦刻 / VALOFRAME”名称的商标检索、可用性与分发区域已由负责人/法律确认；应用、安装器、网站和商店文案使用同一批准名称。
- [x] 仓库负责人已确认有权将 `VALOFRAME_正规图标格式包.zip` 及其派生资产复制、修改并发布到公开 GitHub 仓库；确认日期与 SHA-256 见 `src-tauri/icons/README.md`。
- [ ] 安装器、应用商店或商业分发所需的设计源文件、合同或许可证已另行归档并通过法律与品牌审核。
- [ ] Riot Games、腾讯及《无畏契约》相关商标归属和非官方/非赞助/非认可声明已经批准，并纳入 README、About/许可页和公开发布材料。
- [ ] FFmpeg provenance、SHA-256、许可证结论、SBOM 和源代码镜像/提供方式齐全。
- [ ] Authenticode 证书和可信时间戳服务已配置；最终安装器签名验证成功。
- [ ] 发布流程不会输出或上传未签名的公开安装器。
- [ ] updater 插件、最小权限、公钥、HTTPS endpoint 和 `latest.json` 契约已实现；若本版本不提供自动更新，则所有公开材料明确写明不支持。
- [ ] updater 私钥已安全备份并仅通过发布机密注入；篡改包和错误签名会被拒绝。
- [ ] 支持的 Windows 版本、CPU 架构、WebView2 在线/离线策略已公布。
- [ ] 从每个受支持的上一公开版本完成真实升级测试。
- [ ] 降级被正确拒绝，且“发布更高补丁版本”的回退演练完成。
- [ ] 安装、升级、卸载均不会修改或删除用户原始视频；仅应用内回收站的显式永久删除会删除所选视频，数据库保留策略符合文档。

任一项未完成，公开发布必须停止。

## 7. 虚拟机发布矩阵

| 场景 | 最低要求 | 结果/证据 |
| --- | --- | --- |
| 干净安装 | 标准用户、WebView2 已安装 | |
| 引导安装 WebView2 | 标准用户、WebView2 缺失、允许联网 | |
| WebView2 下载失败 | WebView2 缺失、禁止联网 | |
| 同版本重装 | 已安装同版本 | |
| 正常升级 | 每个受支持的上一公开版本 | |
| 禁止降级 | 当前版本安装后运行更低版本安装器 | |
| 卸载 | 验证安装文件与用户数据策略 | |
| FFmpeg 运行时 | 无系统 FFmpeg，仅使用 `$RESOURCE/bin/ffmpeg.exe` | |
| 用户素材安全 | 安装、升级、卸载前后校验原始素材 | |

## 8. 最终产物与发布后检查

- [ ] 最终 NSIS 安装器的 SHA-256、大小和签名状态已二次核对。
- [ ] Authenticode 链和时间戳在干净机器上验证通过。
- [ ] 发布说明列出系统要求、WebView2 联网行为、数据迁移和已知问题。
- [ ] 安装器、符号/调试材料、SBOM、源代码材料、第三方声明和测试证据已归档。
- [ ] 下载页和更新元数据指向同一已验证文件及版本。
- [ ] 发布后从公开入口重新下载并校验哈希、签名、安装和首次启动。
- [ ] 已准备撤回发布、关闭更新端点和发布更高补丁版本的响应流程。
