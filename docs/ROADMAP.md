# 开发收敛路线图

> 更新于 2026-07-19。本文以当前工作区实现为准，记录已经落地的能力和发布前仍需完成的工作。任务级状态见 [TASKS.md](./TASKS.md)。

## 1. 当前状态

项目已经从“功能快速成型”进入架构、性能、测试和发布安全的收敛阶段。

| 领域 | 状态 | 当前实现 |
| --- | --- | --- |
| 素材扫描 | ✅ 已完成 | 默认路径、自定义目录、固定磁盘发现和多来源 `scan_roots`；一个逻辑批次只采集一次共享元数据 |
| 扫描任务 | ✅ 已完成 | 后台阻塞任务、全局互斥、job ID、进度事件、取消、终态持久化，以及启动时把异常遗留 running/cancelling 收敛为 failed |
| 元数据采集 | ✅ 已完成 | WonderfulDb、导出 JSON、`highlight.log`、Local Storage LevelDB 四路采集与容错合并 |
| 稳定账号身份 | ✅ 已完成 | 优先使用 match account ID / openid，来源目录只作为稳定回退，玩家名仅用于展示 |
| 数据库生命周期 | ✅ 已完成 | schema v13；迁移前后健康检查、未来版本拒绝、WAL 安全在线备份、原子迁移和可见恢复入口；普通请求不迁移 |
| 素材列表 | ✅ 已完成 | `list_clip_page` 分页摘要、全库精确 facets、服务端筛选与稳定排序 |
| 素材详情 | ✅ 已完成 | `get_clip_detail` 按需读取完整素材、标签和 clip-scoped 事件，包含结构化 not-found 错误 |
| 前端大列表 | ✅ 已完成 | 固定 50 条分页、旧响应隔离、跨页去重、对局组及极端单组视觉行虚拟化和临近底部追加 |
| 批量整理 | ✅ 已完成 | 批量收藏、标签绑定/解绑、回收/恢复均为单 command、单事务；永久删除使用回收时身份快照、持久 outbox 与启动恢复 |
| 前端控制器 | ✅ 已完成 | 扫描、分页、facets、详情、标签和素材 mutation 已从 `App.tsx` 拆为独立 hooks |
| 包管理器 | ✅ 已完成 | 统一 npm，`packageManager` 固定版本，仅保留 `package-lock.json` |
| 前端清理与拆包 | ✅ 已完成 | 旧生产 UI 与重复 CSS 已清理，四个 workspace 按页面懒加载，大型参考 PNG 已移除 |
| Rust 模块收敛 | ✅ 已完成 | library repository、scan-run persistence、library commands、migrations、scan service 与 media protocol 已拆出；生产 facade 已收敛为本域入口 |
| 缩略图队列 | ✅ 已完成 | 源封面优先；SQLite 持久队列、单 worker、指纹失效、受控媒体路由和 512→450 MiB 缓存清理已接入；生成器不可用时稳定降级 |
| CI | ✅ 已完成 | GitHub Actions 在 Ubuntu 验证前端，在 Windows 验证 fmt、严格 Clippy 与完整 Rust 测试；Rust 固定为 1.96.1 |
| 发布安全基线 | ✅ 已完成 | 最小 CSP/capability、单实例聚焦、数据库与删除恢复、schema v13/单实例 smoke 和内部 RC JSON 证据门禁已落地 |
| 发布实机回归 | ⏳ 未完成 | 仍需在 release build 验证目录选择、扫描事件、封面、视频、远程图片及安装/升级 |
| Windows 内部 RC | 🟡 部分完成 | 固定 FFmpeg、SPDX/许可材料、manifest 驱动 NSIS 检查、整体公开预检和隔离启动脚本已落地；公开许可/签名/updater 与 VM 验收仍未完成 |

## 2. 已落地的关键契约

### 2.1 扫描与任务状态

- `scan_default_aclos_dir`、`scan_custom_dir`、`scan_roots` 和固定磁盘发现最终进入同一套只读扫描语义。
- `scan_roots` 一次接收全部来源目录，统一建立扫描批次、共享元数据快照和汇总统计。
- 扫描协调器保证同一时刻只有一个任务运行；重复启动返回稳定的 `already-running` 错误。
- 任务状态覆盖 `starting`、`running`、`cancelling`、`completed`、`partial`、`cancelled` 和 `failed`。
- completed、partial、cancelled 和 failed 都会形成可查询的终态；取消和 panic 不得留下永久 running 记录。

### 2.2 元数据与所有权

- WonderfulDb 账号文件只读，使用 openid 派生密钥在进程内存中完成 AES-256-CBC 解密和解析，不生成明文副本。
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

## 3. 下一阶段优先级

### P0：仓库内工程门禁与发布安全基线（已完成；不代表公开发布验收）

- [x] 新增 CI，执行 npm 与 Rust 的完整验证矩阵。
- [x] 清零 Clippy 告警，以严格 `-D warnings` 作为门禁。
- [x] 为 Tauri 建立区分生产与开发环境的最小 CSP。
- [x] 删除未使用的 `tauri-plugin-opener` 及其 capability，收窄主窗口权限。
- [x] 同步 PRD、Architecture、README、Data Model 和元数据说明，移除旧实现口径。
- [x] 增加单实例保护、数据库健康检查/在线备份/原子迁移/恢复提示，以及永久删除 outbox 与启动恢复。
- [x] Windows smoke 门禁已实现，可验证 schema v13、空回收身份/删除日志和第二实例交接；workflow 只保留 JSON 证据且公开分发继续 fail closed。真实干净 VM 安装/升级/卸载证据仍属于下方发布验收项。

### P1：架构与前端体积收敛（已完成）

- [x] 拆分 Rust 扫描、数据库和 command facade，保持公开路径兼容且没有复制 SQL/实现。
- [x] 删除不在生产入口使用的旧组件和旧样式。
- [x] 合并重复 CSS，移除依赖导入顺序的 `App.css` 覆盖层。
- [x] 对四个 workspace 动态加载，主 JS 保持按页面拆包且无 chunk 警告。
- [x] 删除未使用的大型参考 PNG。
- [x] 单个对局组含极大量素材时复用外层滚动容器按视觉行二级虚拟化；1k/10k 单组 DOM 保持有界。

### P2：素材体验

- [ ] 补充真实 Windows 环境下的媒体 Range、路径权限、多磁盘和缩略图生成器回归。

### P3：效率与发布

1. [ ] 批量备注和更完整的批量失败恢复交互。
2. [ ] 在已落地的未签名 NSIS 内部 RC 基础上，闭合 publisher/许可/签名、VM 安装升级、自动更新和发布回滚方案。

## 4. 验证门禁

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
- 旧版本数据库逐级升级。
- 重复迁移幂等。
- 普通 `open_database` 和只读连接不触发迁移。

涉及扫描的改动还必须覆盖：

- 多根目录只建立一个扫描批次并只采集一次共享元数据。
- already-running、进度、取消、partial、failed 和 panic 终态。
- 不可访问来源不会误把历史素材标记 missing。

涉及列表和详情的改动还必须覆盖：

- 1k/10k 素材分页不会全量物化。
- 分页去重、旧响应隔离、失败重试和快速切换详情竞态。
- 虚拟列表首屏和滚动期间 DOM 数量保持有界。

## 5. 发布完成标准

满足以下条件后才进入 v1.0 发布候选：

- 已配置的完整验证门禁在 GitHub 托管 runner 上稳定通过。
- 严格 Clippy 持续保持无告警。
- production release build 下的 CSP、目录选择、事件和媒体协议完成实机回归。
- 缩略图方案至少具备可降级实现。
- 安装、升级、卸载和数据库迁移在 Windows 干净环境验证通过。
- 除回收站中经过二次确认的永久删除外，不会在 ACLOS 原始素材目录写入、移动、重命名或删除文件。
