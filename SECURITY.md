# Security Policy

## Supported versions

瓦刻目前没有受支持的公开发行版。仓库中的代码和未签名内部 RC 都属于开发预览，不应被视为已完成安全审计的产品。

## 报告安全问题

请不要为以下问题创建公开 Issue：越权文件删除、路径穿越、任意文件读写、数据库或身份数据泄露、安装器安全问题，以及能够暴露真实用户数据的缺陷。

优先使用仓库 **Security → Report a vulnerability** 提交私密报告：

<https://github.com/2424521842/valoframe/security/advisories/new>

如果该入口不可用，请先暂停公开披露，并通过你与仓库维护者已经建立的私下渠道联系；不要为了联络而公开粘贴利用细节或用户数据。仓库管理员应在开始主体外测试前启用 GitHub Private Vulnerability Reporting，并补充稳定的安全联系地址。

报告请包含受影响 commit/构建、影响、最小复现步骤和已脱敏的证据。不要附加原始视频、数据库、WonderfulDb、LevelDB、真实路径、玩家名、OpenID、对局 ID、备注或完整日志。

## 处理原则

维护者会先确认收到报告，再评估影响与修复范围。涉及数据丢失、越权文件操作或隐私泄露的问题应立即停止对应测试批次；修复和披露时间以实际调查为准，不在开发预览阶段承诺固定 SLA。
