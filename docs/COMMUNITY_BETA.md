# 瓦刻 v0.1.0 Community Beta 说明

Community Beta 是面向社区测试者的早期 Windows 版本。它用于收集真实环境中的兼容性和体验反馈，不代表仓库定义的严格正式发布门禁已经全部通过。

## 下载前请确认

- 只从本项目明确公布的下载入口获取安装包，并核对发布页同时公布的文件名、版本和 SHA-256。不要运行转发、改名或哈希不一致的副本。
- 安装包尚未进行 Authenticode 签名。Windows 可能显示“未知发布者”、SmartScreen 或信誉警告；这属于当前 Beta 的已知限制，也不等于安装包天然安全。若来源或哈希无法确认，请取消安装。
- Community Beta 没有自动更新。安装后不会自行获取新版本；请手动查看项目发布页和版本说明。
- Beta 可能包含兼容性、性能或数据处理缺陷。首次使用前应备份应用数据；测试永久删除时只使用另外复制、可以丢弃的视频。

当前 `v0.1.0-beta.1` 的主安装包是 [GitHub 上的 `VALOFRAME-v0.1.0-beta.1-x64-unsigned-setup.exe`](https://github.com/2424521842/valoframe/releases/download/v0.1.0-beta.1/VALOFRAME-v0.1.0-beta.1-x64-unsigned-setup.exe)，SHA-256 为 `a4993e5152cddc42623b4fe7dc308100f142617765ddad36aab66ae8aeb40d08`。备用入口是[蓝奏云 `瓦刻_0.1.0_x64-setup.exe`](https://wwbfc.lanzoue.com/iYmie3y080ef)（密码 `4sj6`），SHA-256 为 `3e8bc3692119a8f2c2e32fc4c46928e37cbfa40e6efeeafbc0060c7aac79ef74`。两个入口对应不同文件，必须按所选入口分别核验，不能混用校验值。

## 游戏图片与非官方声明

发布负责人已确认，当前清单锁定的游戏图片可以随 v0.1.0 Community Beta 发布。该确认是项目发布负责人的渠道决定，不声称 Riot Games、腾讯或其他第三方已经批准本项目，也不声称独立法律审核已经完成。

瓦刻是非官方社区项目，与 Riot Games、腾讯及其关联公司不存在隶属、赞助或认可关系。VALORANT、《无畏契约》及相关名称、商标和游戏内容归各自权利人所有；这些材料不因项目代码采用 MIT License 而获得重新授权。

机器可读的渠道决定见 [`release/approvals/community-beta-v0.1.0.json`](../release/approvals/community-beta-v0.1.0.json)，本版本的精确素材范围见 [`release/approvals/community-beta-v0.1.0-game-content-scope.json`](../release/approvals/community-beta-v0.1.0-game-content-scope.json)，原有素材记录见 [`release/approvals/game-content-rights.json`](../release/approvals/game-content-rights.json)。这个范围例外只适用于 v0.1.0 Community Beta，不会把严格正式发布的素材门禁改成已通过。

## FFmpeg

瓦刻调用随包提供的 FFmpeg，仅用于从本地视频生成缩略图，不将它用于录制、上传或自动更新。发布集合必须同时提供 FFmpeg 许可证材料和对应源码；安装目录包含许可证正文、构建/来源信息及源码可用性说明，对应源码包应与安装包一同提供。若下载入口缺少这些材料，请暂停分发或安装并向项目维护者反馈。

FFmpeg 及其相关材料遵循各自许可证，不属于瓦刻第一方代码的 MIT 授权范围。

Beta 已验证该最小构建没有启用外部 FFmpeg 库，并保守附带 IJG 声明；MinGW 工具链运行时许可复核和目标市场编解码器专利复核仍标记为“留待严格正式发布”，不伪装成已经完成的法律审批。

## 更新、反馈与正式发布

此版本只能手动更新。安装新版前，请阅读版本说明并备份应用数据；不要把降级安装当作回滚方案。

反馈前请移除真实路径、玩家名、OpenID、对局 ID、备注和其他个人信息。不要公开上传原始视频、数据库、备份、WonderfulDb、LevelDB 或完整日志；详细规则见[小范围测试指南](./INTERNAL_TESTING.md)。

Community Beta 获准供社区测试，不等于 `release/public-release-policy.json` 所定义的严格正式发布批准。代码签名、可信时间戳、完整许可审阅、干净 VM 证据、数据安全证据等正式门禁仍按现有发布文档独立跟踪。

## 维护者如何发布

Community Beta 只允许从默认分支手动发布，不能通过 push、PR 或普通版本标签自动触发：

1. 确认目标提交已经进入 GitHub 默认分支，CI 全部通过，并复制该提交的完整 40 位小写 SHA。
2. 在 GitHub Actions 中选择 `Unsigned community beta`。
3. 输入新标签，例如 `v0.1.0-beta.1`，并把完整 SHA 填入 `approved_source_commit`。首次发布不要预先创建标签；若上一次发布上传阶段失败，工作流只允许复用精确指向该 SHA 且没有同名 Release 的重试标签。
4. 如需在发布说明中展示备用镜像，必须同时填写 `mirror_url`、`mirror_password`、`mirror_file_name` 和 `mirror_sha256`；没有已核验镜像时四项全部留空。镜像哈希必须来自实际下载后的文件，不能复用 GitHub 安装包哈希。
5. 在确认框输入 `UNSIGNED-COMMUNITY-BETA v0.1.0-beta.1 <完整 SHA>`；标签和 SHA 必须与前两项输入逐字一致。
6. 工作流会重新构建并在原生 Windows 上验证最小 FFmpeg，生成对应源码和许可材料，构建未签名 NSIS，运行 bundle 门禁与启动烟测，再创建带直接安装链接、哈希、安装提示和技术附件说明的 GitHub **Prerelease**。

发布集合固定包含未签名安装器、`SHA256SUMS.txt`、最小 FFmpeg 二进制/构建证据、`ffmpeg-corresponding-source.tar.xz`、许可归档及门禁报告。工作流拒绝覆盖任何已有 Release；若发布上传失败后留下标签，只有标签仍精确指向已批准 SHA 且 Release 查询明确返回不存在时才能安全重试。它不会创建稳定版，并明确禁止生成 updater 的 `latest.json` 或 `.sig`。

首次启用工作流前，必须在仓库 **Settings → Environments → `community-beta-publish`** 完成以下保护配置；仅在 YAML 中写出环境名称不构成审批门禁：

- 配置至少一名 required reviewer，让构建与验证完成后必须再点一次批准。个人仓库可以把自己设为 reviewer，并保持 **Prevent self-review** 关闭；多人维护时建议改由另一名维护者审批并启用防自审。
- 将 deployment branches 限制为 GitHub 默认分支，不能允许任意分支部署到该环境。

required reviewer 和默认分支 deployment rule 未配置时，不得运行 Community Beta 发布工作流。
