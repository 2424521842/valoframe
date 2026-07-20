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

## 二进制分发

Tauri 将仓库根 `LICENSE` 和 `LICENSE-SCOPE.txt` 原样映射为安装目录中的 `licenses/project/LICENSE.txt` 与 `licenses/project/LICENSE-SCOPE.txt`。发布 policy 固定两份获批文件的 SHA-256；预检与 bundle gate 会复核根文件、构建 staging 和最终 NSIS 载荷，任何缺失、差异或未获批改动都会阻断构建。
