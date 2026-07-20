# GitHub 仓库页面设置清单

本文件记录无法通过仓库内容文件自动完成的 GitHub Settings 操作。每项都应由仓库管理员在 Draft PR #1 合并前后核对。

## About

- **Description**：`瓦刻（VALOFRAME）——本地优先的 Windows 对局高光素材管理器（开发预览）`
- **Website**：当前留空；没有经过审核的官网或下载页。
- **Topics**：`tauri`、`rust`、`react`、`typescript`、`sqlite`、`windows`、`local-first`、`video-library`、`highlight-manager`
- 在名称、商标和适用地区审查完成前，不把 Riot、VALORANT 或《无畏契约》用作仓库 topic、广告关键词或社交预览文案。

## Features

- 启用 **Issues**，让 `.github/ISSUE_TEMPLATE/` 中的表单可用。
- 暂不启用 **Discussions**；首轮内部测试使用结构化 Issue。
- 关闭 **Wiki**，避免与 `docs/` 产生两个事实来源。
- 在 **Security** 设置中启用 **Private vulnerability reporting**，并确认 `SECURITY.md` 的私密报告链接可用。

## 默认分支与合并

- 保持 `main` 为默认分支；不要把 RC/feature 分支直接改为默认分支。
- Draft PR #1 可先包含不分发游戏素材的社区与测试准备；只有人工对照原始授权确认公开源码和 GitHub 宣传范围后，才能加入本轮游戏素材、截图并转为 Ready for review。随后确认工作区干净、CI 绑定最新 commit 并通过。
- 为 `main` 建立 ruleset：至少要求 PR、CI 的 frontend 与 rust job 成功、分支为最新状态，并阻止 force push 和删除。
- 当前仓库历史经过公开净化；不要把旧私有历史重新合入公开分支。

## Social preview

- 当前只准备本地候选图，**不得上传**含 42 张游戏图片中任一素材的 Social preview 或 README 截图。仓库负责人虽已声明取得授权，但 `github-project-marketing` 仍只是操作假设；须先人工对照原始授权并形成审阅记录。仅使用项目自有元素的候选图也仍需完成名称/品牌审核。
- 推荐画布 1280×640，并保留边缘安全区；上传前在浅色/深色 GitHub 页面各检查一次。
- 只使用合成对局数据；不使用真实视频帧、玩家名、OpenID、真实路径、聊天或未列入固定清单的第三方素材。
- 画面或相邻文案必须保留非官方、非隶属、非赞助、非认可含义。负责人声明不等于素材范围或商标/品牌审查完成，也不覆盖公开安装包或下载页。
- 新的宣传图需像 [`docs/images/manifest.json`](./images/manifest.json) 一样记录生成方式、所用素材清单、操作假设、人工审阅状态、源 commit/工作树状态和成品 SHA-256。授权原件如含敏感信息，不得直接上传公开仓库；应保存可核验但不泄密的审阅记录。

## Releases

- 在 `release/public-release-policy.json` 返回通过前，不创建公开 GitHub Release，不上传安装器或 FFmpeg。
- 内部 RC 证据不等于公开发布批准；Actions artifact 也不得被用作绕过分发门禁的下载渠道。
- 未来公开预发布必须绑定 tag、完整 commit、安装器 SHA-256、有效签名/时间戳、发布说明和已通过的干净 VM 证据。

## 合并后核对

- GitHub 首页显示新的 README、MIT License、CI 徽章和有效文档链接。
- Issue 新建页只显示 Alpha 记录、Bug 表单和配置的帮助链接。
- Security 页面可私密报告漏洞。
- About 描述与 topics 已更新；Social preview 使用合成数据，且仅含已经人工确认适用于 GitHub 宣传的素材。
- `main` 最新 commit 的 CI 为绿色；不要引用旧 commit 的通过结果作为新候选证据。
