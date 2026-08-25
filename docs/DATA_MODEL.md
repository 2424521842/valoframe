# Data Model

当前 SQLite schema 版本为 v21，数据库文件名为 `highlight-index.sqlite3`，位于 Tauri app data 目录。本文只描述当前落地表，不再保留早期 `sources/assets/asset_metadata` 草案。

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
| `source_dirs` | 真实素材来源路径、来源类型、扫描模式/根、名称、启用状态、最近错误和扫描时间 |
| `clip_groups` | 某来源内的对局/目录分组；`(source_dir_id, group_key)` 唯一 |
| `clips` | 本地 MP4 路径与可空 Windows 稳定文件身份、来源相对目录、文件状态、已有封面引用以及收藏/审核/备注/回收状态 |
| `clip_trash_snapshots` | 用户进入回收站时捕获的不可变文件/来源身份授权快照 |
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
| `pending_manual_clips` | NVIDIA 目录扫描发现但尚未手动分类的 MP4；`(normalized_path)` 唯一，逐来源登记、完整扫描后清理 |
| `scan_runs` | job、终态、批次统计（含 NVIDIA 新增待录入数）、错误摘要和起止时间 |
| `ad_creatives` | 广告方清单的本地缓存：素材文案、图片 URL、落地页模板、广告主名、权重与投放期；`creative_id` 为主键 |
| `ad_click_log` | 每次广告点击生成的 `click_id`、素材、广告位与时间；本地对账凭据，刷新清单时不清除 |
| `ad_impression_log` | 按 `(creative_id, slot, impression_date)` 聚合的曝光计数，避免逐条事件无界增长 |

当前没有 `app_settings` 表。`clips.cover_path` 仍只引用素材目录中已有的 `cover-*.jpeg`；自动生成文件由 `clip_thumbnails` 独立引用，避免扫描覆盖生成状态或把应用缓存误当成源文件。

三张 `ad_*` 表只保存广告方素材元信息与本地计数，不含任何玩家、对局或文件字段；广告开关与接口配置存在前端偏好（localStorage）而非数据库。

## 3. 核心表

### 3.0 `pending_manual_clips`（NVIDIA 待录入队列）

`source_kind = nvidia` 的来源在递归扫描时不把新发现 MP4 写入 `clips`，而是按 `normalized_path` 幂等登记到本表；历史已入库的 NVIDIA clip 仍按普通路径/稳定身份更新与重连。行记录来源、文件路径与规范路径、文件名、大小、修改时间、来源相对目录、`ignored` 标志（用户忽略后重扫不会恢复，也不会重复入库）与发现/最近可见时间。

- 重扫已见文件只刷新 `last_seen_at`，不重置 `ignored`；同一规范化路径只允许归属一个来源，repository 写入口、重连原子条件与数据库双向触发器共同保证 `pending_manual_clips` 和 `clips` 严格互斥，即使重叠来源或并发连接竞争也不能绕过。
- 只有来源完整枚举且未取消时，扫描才会删除本轮未见的待录入行，与 clip 的 missing 判定同策略。
- 用户提交分类后，`import_pending_manual_clip` 在同一事务内：校验文件仍存在 → 生成账户/对局标识 → 删除 pending 行以原子认领该路径 → 写入合成 `matches`、`clips` 与 `clip_metadata`（`metadata_status = 'manual'`、`metadata_source = 'manual'`、`match_id = game_id`）。任一步失败都会回滚，待录入行保持原状。
- 账户身份沿用既有派生：`match-account-<id>` 键直接复用 `matches.account_id`；新账户生成 `manual-<随机>` 的 `account_id`，并按展示名复用已存在的同名 manual 账户；`source-<id>` 兜底账户没有可携带身份，按新 manual 账户处理。

### 3.1 `source_dirs` 与 `clip_groups`

`source_dirs.path` 是来源记录的唯一真实路径；`scan_root_path` 是扫描器的授权边界根。`source_kind` 只接受 `aclos`、`nvidia`、`tracker`、`generic`，`scan_mode` 只接受 `aclos-structured`、`recursive-mp4`。ACLOS 使用结构化扫描器；NVIDIA、Tracker 和 generic 使用只读递归 MP4 适配器。`enabled` 控制所有类型来源（包括 NVIDIA）是否加入自动/批量同步，禁用不删除索引且不参与全局新鲜度提醒。全局“启动时自动扫描”偏好默认关闭并存于前端版本化 `localStorage`，没有写入 SQLite；开启后下次启动只同步 `enabled = true` 的来源，手动同步不受影响。来源 DTO 直接从该表返回，因此零素材、离线和 partial 来源也能出现在前端，不再依赖视频路径反推来源。

`clip_groups` 以来源 + `group_key` 唯一；来源根目录直接 MP4 可以没有 group，`来源/对局/MP4` 使用直接子目录作为 group。

来源注册按规范化根检查完全重复和父子重叠。完全重复来源会复用已有行；重叠来源需要用户确认，但 `clips.normalized_path` 的唯一身份与入库前归属检查仍禁止两个来源认领同一文件。完整同步才能标记 missing 并刷新 `last_scanned_at`；来源离线、权限/枚举部分失败、文件仍在变化、扫描 partial/failed 或取消时保留历史 clip 状态和上一次成功扫描时间，同时更新 `status`、`last_error`。前端将 ISO UTC 时间按本地自然日映射为首次/今天/N 天状态。

### 3.2 `clips`

关键字段：

| 字段 | 说明 |
| --- | --- |
| `file_path` / `normalized_path` | 原路径和大小写无关规范化路径，均有唯一约束 |
| `source_dir_id` / `clip_group_id` | 来源和可空对局组 |
| `file_name` / `extension` / `size_bytes` / `modified_at` | 文件摘要 |
| `file_volume_serial` / `file_index_high` / `file_index_low` | Windows 稳定文件身份；必须全空或全非空，读取失败仍允许正常索引 |
| `duration_ms` / `recorded_at` | 可空媒体/录制时间；当前扫描不会主动探测视频时长 |
| `source_relative_dir` | 视频父目录相对 `scan_root_path` 的 `/` 分隔路径；根目录视频为空字符串 |
| `cover_path` / `cover_source` | 已有封面引用和来源状态 |
| `file_status` | `available`、`missing` 或 `trashed` 等状态 |
| `is_favorite` / `note` | 用户数据，重扫不得覆盖 |
| `review_decision` / `reviewed_at` | `unreviewed`、`liked`、`disliked` 及可空审核时间；用于后续卡片筛选 |
| `first_indexed_at` / `last_seen_at` / `updated_at` | 索引生命周期 |

v13 升级到 v14 的历史步骤会把现有来源回填为 `aclos + aclos-structured`，`scan_root_path = path`；相对目录优先由文件父目录与来源根做大小写无关的词法计算，无法计算时退回已有 group key。已有收藏回填为 `liked`，`reviewed_at` 使用该 clip 的 `updated_at`；非收藏保持 `unreviewed`。

v14→v15 只增加稳定身份、事件本人死亡和扫描摘要可用性字段/索引，不在迁移中遍历磁盘；既有 clip 的三个文件身份字段保持 `NULL`，由后续扫描惰性填充。v16 随后统一普通与 Win32 verbatim 路径键，并只合并同来源、完整身份相同、去前缀路径等价且授权状态无歧义的双行配对。v17 新增 `pending_manual_clips` 待录入队列（幂等建表，已有库升级时由启动迁移自动创建），用于暂存 NVIDIA 目录中发现、但尚未手动分类的 MP4；同时为 `scan_runs` 增加 `pending_clip_count`，保证后台/启动扫描的终态摘要恢复后仍能提示待录入数量。v18 在同一迁移事务内按可靠时间戳传播 WonderfulDb 的最新账号名称，并清除旧版按文件排序猜测出的 ACLOS 封面绑定、同步转入视频缩略图生成队列；升级前仍创建经校验的数据库备份，任一数据修复失败都会连同 `user_version` 更新一起回滚。v19 修正手动录入的比赛时间：v19 之前 NVIDIA 手动录入把扫描得到的裸 Unix 秒 `modified_at` 直接写入 `matches.started_at`，与其他元数据通道写入的可读日期时间格式不一致，素材库因此把原始纪元秒当作比赛时间显示；迁移在同一事务内就地重写这些纯数字取值，官方通道写入的日期时间保持不变。v20 拆分共享的原始录像分组：无畏时刻把每场对局的高光放在各自的 `<对局 ID>/` 目录，但完整对局录像统一落在共享的 `record/` 目录，早期扫描把整个目录当作一个 group，导致互不相关的对局被折叠为同一场；迁移按文件为 `record/` 中每段录像单独建组，并清除随之空掉的旧 group。v21 新增 `ad_creatives`、`ad_click_log`、`ad_impression_log` 三张广告位表（幂等建表），仅在用户开启广告后写入。升级保留文件状态、普通路径 clip ID、收藏、评审、备注、标签、元数据、时间轴、可确认的精确封面、回收快照、删除 intent 和原始 JSON。

`trashed` 本身只是应用数据库状态，不会移动或删除视频。`remove_clip_from_index` / `remove_clips_from_index` 只允许普通库中 missing 或来源不可用、且没有删除 intent 的记录；它删除索引行及级联的标签/备注等应用状态，不触碰原视频，批量操作逐项报告结果。用户在回收站明确二次确认 `delete_clips_permanently` 后，应用先持久化 `clip_delete_intents`，再触碰本地文件，最后在一个短事务内同时移除 intent 与 clip；非 `trashed` 记录会被后端拒绝。

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
clip_segments + clip_events        = 单个视频的组装区间和归一化事件时间
```

`clip_events.segment_id` 若非空，必须引用同一 `clip_id` 的 segment。三个数据库触发器阻止跨 clip 插入、更新或移动已引用 segment。

`clip_events` 以 `(clip_id, event_key)` 唯一；`killer_is_me` 标记本人击杀，schema v15 的 `killed_is_me` 标记本人死亡。普通高光按 `segmentStart + eventStart` 归一化，击杀/死亡集锦把 `eventStart` 当作视频绝对时间；超出 `[0, duration]` 的值不裁剪并保存为空/警告。`clip_metadata.kill_count` 只统计同一 clip 下 `event_type = 'kill' AND killer_is_me = 1` 的事件。整场 `match_events` 或其他视频事件不得累加到当前视频。

### 3.8 标签与扫描批次

`tags.name` 唯一；`clip_tags` 以 `(clip_id, tag_id)` 为主键并在任一端删除时级联。扫描器不创建标签，视频类型只写入 `clip_metadata`。v10 会把旧版自动多杀/集锦标签回填为视频类型元数据后移除，其他用户标签保持不变。

`scan_runs` 保存 `job_id`、root、状态、来源/分组/新增/更新/missing/封面/元数据统计、有限错误 JSON、兼容性的省略计数和用户消息。状态终态包括 `completed`、`partial`、`failed`、`cancelled`。schema v15 的 `summary_available` 明确区分真实持久化摘要与终态兜底行；只有值为 1 时才能把 `new_clip_count = 0` 展示为“新增 0 个”，否则显示新增数量不可用。

## 4. 稳定身份

正常扫描首先以 `normalized_path` 识别现有素材；路径已消失时，schema v15 才在同一来源内使用双侧唯一的 Windows 稳定文件身份进行重连。旧库身份全空时可以退回双侧唯一的“文件名 + 大小 + 修改时间”指纹。稳定身份索引故意不是唯一索引，因为硬链接共享身份；任何复制、硬链接、重复身份或重复指纹都安全地不合并。只有 `symlink_metadata` 明确返回 `NotFound` 才把旧路径视为消失。

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
| `manual` | 用户手动录入的 NVIDIA 分类（`metadata_source = 'manual'`，字段由待录入导入流程写入） |

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

### 5.4 卡片审核状态

| 值 | 含义 |
| --- | --- |
| `unreviewed` | 尚未在快速筛选中作出决定 |
| `liked` | 保留候选；v13 的已有收藏在迁移时进入该状态 |
| `disliked` | 剔除候选；只改变审核维度，不删除文件或索引 |

`review_decision` 与 `is_favorite` 自 v14 起是可分别查询的字段。迁移只做一次历史映射；后续“喜欢”事务同步收藏，“不喜欢”取消收藏但不修改文件状态。

## 6. 读模型

### 6.1 `ClipSummary` / `ClipPage`

生产列表使用 `list_clip_page`，不是 legacy `list_clips`。查询 join `clips`、`source_dirs`、`clip_groups`、`clip_metadata`、`matches` 和 `match_stats`，当前页标签通过单次批量查询附加。

摘要保留：

- 来源真实路径、来源类型、扫描模式/根、相对目录和分组。
- 稳定账号 key、展示名和来源类型。
- 英雄、地图、模式、比分、KDA、战斗分、胜负和官方分类。
- 文件状态、收藏、卡片审核结果/时间、源/生成封面可用性、`thumbnailStatus`、`thumbnailRevision` 及 tag IDs。

摘要不返回备注、OCR/提取详情、raw JSON、完整 Tag 对象或事件。`ClipPage` 同时返回 offset、limit、total、hasMore 和 nextOffset。

### 6.2 `ClipDetail`

`get_clip_detail` 只读取目标 clip，返回完整 Clip、完整 Tag 对象和该 clip 的事件。不存在时 command 返回稳定 `clip-not-found` 错误。

### 6.3 `LibraryFacets`

facets 针对整个索引计算总量、活跃量、收藏、回收、自定义标签、大小/日期范围，以及账号、来源、英雄、地图、模式、状态和视频类型。视频类型依次为三杀时刻、四杀时刻、五杀时刻、六杀时刻、击杀集锦和死亡时刻；`count` 包含回收数据，`active_count` 排除 `trashed`，避免前端从已加载页面推导错误聚合。

## 7. 写入和事务

- 文件 upsert 以规范化路径匹配；更新文件摘要和来源相对目录时保留收藏、审核结果/时间、备注和标签。
- 同源重连先在 SQLite TEMP 表完成来源级候选唯一性统计，再分页 apply；可信匹配原地更新 clip 路径并保留 ID/用户状态，同时重置缩略图指纹。歧义候选作为新素材写入，旧行只在完整枚举后收敛 missing。
- 来源根重新定位先只读预览，提交时重新验证；路径更新在单事务中使用两阶段占位以支持大小写变化/目录互换，并同步来源根、clip 路径/相对目录、分组和根内封面/元数据引用。回收素材、删除 intent 或关键任务冲突会阻断提交。
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

v14 新增来源同步索引 `(enabled, scan_mode, scan_root_path)`、相对目录索引 `(source_dir_id, source_relative_dir)`，以及以审核结果和文件状态开头的卡片队列索引。列表筛选通过绑定参数读取 `review_decision`。

v15 新增来源内非唯一稳定身份索引、来源 + 旧文件名/大小/修改时间候选索引，以及 `summary_available`/`killed_is_me` 所需字段约束。身份索引不得改为 UNIQUE；歧义必须由来源级匹配计划显式处理。

v9 版本号继续保留以兼容已经升级的本地数据库。早期开发库中可能残留未使用的 `diagnostic_events` 表；当前应用不会创建、写入、读取或导出该表，也不会对已有数据库执行破坏性删除。
