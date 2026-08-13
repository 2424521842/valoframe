# 许可范围与决定记录

## 项目许可决定

项目负责人于 2026-07-20 选择使用 [MIT License](../LICENSE) 发布瓦刻（VALOFRAME）的项目自有源代码和随附文档。仓库根目录 `LICENSE` 是本项目的正式第一方许可文本，npm 与 Cargo 清单使用 SPDX 标识 `MIT`。

MIT 已提供使用、复制、修改和分发本项目第一方软件的条款及免责声明，因此当前公开发布决定是不再另设一份重复的最终用户许可协议（EULA）。这项决定不替代第三方合规、品牌审核或 Windows 代码签名要求。

仓库根目录的 [`LICENSE-SCOPE.txt`](../LICENSE-SCOPE.txt) 是面向源码与二进制接收者的自包含范围说明；它与 MIT 正文一起进入安装包。

## 不被重新授权的材料

MIT 不会改变仓库中第三方材料原有的许可或权利归属：

- npm、Cargo 依赖和 FFmpeg 继续遵循各自许可证、声明和源代码提供义务；生成的 SBOM 与第三方许可材料是这些义务的证据，不属于第一方 MIT 授权的替代品。
- Riot Games、腾讯、《无畏契约》、VALORANT 及相关游戏名称、商标和内容归各自权利人所有；本项目的 MIT 许可不授予这些权利。
- `src-tauri/icons/**`、`public/favicon.ico` 和 `public/valoframe-mark.png` 的图标/品牌素材不属于本项目的 MIT 授权范围；其允许用途和审批范围以 `src-tauri/icons/README.md` 为准。VALOFRAME/瓦刻名称、标识和图标不得用于暗示衍生项目获得官方认可。

文件或目录旁存在单独许可声明时，以该声明为准。

## 游戏素材的负责人声明与待审范围

仓库负责人于 2026-07-20 表示，`public/valorant-assets/**` 中清单锁定的 29 张英雄图和 13 张地图图已经取得授权。仓库自动化没有收到授权原件、可验证的外部证据编号或文件哈希，也没有收到权利人/被授权主体、地域、期限、撤销条件和具体允许用途条款，因此不能把负责人声明扩写为这些法律事实。[公开记录](../release/approvals/game-content-rights.json)只保存该声明、待人工核对事项和逐文件技术证据。

为准备本地受控测试，仓库采用最窄操作假设：公开源码、应用内展示、受控内部测试、Windows 内部测试包和 GitHub 项目宣传；源文件保持逐字节不变，当前 UI 仅进行缩放、CSS 裁切/遮罩和界面合成。这些是限制项目行为的操作假设，不是已核实的授权范围。

2026-07-21，发布负责人又明确批准清单完全一致的这 42 个文件用于 v0.1.0 Community Beta 的 GitHub Prerelease、二进制下载、Windows 安装包和应用内展示；精确范围见 [Community Beta 素材渠道记录](../release/approvals/community-beta-v0.1.0-game-content-scope.json)。这是基于负责人明确判断建立的单版本、非商业社区测试例外，不声称仓库自动化看过授权原件，也不声称 Riot Games、腾讯或其他第三方背书；它不会让严格正式发布 policy 自动通过。

素材清单 [`src/data/valorantAssets.json`](../src/data/valorantAssets.json) 固定精确来源 URL、路径、尺寸、字节数和 SHA-256；集合指纹为 `26c4c77a5a13d3ca1a84f4616b0cba1f251462882a0e86f9592d5fc8ef2e1c13`。`npm run assets:verify` 会校验 42 个 PNG 的完整结构、清单哈希和负责人声明记录绑定。自动化只能证明仓库字节与记录一致，不能证明授权真实性、有效性或范围。

2026-08-13，仓库负责人又为 `v0.2.1` 建立了 [`personal-community-stable` 渠道决定](../release/approvals/personal-community-stable-v0.2.1.json)：允许清单完全一致的 42 个文件用于免费个人社区版的 GitHub Stable Release、Windows 安装包和应用内展示。该记录只表达仓库负责人的单版本渠道决定，不声称自动化看过授权原件，不声称 Riot Games、腾讯或其他第三方批准，也不把这些素材纳入 MIT 或授予接收者独立复用权。

除已记录的 Community Beta 与 `personal-community-stable` 精确渠道外，在人工审阅完成并形成新的批准记录前，未来严格企业式发行、商业分发、转授权、第三方复用和独立修改/派生素材文件仍视为未获确认，严格 public policy 保持 fail closed。免费或非商业的发布意图不会免除第三方许可证义务；项目第一方 MIT 代码也不会因此增加“禁止商业使用”的附加限制。

## 二进制分发

Tauri 将仓库根 `LICENSE` 和 `LICENSE-SCOPE.txt` 原样映射为安装目录中的 `licenses/project/LICENSE.txt` 与 `licenses/project/LICENSE-SCOPE.txt`。发布 policy 固定两份获批文件的 SHA-256；预检与 bundle gate 会复核根文件、构建 staging 和最终 NSIS 载荷，任何缺失、差异或未获批改动都会阻断构建。
