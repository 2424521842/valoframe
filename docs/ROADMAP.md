# 开发收敛路线图

> 更新于 2026-08-10。当前路线为手动安装 `v0.2.1`，再以 `v0.2.2` 完成第一次应用内更新。v0.2.0/v0.2.1 计划保留为历史工程基线，当前任务见 [TASKS.md](./TASKS.md)。

## 1. 当前状态

首个 Community Beta 是既有历史渠道。当前优先完成个人开发者稳定 updater 发布：Tauri 更新签名和自动化资产验证为强制项；Authenticode、完整审批 policy 和大规模 Windows VM 矩阵保留为未来加固。

| 领域 | 状态 | 当前实现 |
| --- | --- | --- |
| 素材扫描 | ✅ 已完成 | 默认路径、自定义目录、固定磁盘发现和多来源 `scan_roots`；一个逻辑批次只采集一次共享元数据 |
| 扫描任务 | ✅ 已完成 | 后台阻塞任务、全局互斥、job ID、进度事件、取消、终态持久化，以及启动时把异常遗留 running/cancelling 收敛为 failed |
| 元数据采集 | ✅ 已完成 | WonderfulDb、导出 JSON、`highlight.log`、Local Storage LevelDB 四路采集与容错合并 |
| 稳定账号身份 | ✅ 已完成 | 优先使用 match account ID / openid，来源目录只作为稳定回退，玩家名仅用于展示 |
| 数据库生命周期 | ✅ 代码完成 | schema v16；v13/v14/v15→v16 迁移前后健康检查、专用在线备份、原子迁移、Windows 路径别名去重、未来版本拒绝和可见恢复入口；普通请求不迁移 |
| 素材列表 | ✅ 已完成 | `list_clip_page` 分页摘要、全库精确 facets、服务端筛选与稳定排序 |
| 素材详情 | ✅ 已完成 | `get_clip_detail` 按需读取完整素材、标签和 clip-scoped 事件，包含结构化 not-found 错误 |
| 前端大列表 | ✅ 已完成 | 固定 50 条分页、旧响应隔离、跨页去重、对局组及极端单组视觉行虚拟化和临近底部追加 |
| 批量整理 | ✅ 已完成 | 批量收藏、标签绑定/解绑、回收/恢复均为单 command、单事务；永久删除使用回收时身份快照、持久 outbox 与启动恢复 |
| 前端控制器 | ✅ 已完成 | 扫描、分页、facets、详情、标签和素材 mutation 已从 `App.tsx` 拆为独立 hooks |
| 包管理器 | ✅ 已完成 | 统一 npm，`packageManager` 固定版本，仅保留 `package-lock.json` |
| 前端清理与拆包 | ✅ 已完成 | 旧生产 UI 与重复 CSS 已清理，四个 workspace 按页面懒加载，大型参考 PNG 已移除 |
| Rust 模块收敛 | ✅ 已完成 | library/reconnect/relocation repositories、scan-run persistence、recursive adapter、共享文件身份、library/source commands、migrations、scan service 与 media protocol 已拆出 |
| 缩略图队列 | ✅ 已完成 | 源封面优先；SQLite 持久队列、单 worker、指纹失效、受控媒体路由和 512→450 MiB 缓存清理已接入；生成器不可用时稳定降级 |
| CI | ✅ 已完成 | GitHub Actions 在 Ubuntu 验证前端，在 Windows 验证 fmt、严格 Clippy 与完整 Rust 测试；Rust 固定为 1.96.1 |
| 发布安全基线 | ✅ 已完成 | 最小 CSP/capability、单实例聚焦、数据库与删除恢复、schema v13/单实例 smoke 和内部 RC JSON 证据门禁已落地 |
| v0.2.0 功能基线 | ✅ 代码完成 | 国际服来源、播放器快捷键、快速筛选和稳定更新工程已纳入历史计划；公开稳定发布资格未据此关闭 |
| v0.2.1 可靠性 | 🟡 集成收敛 | 扫描新鲜度/新增数量、同源重连、根恢复、仅移除索引、本人击杀/死亡时间轴和 schema v16 已实现并进入 M7 回归/文档收敛 |
| Community Beta | 📚 历史 | `v0.1.0-beta.1` 已发布；该未签名、无 updater 的渠道不构成稳定发布或 v0.2.x updater 起点 |
| 发布实机回归 | ⏳ 未完成 | 仍需手动安装 v0.2.1，并验证 v0.2.1→v0.2.2 签名 updater、安装/失败恢复和数据安全 |
| 稳定发布配置 | ✅ 已完成 | updater 私钥/密码 Secrets、公钥 Variable 和可读取的加密离线备份已配置；Authenticode 暂不要求 |

## 2. 已落地的关键契约

### 2.1 扫描与任务状态

- `scan_default_aclos_dir`、`scan_custom_dir`、`scan_roots` 和固定磁盘发现最终进入同一套只读扫描语义。
- `scan_roots` 一次接收全部来源目录，统一建立扫描批次、共享元数据快照和汇总统计。
- 扫描协调器保证同一时刻只有一个任务运行；重复启动返回稳定的 `already-running` 错误。
- 任务状态覆盖 `starting`、`running`、`cancelling`、`completed`、`partial`、`cancelled` 和 `failed`。
- completed、partial、cancelled 和 failed 都会形成可查询的终态；取消和 panic 不得留下永久 running 记录。

### 2.2 元数据与所有权

- WonderfulDb 账号文件只读，使用 openid 派生密钥在进程内存中完成 AES-256-CBC 解密和解析；不生成独立的解密数据库文件，但归一化字段和部分 snapshot/event 原始记录会以明文 `raw_json` 保存到应用 SQLite。
- 官方 video、segment 和 event 写入 `clips` / `clip_metadata` / `clip_segments` / `clip_events`。
- `matches` / `match_stats` / `match_events` 保存整场对局数据和回退事件。
- 执行顺序为文件索引与导出 JSON、日志与 LevelDB 回退、WonderfulDb 权威覆盖；任一数据源损坏不得阻断 mp4 入库。

### 2.3 分页、详情与聚合

- `list_clip_page` 默认返回 50 条，limit 范围为 1..200；返回 `totalCount`、`hasMore` 和 `nextOffset`。
- 列表只返回卡片/分组需要的 `ClipSummary`，不携带备注、OCR 原文、raw JSON 或事件。
- `get_clip_detail` 只加载当前素材的完整 `Clip`、完整标签和事件。
- `get_library_facets` 在全库快照中计算账号、来源、英雄、地图、模式、自定义标签、视频类型、状态、日期和大小聚合，不依赖前端已加载页。
- 生产前端不再使用 legacy `list_clips` 驱动素材库；legacy command 暂时仅保留兼容。

### 2.4 用户整理数据

- 单条和批量收藏、标签、回收操作共享后端事务实现。
- 批量命令对 ID 去重，并返回成功素材、未找到 ID 和必要的局部结果；前端按结果精确同步分页摘要、详情缓存和 facets。
- 备注仍是单素材操作；保存备注不会重新加载媒体，快速切换素材时旧请求不会污染新详情。

### 2.5 缩略图队列与缓存

- `clip_thumbnails` 保存独立于 `clips.cover_path` 的持久队列状态；扫描器继续只读源目录，源 `cover-*.jpeg` 始终优先。
- 前端按已加载页面批量调用 `ensure_clip_thumbnails`，不因卡片挂载或虚拟滚动产生 N 次调用；ready 事件只局部更新摘要与详情缓存。
- 一个后台 worker 从视频生成 JPEG，输出只写应用缓存目录；视频路径、大小或修改时间变化会使旧指纹失效，陈旧任务不能提交。
- 缓存以 512 MiB 为高水位并清理到 450 MiB；缺失、损坏、孤立和陈旧文件可幂等修复，不递归删除未知路径。
- 生成器只接受打包资源 `bin/ffmpeg(.exe)` 或绝对路径 `VHM_FFMPEG_PATH`，不搜索 `PATH`、不通过 shell 启动；不可用时队列进入全局稳定降级。

## 3. 当前版本：v0.2.1

### 3.1 版本目标

v0.2.1 不扩大 v0.2.0 的媒体来源或删除权限，聚焦“扫描结果可信、目录改名可恢复、时间轴语义正确”：

- 以每个启用来源最后一次完整成功扫描为准，显示首次/今天/N 天状态并在 7 个本地自然日后突出提醒。
- 每个终态按 job ID 展示真实新增数量；摘要缺失时显示不可用，不把未知伪造为 0。
- 同一来源内以规范化路径、双侧唯一稳定身份、身份全空旧行的双侧唯一旧指纹依次重连；歧义不合并。
- 用户选择新根后先预览、再在关键任务门禁下原子重定位；提交本身不刷新扫描新鲜度。
- missing/来源不可用素材可仅移除索引，但不触碰磁盘、不绕过回收站和永久删除授权。
- ACLOS 普通高光与击杀/死亡集锦使用正确的相对/绝对时间，仅显示本人击杀和本人死亡。
- 数据库升级到 schema v16，并安全支持 v13/v14/v15→v16。

### 3.2 当前收敛状态

- [x] M1–M6 的代码路径和定向自动化已进入共享工作区。
- [x] 版本 manifest、schema/架构/恢复/元数据/发布文档与 v0.2.1 发布说明对齐。
- [ ] 执行完整 npm、Rust、构建和差异门禁，清零影响发布的数据丢失、错误合并、越权路径或删除安全缺陷。
- [ ] 在干净 Windows 10/11 x64 环境保存真实来源改名/根恢复、仅移除索引、时间轴和 schema v16 迁移证据。
- [ ] 配置真实 Tauri updater 密钥和离线备份，完成 v0.2.1→v0.2.2 升级、篡改/错误签名/失败恢复和禁止降级演练。

### 3.3 发布边界

`release/public-release-policy.json` 继续记录第三方合规、品牌、publisher、identifier、素材权利、Authenticode、VM 和数据安全等严格发行检查，但不再阻止个人开发者稳定 updater。当前硬门禁是密钥配置、Tauri 签名、版本单调递增、下载地址与安装器版本绑定、512 MiB 上限、完整自动化和 draft 远端复核。

## 4. 历史计划：v0.1.0-beta.2

### 4.1 版本目标

本轮定位为首个 Community Beta 之后的小步稳定性更新，优先修复已知的素材库刷新问题、打通社区反馈入口，并补齐真实 Windows 环境的回归证据。不在本轮扩张数据边界或引入高风险的新发布基础设施。

版本仍使用应用基础版本 `0.1.0`，发布标签为 `v0.1.0-beta.2`。如果后续决定升级到 `0.1.1`，必须先为新基础版本补齐 Community Beta 渠道批准、素材范围记录及发布门禁适配，不能只修改 manifest 版本号。

### 4.2 已进入候选

- [x] 为回收站补充独立空状态，避免错误引导用户前往扫描。
- [x] 改进 Community Beta 下载、镜像、截图、安装提示和技术附件说明。

### 4.3 P0：发布阻断项

- [ ] 修复回收、恢复及同查询数据刷新后素材库跳回顶部的问题；刷新期间保留当前列表，避免短暂闪空，并覆盖“同查询保持位置、查询变化回到顶部、批量 mutation 后保持位置”三类回归。
- [ ] 把反馈入口从旧的“内部 Alpha”口径更新为 Community Beta，保留敏感数据脱敏、安全问题私下报告和永久删除仅用可丢弃副本等约束。
- [ ] 汇总并分级处理 `v0.1.0-beta.1` 反馈：P0 必须清零；P1 必须修复或明确阻断发版；P2/P3 必须记录去向，不允许无结论丢失。
- [ ] 在 production release build 上完成目录选择、扫描事件、默认/自定义/多来源/固定磁盘、封面与视频、媒体 Range、路径权限、缩略图生成及降级的 Windows 实机回归。
- [ ] 在干净 Windows 环境验证 `v0.1.0-beta.1 → v0.1.0-beta.2` 覆盖安装、启动、单实例、禁止降级和卸载；确认 schema v13、收藏、标签、备注与回收状态保留，安装/升级/卸载不修改 ACLOS 原始素材。
- [ ] 候选提交通过完整 CI、Community Beta preflight、最小 FFmpeg/许可材料、bundle gate 和隔离启动 smoke；发布说明记录已知问题、手动更新方式、完整 commit、安装器大小与 SHA-256。

### 4.4 P1：随版本完成

- [ ] 对 Beta 反馈中可稳定复现、改动面受控的扫描、预览、缩略图和整理问题做定向修复，并为每项修复补自动化回归或手工测试记录。
- [ ] 检查 760×560 最小窗口及 100%/150%/200% 缩放下的素材库、回收站、详情面板和确认对话框；修复阻断主要操作的布局问题。
- [ ] 发布前同步 README、Community Beta 说明和版本内发布说明；在安装器及哈希实际生成前，不提前把 `v0.1.0-beta.2` 写成当前可下载版本。

### 4.5 本轮非目标

- 批量备注、云同步、账号系统、素材上传、实时录制和游戏进程集成。
- Authenticode、自动更新、正式 publisher、完整法律/许可闭环，以及严格正式发布所需的全部 VM 矩阵。
- 数据库 schema 升级或改变“默认只读访问 ACLOS 原始素材，只有回收站二次确认后才允许永久删除”的安全边界。

### 4.6 发布完成标准

- P0 发布阻断项全部完成，且没有未关闭的数据丢失、越权文件操作、安全/隐私或主要流程缺陷。
- 标准门禁和 Community Beta 专用门禁均绑定同一个候选 commit 并通过。
- Windows 实机记录覆盖至少一次从 `v0.1.0-beta.1` 升级和一次干净安装，未发现用户索引数据或原始视频被意外修改。
- 发布产物仍明确标注“未签名 Community Beta、无自动更新”；未来严格发行检查继续单独跟踪。

## 5. 验证门禁

本地提交和 CI 应执行：

```powershell
npm ci
npm test
npm run build
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo clippy --manifest-path src-tauri/Cargo.toml --locked --all-targets --all-features -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml --locked --all-targets --all-features
git diff --check
```

涉及数据库迁移的改动还必须覆盖：

- 全新数据库初始化。
- v13→v15、v14→v15 专用备份和原子升级，以及可确定/不可确定事件回填。
- 重复迁移幂等。
- 普通 `open_database` 和只读连接不触发迁移。

涉及扫描的改动还必须覆盖：

- 多根目录只建立一个扫描批次并只采集一次共享元数据。
- already-running、进度、取消、partial、failed 和 panic 终态。
- 不可访问来源不会误把历史素材标记 missing。
- 首次/今天/6 天/7 天/多来源/停用来源/跨本地午夜的新鲜度，以及新增 0/正数/摘要不可用的终态文案。
- 同源目录改名、hardlink/复制/重复指纹、根重定位冲突、reparse point 和取消后重入均安全处理。

涉及列表和详情的改动还必须覆盖：

- 1k/10k 素材分页不会全量物化。
- 分页去重、旧响应隔离、失败重试和快速切换详情竞态。
- 虚拟列表首屏和滚动期间 DOM 数量保持有界。

## 6. 个人稳定更新完成标准

满足以下条件后即可完成首次个人稳定更新；严格企业级发行检查另行推进：

- 已配置的完整验证门禁在 GitHub 托管 runner 上稳定通过。
- 严格 Clippy 持续保持无告警。
- production release build 下的 CSP、目录选择、事件和媒体协议完成实机回归。
- 缩略图方案至少具备可降级实现。
- 手动安装 v0.2.1 后，v0.2.1→v0.2.2 签名升级、取消/离线/失败恢复、禁止降级和 schema v16 用户状态保持通过。
- 未启用 Authenticode 时，下载页明确提示 Windows 可能显示未知发布者或 SmartScreen。
- 除回收站中经过二次确认的永久删除外，不会在 ACLOS 原始素材目录写入、移动、重命名或删除文件。
