# Data Model

当前 SQLite schema 版本为 v13，数据库文件名为 `highlight-index.sqlite3`，位于 Tauri app data 目录。本文只描述当前落地表，不再保留早期 `sources/assets/asset_metadata` 草案。

## 1. 生命周期与连接

- `initialize_database` 只在应用启动时创建目录并运行版本迁移；高于当前版本的数据库会被拒绝，旧库升级前通过 SQLite Online Backup API 生成可独立读取的 WAL 安全备份。
- `open_database` 打开已有数据库用于普通写请求，不运行迁移。
- `open_database_read_only` 用于列表、详情、facets、来源和媒体路径查询。
- 所有连接启用 foreign keys、busy timeout 和自然文件名 collation；迁移连接启用 WAL，并在迁移前后运行 `quick_check` 与 `foreign_key_check`。
- 时间列兼容 Unix 秒文本和 SQLite 时间文本，DTO 层统一转换；筛选 SQL 显式处理两种表示。

迁移必须向前兼容已有数据库，通过 `PRAGMA user_version`、`ALTER TABLE`、幂等建表/索引完成，不以删除用户数据库作为升级策略。全部 schema 变更和 `user_version` 更新位于同一个 `BEGIN IMMEDIATE` 事务；备份保存在应用数据目录的 `backups` 子目录并只保留最近 3 份。完整性检查或初始化失败时 GUI 会明确停止启动，并提供打开数据/备份目录的恢复入口，不会自动覆盖可疑数据库。

## 2. 表与所有权

| 表 | 所有权与用途 |
| --- | --- |
| `source_dirs` | 真实素材来源路径、名称、启用状态、最近错误和扫描时间 |
| `clip_groups` | 某来源内的对局/目录分组；`(source_dir_id, group_key)` 唯一 |
| `clips` | 本地 MP4 身份、路径、文件状态、已有封面引用以及收藏/备注/回收状态 |
| `clip_delete_intents` | 用户已确认的永久删除意图、目标快照、执行状态、租约与稳定错误；用于崩溃后幂等恢复 |
| `clip_thumbnails` | 独立于源封面的生成指纹、持久队列状态、安全缓存 basename、重试与缓存大小 |
| `clip_metadata` | 单个视频的展示字段、官方分类、解析状态、来源和详情文本 |
| `matches` | 多个 clips 共享的整场身份和展示字段 |
| `match_stats` | KDA、战斗分、回合与胜负等整场统计 |
| `match_snapshots` | WonderfulDb 快照记录及必要原始 JSON；按 snapshot ID 去重 |
| `match_events` | 整场回退事件，不能用于某个 clip 的官方击杀计数 |
| `clip_segments` | 单个视频的剪辑/组装区间 |
| `clip_events` | 单个视频的相对事件时间线 |
| `tags` | 用户创建的标签名称和颜色 |
| `clip_tags` | clip 与 tag 的多对多绑定 |
| `scan_runs` | job、终态、批次统计、错误摘要和起止时间 |

当前没有 `app_settings` 表。`clips.cover_path` 仍只引用素材目录中已有的 `cover-*.jpeg`；自动生成文件由 `clip_thumbnails` 独立引用，避免扫描覆盖生成状态或把应用缓存误当成源文件。

## 3. 核心表

### 3.1 `source_dirs` 与 `clip_groups`

`source_dirs.path` 是来源的唯一真实路径。来源 DTO 直接从该表返回，因此零素材来源和失败来源也能出现在前端，不再依赖视频路径反推来源。

`clip_groups` 以来源 + `group_key` 唯一；来源根目录直接 MP4 可以没有 group，`来源/对局/MP4` 使用直接子目录作为 group。

### 3.2 `clips`

关键字段：

| 字段 | 说明 |
| --- | --- |
| `file_path` / `normalized_path` | 原路径和大小写无关规范化路径，均有唯一约束 |
| `source_dir_id` / `clip_group_id` | 来源和可空对局组 |
| `file_name` / `extension` / `size_bytes` / `modified_at` | 文件摘要 |
| `duration_ms` / `recorded_at` | 可空媒体/录制时间；当前扫描不会主动探测视频时长 |
| `cover_path` / `cover_source` | 已有封面引用和来源状态 |
| `file_status` | `available`、`missing` 或 `trashed` 等状态 |
| `is_favorite` / `note` | 用户数据，重扫不得覆盖 |
| `first_indexed_at` / `last_seen_at` / `updated_at` | 索引生命周期 |

`trashed` 本身只是应用数据库状态，不会移动或删除视频。`remove_clip_from_index` 只删除索引行及级联数据，也不触碰原视频。用户在回收站明确二次确认 `delete_clips_permanently` 后，应用先持久化 `clip_delete_intents`，再触碰本地文件，最后在一个短事务内同时移除 intent 与 clip；非 `trashed` 记录会被后端拒绝。

### 3.3 `clip_trash_snapshots`

素材从非回收状态进入 `trashed` 的同一事务，必须先写入一条不可变的回收身份快照；数据库触发器拒绝没有快照的状态切换。快照保存用户回收时的视频/来源原始路径与规范路径、扩展名、文件是否存在、大小、修改时间，以及 Windows 目标和来源目录的 volume serial/file index。恢复素材会删除该快照；再次回收会重新捕获当前对象。由 v12 升级而来的旧回收记录没有可证明的快照，永久删除会安全拒绝，用户需先恢复再重新放入回收站。

扫描可以更新 clip 的普通索引字段，但不会修改快照，因此同路径文件被外部替换后无法继承旧对象的删除授权。快照随 clip 删除级联清理，禁止原地更新。

### 3.4 `clip_delete_intents`

每个 clip 最多一个永久删除意图，并以 `ON DELETE RESTRICT` 阻止普通“仅移除索引”、来源级联或恢复操作绕过待完成删除。intent 保留用户确认时的视频路径、来源路径、规范路径、扩展名、大小、修改时间，以及 Windows 目标/来源的 volume serial 与 file index；重复请求复用原 intent，不会把授权静默扩展到同路径的替换文件。实际删除通过已校验且禁止后续 delete-share 的句柄执行 `SetFileInformationByHandle`，不在校验后重新按路径删除。

`pending` 表示可安全重试，`processing` 带有短租约；schema 保留 `blocked` 兼容状态，但当前身份变化或路径安全阻断会立即取消旧授权并删除 intent，文件和 clip 保持不变，用户必须重新确认。只有在来源目录仍可验证时，目标不存在才可被确认为已删除；来源离线不会被误判成缺失。数据库连接使用 `WAL + synchronous=FULL`，因此删除授权在触碰文件前完成耐久提交；文件删除后即使进程在 SQLite 收尾事务前退出，启动恢复仍可用保留的 intent 收敛索引。

### 3.5 `clip_thumbnails`

每个 clip 至多一行，并通过外键在删除 clip 索引时级联删除。关键字段：

| 字段 | 说明 |
| --- | --- |
| `fingerprint` | 输出版本 + 规范化视频路径 + 大小 + 修改时间的 SHA-256，用于失效和陈旧结果保护 |
| `status` | `pending`、`running`、`ready`、`failed`、`unavailable`、`suppressed` 或 `evicted` |
| `cache_file` | 只允许直接位于应用缓存根的安全 basename；不保存或返回任意绝对路径 |
| `attempt_count` / `next_attempt_at` | 持久重试次数与退避时间 |
| `error_code` / `last_error` | 稳定机器码及可空内部错误详情；前端契约只依赖稳定码 |
| `byte_size` / `revision` / `generated_at` | ready 缓存大小、版本和生成时间 |

ready 行必须同时具有 `cache_file`、`revision` 和 `byte_size`；`cache_file` 唯一，且数据库和媒体协议都会拒绝斜杠、反斜杠及路径穿越。源封面存在或素材不可用时使用 `suppressed`，但不会修改 `clips.cover_path`。

### 3.6 `clip_metadata`

每个 clip 至多一行。主要字段分为：

- 展示与搜索：账号名、玩家名、英雄、地图、模式、比分、KDA、武器、提取文本。
- 对局关联：`match_id`。
- 官方视频：video ID/name/type、highlight type、round score、`round_score_source`、`kill_count`。
- 来源保护：`metadata_status`、`metadata_source`、parse error 和必要详情 JSON。

`metadata_source = video_export` 保护导出 JSON 已提供字段；`metadata_source = wonderful_db` 表示官方 clip 数据具有最高优先级。

`round_score_source` 只接受受信来源语义：`wonderful_db` 表示 ACLOS 已保存的官方原值，`highlight_log_delta` 表示由官方日志中本人累计战斗分的相邻回合差恢复。日志事件先按 `GameStart` / `GameSettle` 和 GameID 分局，再以 GameSettle OpenID 绑定同账号 WonderfulDb，并校验唯一结算总分、可用的当前玩家名以及从 round 0 开始连续出现的 `RoundEnd`。日志尾部缺少 `RoundEnd` 时只恢复已观测的连续前缀，缺失尾局不推算。重扫只在官方 video ID 与 match ID 均未变化时保留这些受信值；无法验证的旧值会被清空。

### 3.7 对局与视频时间线

```text
matches + match_stats + match_events = 整场所有权
clips + clip_metadata              = 官方视频身份和分类
clip_segments + clip_events        = 单个视频的组装区间和相对事件
```

`clip_events.segment_id` 若非空，必须引用同一 `clip_id` 的 segment。三个数据库触发器阻止跨 clip 插入、更新或移动已引用 segment。

`clip_events` 以 `(clip_id, event_key)` 唯一；`clip_metadata.kill_count` 只统计同一 clip 下 `event_type = 'kill' AND killer_is_me = 1` 的事件。整场 `match_events` 或其他视频事件不得累加到当前视频。

### 3.8 标签与扫描批次

`tags.name` 唯一；`clip_tags` 以 `(clip_id, tag_id)` 为主键并在任一端删除时级联。扫描器不创建标签，视频类型只写入 `clip_metadata`。v10 会把旧版自动多杀/集锦标签回填为视频类型元数据后移除，其他用户标签保持不变。

`scan_runs` 保存 `job_id`、root、状态、来源/分组/新增/更新/missing/封面/元数据统计、有限错误 JSON、兼容性的省略计数和用户消息。状态终态包括 `completed`、`partial`、`failed`、`cancelled`。

## 4. 稳定身份

素材文件身份是 `normalized_path`，不是文件名。

账号分组身份按以下顺序派生：

1. `matches.account_id`：`match-account-<id>`。
2. 来源名/路径中可验证的数字 openid：同样归一为 `match-account-<openid>`。
3. 无稳定账号 ID 时：`source-<source_dir_id>`。

玩家名、账号展示名及其历史改名永远不进入 identity key。列表和 facets 使用相同表达式，因此改名不会拆分账号，相同昵称也不会合并不同账号。

## 5. 状态

### 5.1 文件状态

| 值 | 当前语义 |
| --- | --- |
| `available` | 文件存在且未进入应用回收站 |
| `missing` | 某次可判定扫描中未再见到历史文件 |
| `trashed` | 应用回收状态，原文件保持不变 |

`inaccessible`、`unsupported` 等值仍可被查询契约接受，但当前扫描器不会把它们作为常规 clip 持久化终态。来源不可访问时记录来源错误，并避免把该来源历史 clips 误标 missing。

### 5.2 元数据状态

| 值 | 含义 |
| --- | --- |
| `not_found` | 未找到可关联元数据 |
| `parsed` | 导出 JSON 完整解析 |
| `partial` | 仅提取到部分字段 |
| `failed` | 元数据存在但解析失败 |
| `enriched` | 已关联 WonderfulDb 或结构化对局数据 |

### 5.3 缩略图状态

| 值 | 含义 |
| --- | --- |
| `pending` | 等待单 worker 处理，可能带未来退避时间 |
| `running` | 已原子 claim；应用重启会恢复为 pending |
| `ready` | 当前指纹已有受控 JPEG 缓存 |
| `failed` | 重试耗尽或不可重试的单素材错误；显式 retry 可重新排队 |
| `unavailable` | 受控生成器不可用时的全局降级状态 |
| `suppressed` | 已有源封面，或素材当前不是 available |
| `evicted` | 缓存预算清理了文件；再次 ensure 时可重新排队 |

## 6. 读模型

### 6.1 `ClipSummary` / `ClipPage`

生产列表使用 `list_clip_page`，不是 legacy `list_clips`。查询 join `clips`、`source_dirs`、`clip_groups`、`clip_metadata`、`matches` 和 `match_stats`，当前页标签通过单次批量查询附加。

摘要保留：

- 来源真实路径和分组。
- 稳定账号 key、展示名和来源类型。
- 英雄、地图、模式、比分、KDA、战斗分、胜负和官方分类。
- 文件状态、收藏、源/生成封面可用性、`thumbnailStatus`、`thumbnailRevision` 及 tag IDs。

摘要不返回备注、OCR/提取详情、raw JSON、完整 Tag 对象或事件。`ClipPage` 同时返回 offset、limit、total、hasMore 和 nextOffset。

### 6.2 `ClipDetail`

`get_clip_detail` 只读取目标 clip，返回完整 Clip、完整 Tag 对象和该 clip 的事件。不存在时 command 返回稳定 `clip-not-found` 错误。

### 6.3 `LibraryFacets`

facets 针对整个索引计算总量、活跃量、收藏、回收、自定义标签、大小/日期范围，以及账号、来源、英雄、地图、模式、状态和视频类型。视频类型依次为三杀时刻、四杀时刻、五杀时刻、六杀时刻、击杀集锦和死亡时刻；`count` 包含回收数据，`active_count` 排除 `trashed`，避免前端从已加载页面推导错误聚合。

## 7. 写入和事务

- 文件 upsert 以规范化路径匹配；更新文件摘要时保留收藏、备注和标签。
- 一次扫描由多个可恢复短事务组成，不承诺整个多根批次原子提交。
- 单个官方 clip 的视频类型元数据与时间线各有事务边界；重扫按权威来源幂等修复，且不改动用户标签。
- 批量收藏、标签和回收在单个事务中完成；任何一行失败则整批回滚。
- 删除 tag 由外键级联清理 `clip_tags`；删除 clip 索引级联清理元数据、缩略图队列行、标签绑定、segments 和 events。生成缓存的孤立文件由受控维护任务清理。

## 8. 性能索引

v7 在既有来源、状态、日期和事件索引之外增加：

- 修改时间 + ID。
- 文件大小 + ID。
- 数字感知名称 + ID。
- tag + clip 复合索引。

所有排序追加 ID tie-breaker，确保 offset 分页无重复或缺口。count、页面和标签读取处于同一只读事务快照。

v8 新增 `clip_thumbnails(status, next_attempt_at, clip_id)` 队列索引，以及非空 `cache_file` 的唯一部分索引。队列 claim、ready 提交和失效更新使用绑定参数与条件 UPDATE；ready 提交同时验证当前 clip 行，避免扫描和生成并发时提交陈旧缓存。

v9 版本号继续保留以兼容已经升级的本地数据库。早期开发库中可能残留未使用的 `diagnostic_events` 表；当前应用不会创建、写入、读取或导出该表，也不会对已有数据库执行破坏性删除。
