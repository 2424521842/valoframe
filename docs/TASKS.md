# Tasks

> 更新于 2026-08-10。当前发布路线为：手动安装 `v0.2.1`，再以 `v0.2.2` 验证第一次应用内更新。旧版本计划仅作历史参考。

状态说明：

- ✅ 已完成：生产路径已接入并有自动化验证。
- 🟡 部分完成：核心能力已落地，但仍有明确收尾项。
- ⏳ 待执行：尚未进入生产实现。

## 0. 状态总览

| 任务 | 状态 | 当前结论 |
| --- | --- | --- |
| npm 与基础构建 | ✅ 已完成 | npm 为唯一包管理器；Node 测试、Vitest UI 测试和生产构建可运行 |
| SQLite schema 与迁移 | ✅ 代码完成 | schema v16；v13/v14/v15 安全迁移、来源/审核模型、稳定文件身份、Windows 路径别名去重、本人死亡事件、扫描摘要可用性、回收身份快照、永久删除 outbox、未来版本拒绝、专用在线备份与原子升级 |
| 数据库连接生命周期 | ✅ 已完成 | 启动时健康检查/迁移/中断恢复；普通读写连接不迁移；查询与媒体使用只读连接 |
| 默认、自定义和固定磁盘发现 | ✅ 已完成 | 支持默认 ACLOS、自定义目录、videocut 日志根和固定磁盘验证发现 |
| 统一多根扫描 | ✅ 已完成 | `scan_roots` 一次处理全部来源，共享一次元数据快照和一个扫描批次 |
| 扫描互斥、反馈与取消 | ✅ 已完成 | 后台任务、job ID、already-running、事件、取消、按 job 摘要恢复、真实新增数量和 completed-only 来源新鲜度已接入 |
| 四路元数据采集 | ✅ 已完成 | WonderfulDb、导出 JSON、highlight.log、LevelDB 容错合并 |
| 稳定账号身份与来源 DTO | ✅ 已完成 | 稳定 ID 分组；`list_sources` 返回来源类型、扫描模式/根、路径、状态、错误、数量和最后一次完整成功扫描时间 |
| 媒体协议 | ✅ 已完成 | 只读查询视频路径，Range/HEAD 支持，单响应最多 1 MiB |
| 分页列表与按需详情 | ✅ 已完成 | `list_clip_page` + `get_clip_detail` 已进入生产前端 |
| 全库 facets | ✅ 已完成 | 聚合不再依赖已加载摘要页 |
| 真实虚拟列表 | ✅ 已完成 | 顶层对局组与极端单组视觉行分层虚拟化、动态测量和有界 DOM |
| 收藏、标签、备注 | ✅ 已完成 | 单素材操作持久化，备注保存不重载媒体 |
| 批量收藏、标签、回收 | ✅ 已完成 | 单 command、单事务，前端按局部结果同步 |
| 前端 controller 拆分 | ✅ 已完成 | 扫描、分页、详情、facets、标签和 mutation hooks 已拆出 |
| Rust 模块收敛 | ✅ 已完成 | library/reconnect/relocation repositories、共享文件身份、scan-run persistence、library/source commands、migrations、media protocol 与 scan service 已拆分；兼容 facade 仅保留聚合入口与本域实现 |
| 旧 UI/CSS/大资源清理 | ✅ 已完成 | 旧生产组件和 App.css 覆盖层已删除；workspace 已懒加载；大型参考图已移除；主 JS 约 388 kB |
| 缩略图队列 | ✅ 已完成 | 源封面优先；持久队列、单 worker、指纹失效、缓存预算、局部前端更新和生成器不可用降级已接入 |
| CI | ✅ 已完成 | `.github/workflows/ci.yml` 已覆盖 npm 与 Rust 门禁，Rust toolchain 固定为 1.96.1；Draft PR #1 的 CI run 11 已通过 |
| CSP 与权限最小化 | 🟡 部分完成 | CSP、依赖和 capability 已收敛；仍需 release build 实机回归目录选择、事件与媒体 |
| 批量备注 | ⏳ 待执行 | 仍需补充批量备注和更完整的失败恢复交互 |
| v0.2.1 可靠性功能 | ✅ 代码完成 | 扫描新鲜度/终态数量、同源重连、根恢复、仅移除索引、本人击杀/死亡时间轴已接入；仍需 M7 全量与实机证据 |
| 应用内更新工程 | ✅ 已完成 | v0.2.0 M5 设置/关于、每日/手动检查、下载进度/取消、确认安装、含来源重定位的关键任务门禁、签名 verifier 与稳定 draft→latest workflow 已落地 |
| 正式安装与发布 | 🟡 待配置 | 稳定 updater 工程已实现；仍需配置真实 Tauri 密钥、离线备份并完成 v0.2.1→v0.2.2 首次 OTA。Authenticode 与严格 policy 改为未来可选加固 |

## 1. 已完成：扫描与数据层

### 1.1 数据库

- [x] 使用 `PRAGMA user_version` 管理 schema v16。
- [x] 保留 v13→v15 的历史迁移，并支持 v13/v14/v15→v16 原子升级；生成对应 `pre-v*-to-v16` 在线备份。
- [x] v15 增加全空/全非空的 Windows 稳定文件身份、`clip_events.killed_is_me`、`scan_runs.summary_available` 和非唯一匹配索引。
- [x] v15 只回填可由 `raw_json`/duration 确定的本人死亡和集锦时间；不确定值安全置空等待 ACLOS 重扫，并保留 clip ID/全部用户状态和删除授权数据。
- [x] v16 统一普通路径与 `\\?\`/`\\?\UNC\` 路径键；只合并身份和去前缀路径都唯一确定的重复索引，授权歧义保守跳过。
- [x] 启动路径通过 `migrate_database` 创建或升级数据库。
- [x] 升级前通过 SQLite Online Backup API 生成并验证 WAL 安全备份，保留最近 3 份；未来 schema、损坏库和外键损坏均失败关闭。
- [x] schema 变更和版本号在同一 `BEGIN IMMEDIATE` 事务提交；启动失败显示恢复提示并保留数据库。
- [x] `open_database` 只建立普通读写连接，不执行 schema 初始化。
- [x] `open_database_read_only` 用于列表、详情、facets、来源和媒体路径查询。
- [x] 迁移包含 modified/id、size/id、自然名称/id 和 tag/clip 复合索引。
- [x] 标签目录只保存用户创建的标签；v10 清理旧自动标签并回填视频类型。
- [x] migrations、models 和 repositories 已从主数据库 facade 中拆出。
- [x] 永久删除先提交 `clip_delete_intents`，再通过 Windows 句柄身份校验删除目标，并在启动时幂等恢复；待删除记录阻止普通恢复和仅移除索引。

### 1.2 扫描来源与批次

- [x] 发现默认 `AppData\ACLOS\aclos-highlight`。
- [x] 从 videocut 日志补充实际存储根。
- [x] 支持自定义目录和固定磁盘只读发现。
- [x] 提供 `scan_roots(paths)`，一次接收全部真实来源路径。
- [x] 支持来源根目录直接 MP4 和“来源/对局/MP4”两种结构。
- [x] 多来源扫描只创建一个 scan run，并只读取一次共享元数据。
- [x] 不可访问来源记录状态和错误，不误标历史素材 missing。
- [x] 扫描后保留收藏、标签和备注。
- [x] 来源向导支持 ACLOS、NVIDIA、Tracker、generic 的显示名、目录授权、是否加入自动同步及重复/重叠确认。
- [x] `recursive-mp4` 流式递归大小写不敏感的 MP4，跳过 reparse point/越界路径，以大小和 mtime 稳定性延迟仍在录制的文件，并分批提交索引。
- [x] 全局“启动时自动扫描”默认关闭；开启后下次启动非阻塞同步全部已加入来源，扫描页支持单来源和全部来源手动同步；所有入口复用同一个 job、10 分钟重启冷却、全局互斥、进度和取消协议。
- [x] 完整同步才标记 missing；offline、partial、cancelled 保留历史素材状态，同一规范化视频路径不会被第二来源重复认领。
- [x] ACLOS 与 recursive 都先完成来源级 TEMP 候选统计，再按 path→双侧唯一稳定身份→身份全空旧行的双侧唯一旧指纹重连；hardlink、复制和重复指纹不合并。
- [x] 稳定身份读取失败继续正常索引且不单独置 partial；旧路径只有明确 `NotFound` 才能原地重连，权限/共享错误失败关闭。
- [x] 来源根重新定位提供只读预览和事务提交，拒绝 reparse/越界/重叠/零可信匹配、回收素材、删除 intent 与关键任务冲突；提交后同步成功才刷新新鲜度。

### 1.3 扫描任务协议

- [x] 扫描运行于后台阻塞任务。
- [x] 全局协调器禁止并发扫描，并返回结构化 `already-running`。
- [x] job ID、进度事件、查询状态和取消命令已接入。
- [x] completed、partial、cancelled 和 failed 终态持久化。
- [x] worker 错误或 panic 会兜底结束 scan run。
- [x] `get_scan_summary(jobId)` 能恢复漏失事件的真实摘要；`summary_available` 区分真实新增 0 与数量不可用，终态通知按 job ID 去重。
- [x] 只有 completed 完整扫描刷新来源 `lastScanAt`；扫描页按本地自然日显示首次/今天/N 天，7 天及从未扫描的启用来源进入全局提醒，停用来源排除。

### 1.4 元数据

- [x] 弱解析 `videoExportTmp/config-*.json`。
- [x] 容错读取 Local Storage LevelDB battle list。
- [x] 解析 `highlight.log` 明文对局数据。
- [x] 只读读取 WonderfulDb，内存解密并按账号隔离 warning。
- [x] 合并 `matches`、`match_stats` 和 `match_events`。
- [x] 写入官方 video、`clip_segments` 和 clip-scoped `clip_events`。
- [x] WonderfulDb 权威字段不会被低质量回退覆盖。
- [x] 解析 `KilledIsMe` 并区分本人击杀/本人死亡；普通高光使用 segment+event，相应击杀/死亡集锦使用绝对 event 时间，越界事件不裁剪且不渲染。

## 2. 已完成：素材库与详情

### 2.1 分页列表

- [x] `list_clip_page` 默认 limit 50，允许范围 1..200，offset 非负。
- [x] 支持搜索、稳定账号、来源、英雄、地图、模式、自定义标签、视频类型、收藏、文件/元数据状态、日期和大小过滤。
- [x] 支持修改时间升降序、大小升降序和自然文件名排序，并使用 ID 稳定打破并列。
- [x] count、当前页和页内标签在同一只读快照中读取。
- [x] 标签一次批量加载，无 N+1。
- [x] `ClipSummary` 不返回详情专用的备注、OCR、raw JSON 或事件。
- [x] legacy `list_clips` 不再被生产素材库调用。

### 2.2 全库聚合

- [x] `get_library_facets` 返回全库与活跃素材统计。
- [x] 账号、来源、自定义标签、英雄、地图、模式、视频类型和状态计数来自数据库全量快照。
- [x] facets 不查询事件或 raw metadata，不从前端已加载页推导。

### 2.3 按需详情

- [x] 选中素材后才调用 `get_clip_detail`。
- [x] 返回完整 Clip、完整 Tag 对象和 clip event。
- [x] 素材不存在时返回稳定的 `clip-not-found` 结构化错误。
- [x] request token 忽略快速切换和关闭后的旧响应。
- [x] 详情使用有界 LRU 缓存，扫描和显式刷新会失效。

### 2.4 虚拟化与交互

- [x] 使用 `@tanstack/react-virtual` 虚拟化顶层对局组。
- [x] 动态测量组高度并监听尺寸变化。
- [x] 单个对局超过 48 条素材时复用外层滚动容器按视觉行二级虚拟化，不创建嵌套滚动区。
- [x] 组内列数由真实内容宽度和视图模式推导；跨页追加、折叠、宽度及 grid/list 变化会失效旧高度缓存。
- [x] 临近底部自动加载下一页，同时保留键盘可用的加载和重试按钮。
- [x] 跨页使用稳定账号 + 对局 key 合并，不重复标题。
- [x] 卡片键盘事件不会由内层复选框或收藏按钮冒泡触发预览。
- [x] 卡片显示 WonderfulDb 单视频 `roundScore`，或将日志按 GameID 分局、按 OpenID 绑定同账号后，经唯一结算总分、可用玩家身份和连续回合共同校验的累计差；只恢复已观测回合，不推算缺失尾局。应计分但缺失时显示“官方未同步”，不计分类型不显示占位，且不使用对局级 `combatScore`。
- [x] 备注草稿同步与媒体加载分离，保存备注不重置播放。

## 3. 已完成：用户整理

- [x] 创建、重命名、改色和删除标签。
- [x] 单素材收藏、标签绑定/解绑、备注和回收状态。
- [x] 批量收藏/取消收藏。
- [x] 批量添加/移除标签。
- [x] 批量移入/移出回收区。
- [x] 回收站单条/批量永久删除本地视频，包含二次确认、状态校验和局部失败反馈。
- [x] 普通库对 missing/来源不可用且无删除 intent 的素材开放单条/批量仅移除索引；逐项返回结果，删除应用索引状态但不触碰磁盘文件。
- [x] 批量后端操作使用单事务，并返回明确的局部结果。
- [x] mutation 同步分页摘要、当前详情、详情缓存和全库 facets。

## 4. 工程与发布收尾

### 4.1 CI（实现完成）

- [x] GitHub Actions 在 Ubuntu 执行前端门禁、在 Windows 执行 Rust 门禁。
- [x] 使用 `npm ci` 安装唯一 npm lockfile。
- [x] 执行 Node 与 Vitest UI 测试。
- [x] 执行 TypeScript/Vite 生产构建。
- [x] 执行 Rust fmt、严格 Clippy 和完整测试。
- [x] workflow 不读取或上传真实 ACLOS 数据、本地数据库或用户素材。
- [x] Draft PR #1 的 GitHub 托管 CI run 11 已完整通过；后续提交仍需重新通过。

### 4.2 CSP、权限和依赖（代码完成，待实机回归）

- [x] 为生产窗口设置最小 CSP，并为 Vite HMR 单独配置开发 CSP。
- [x] CSP 仅开放必要的本地资源、IPC、自定义 `clip-media` 与英雄资源域名。
- [x] 删除未使用的 `tauri-plugin-opener` Rust/前端依赖。
- [x] 删除 opener capability；主窗口只保留事件 listen/unlisten 和目录选择。
- [x] 统一使用 npm，删除过期 pnpm lock/workspace 文件。
- [ ] 在 production release build 实测目录选择、扫描事件、封面、视频和远程图片。

### 4.3 代码与资源清理

- [x] 拆分过大的 Rust facade：分页/facets 查询、scan-run 持久化和素材库 commands 已迁入独立模块，原公开路径保持兼容且未复制 SQL/实现。
- [x] 删除不在生产入口使用的旧 UI 组件及对应旧测试。
- [x] 将仍使用的样式收敛到 `cinematic.css`，删除 App.css 覆盖层。
- [x] 四个 workspace 使用 `React.lazy` 动态加载，生产入口保持按页面拆包。
- [x] 删除未使用的约 2.07 MB 参考 PNG 和旧 SVG 资源。
- [x] 为极端单对局组增加共享滚动容器的视觉行二级虚拟化，并用单组 1k/10k fixture 验证卡片 DOM 有界。

### 4.4 素材体验

- [x] 使用 SQLite 持久化缩略图队列，单 worker 有界处理并支持启动/扫描终态恢复。
- [x] 缩略图只写应用缓存目录；源封面优先，视频指纹变化使缓存失效，陈旧结果不能提交。
- [x] 缓存达到 512 MiB 后清理到 450 MiB，并修复缺失、损坏、孤立及中断临时文件。
- [x] FFmpeg 只从打包资源或绝对路径环境变量解析，不搜索 `PATH`；不可用时全局稳定降级。
- [x] 前端按页面批量 ensure，ready 事件局部更新列表、详情与 poster，不刷新分页或重载视频。
- [x] 固定 Windows x64 LGPL FFmpeg 的 archive/exe SHA、构建来源和运行能力，并在未签名 NSIS 内部 RC 中验证资源清单。
- [x] 将 29 张英雄图和 13 张地图图固化为逐文件来源/尺寸/大小/SHA-256 清单，记录负责人“已取得授权”的声明、保守操作假设和待人工核对项，并以完整 PNG verifier 接入测试与构建；发布源码/宣传图及所有公开安装器/Release 二进制下载继续 fail closed。
- [x] 自动生成 npm/Cargo SPDX、FFmpeg 组件快照、第三方声明、去重许可证全文和哈希 manifest，并把全部材料纳入 NSIS 逐字节门禁。
- [x] 建立最小自建 FFmpeg 候选构建、精确源码包与 Windows 合成视频验证链；候选默认不可自我晋升。
- [x] 最小 FFmpeg 候选固定 H.264、HEVC、AV1 原生解码器，并用三种合成 MP4 分别验证受控缩略图链；内嵌播放失败时提供重新校验边界的系统默认播放器回退。
- [ ] 发布前晋升最小自建 FFmpeg（或为现有 BtbN 构建补齐对应源码、外部库许可证及 IJG attribution），并完成专利与法律审批；此前禁止主体外分发。

### 4.5 效率与发布

- [ ] 批量备注及失败恢复交互。
- [x] 配置 NSIS current-user 内部 RC、禁止降级、显式 WebView2 策略和简中/英文安装语言。
- [x] 增加 FFmpeg 准备/验证、bundle 静态检查、安全启动烟测和手工 release-readiness workflow；公开模式默认 fail closed。
- [x] 增加整体公开发布 policy/preflight，覆盖许可、游戏素材范围与字节证据、品牌、publisher、签名/时间戳、VM、updater 和数据安全，并输出稳定 blocker code。
- [x] 项目自有代码选择 MIT，npm/Cargo/根许可文本一致，并记录不另设重复 EULA。
- [x] 为发布归档漏带正文的 1 个 npm 和 11 个 Cargo 组件增加版本/checksum/VCS/SHA-256 固定的离线 override 证据。
- [ ] 负责人/法律审核并批准第三方许可证 override，单独确认 `selectors` 的 MPL 2.0 源代码形式义务。
- [x] 加入 Tauri 单实例保护，第二次启动只恢复/显示/聚焦原窗口；发布烟测保留 PID、窗口和 Job 交接证据。
- [ ] （可选加固）使用正式 publisher、品牌和 Authenticode 在一次性 Windows VM 完成安装/升级/卸载。
- [x] 实现固定稳定更新端点、最小权限 Rust service、设置/关于 UI、每日/手动检查、下载进度/取消、安装确认及扫描/永久删除互斥。
- [x] 将来源根重定位加入关键任务互斥；重定位运行时拒绝扫描、永久删除和更新安装，lease 在错误/panic 后自动释放。
- [x] 增加受保护稳定发布 workflow、受控 `latest.json` 生成器、正确/篡改/错误签名门禁和 Community Beta 无 updater 产物反向断言。
- [ ] 本地生成带强密码的 updater 密钥；将私钥/密码配置为仓库 Secrets，将公钥配置为 `VALOFRAME_UPDATER_PUBLIC_KEY` Variable，并制作可读取的加密离线备份（仓库目前不包含真实密钥）。
- [ ] 手动安装 `v0.2.1`，再发布 `v0.2.2`，完成发现、下载、验签、安装重启、取消/离线/失败恢复、篡改/错误公钥拒绝和禁止降级验收。
- [x] 增加 Windows 发布说明、内部 RC 边界和干净机器回归清单。
- [x] 对齐 v0.2.1 manifests、schema v16 架构/数据/恢复/元数据文档和候选发布说明，同时保留 v0.2.0 历史基线。
- [ ] （未来严格发行）逐项关闭 public policy 中的第三方合规、品牌、publisher、identifier、权利、Authenticode、VM 和数据安全检查；这些项目不阻止个人 updater 发布。

## 5. 当前验证门禁

```powershell
npm ci
npm test
npm run build
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo clippy --manifest-path src-tauri/Cargo.toml --locked --all-targets --all-features -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml --locked --all-targets --all-features
git diff --check
```

说明：

- 严格 Clippy 的既有告警已清零；后续改动不得通过降低门禁或新增 allow 规避问题。
- 真实 ACLOS 回归测试保持手工 ignored，自动化只使用脱敏 fixture。
- 涉及大数据集的改动至少验证 1k/10k 素材分页不会全量物化，虚拟列表 DOM 数量保持有界。
- v0.2.1 候选还必须覆盖首次/今天/6/7 天、终态新增 0/正数/不可用、同源改名/歧义、根重定位、仅移除索引、本人击杀/死亡与 v13/v14→v15 迁移。

## 6. 发布验收

- [x] CI workflow、固定 toolchain 和严格 Clippy 门禁已配置，本地等价命令无告警。
- [x] Draft PR #1 的 GitHub 托管 CI run 11 已通过；新的测试候选必须以对应 commit 的绿色 CI 为准。
- [x] CSP、opener 依赖和 capability 已完成代码层最小化。
- [ ] production release build 完成 CSP、目录选择、事件和媒体协议实机回归。
- [ ] 默认、自定义、多来源、固定磁盘、同源子目录改名和来源根恢复在 Windows 验证。
- [ ] 扫描互斥、进度、取消、partial 和 failed 可恢复。
- [ ] 1k/10k 素材库分页、详情和虚拟化保持响应。
- [ ] v0.2.1→v0.2.2 签名 updater、禁止降级、安装/失败恢复和 schema v16 用户状态保持通过。
- [ ] 除回收站中经过二次确认的永久删除外，应用不会在 ACLOS 原始素材目录创建、修改、移动、重命名或删除文件。
