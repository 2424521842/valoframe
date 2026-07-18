# WonderfulDb 账号名称优先级设计

## 目标

在不改变账号身份主键的前提下，把 WonderfulDb `event_ext.PlayerName` 提升为账号展示名的最高优先级来源，减少界面回退到 `账号 <openid>` 或 `wonderfulVideos<openid>` 的情况。

账号身份始终使用 WonderfulDb 文件名对应的 `openid`。昵称只用于展示和搜索，不用于合并、拆分或重新识别账号。

## 方案比较

### 方案 A：逐视频直接采用第一个事件名称

实现最简单，但同一账号的视频可能显示不同历史昵称；没有事件的视频仍会回退为目录名，账号列表也会被拆成多个展示组。不采用。

### 方案 B：按 openid 聚合并选择最新有效名称（采用）

从同一 WonderfulDb 账号的所有 match/video/segment/event 中收集有效 `PlayerName`，按 `match_time` 选择最新候选；同一对局内使用事件时间作为稳定次序，最后使用遍历次序作确定性兜底。选出的名称传播到该 openid 对应的所有 `wonderfulVideos<openid>` clips，并同步补齐相应 match 的玩家名。

优点是同一账号统一显示当前最近的 Riot ID，也能覆盖没有事件名称的同账号视频。代价是历史视频不会保留当时旧昵称，但账号列表本来表达的是账号而非历史身份快照。

### 方案 C：只有名称全局唯一时才传播

最保守，不会在多个历史昵称间选择，但真实数据库中已有多个账号存在历史名称变化，会继续保留大量目录名。不采用。

## 最终字段优先级

账号展示名称按以下顺序决策：

```text
WonderfulDb 最新有效 PlayerName
> video export JSON
> highlight.log
> LevelDB
> 账号 ID
> wonderfulVideos 目录名
```

WonderfulDb 名称会覆盖较低优先级的 scanner-owned `clip_metadata.account_name` / `player_name` 和同一 account ID 的 `matches.player_name`。收藏、备注、自定义标签和 clip ID 不受影响。

## 有效名称规则

候选名称必须：

- 去除首尾空白后非空；
- 包含一个非空名称部分和非空 tag 部分，格式为 `名称#tag`；
- 不能是文件路径、资源路径、URL、纯账号 ID、`undefined`、`null` 或 `wonderfulVideos...`；
- 保留原始 Unicode 名称和数字 tag，不擅自改写大小写或空格。

WonderfulDb 没有有效候选时，不清空现有名称，继续沿用导出 JSON、日志、LevelDB 和前端既有回退链。

## 数据流

1. `wonderful_db` 继续解析 `PlayerName` 到 clip-scoped event，不改变解密边界。
2. `wonderful_ingest` 在导入一个账号前计算该 openid 的最新有效名称。
3. 官方视频仍按 `video_src` 或 `(openid, match_id, video stem)` 严格匹配并写入时间线。
4. 官方导入完成后，通过现有账号提示传播边界，把名称写到该 openid 的 clips 和 matches；不按昵称重新匹配视频。
5. 前端无需新增回退规则，现有 `account_name/player_name` 优先逻辑会自然显示 WonderfulDb 名称。

`wonderfulVideosundefined` 不参与 openid 聚合；它继续作为无效来源处理，后续可单独清理历史索引。

## 错误与冲突处理

- 单个坏账号文件仍只产生 warning，不影响其他账号。
- 多个历史名称不是错误；以最新 `match_time` 为主。
- `match_time` 相同或缺失时使用稳定次序，保证重复扫描结果一致。
- 若传播数据库更新失败，本次扫描记录 warning；原始 ACLOS 文件始终只读。

## 测试

- 同一 openid 的旧名和新名并存时选择较新 match 的名称。
- 新名称传播到该账号所有 clips，包括自身事件没有 `PlayerName` 的视频。
- 名称覆盖导出 JSON、日志和 LevelDB 的旧展示名，但不改变 openid/clip ID/用户状态。
- 无效名称和 `wonderfulVideosundefined` 被忽略。
- 没有 WonderfulDb 名称时保留现有低优先级名称。
- 重复扫描结果幂等，不产生新的账号分组。
