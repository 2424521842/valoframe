# 瓦刻（VALOFRAME）图标资产

本目录的应用图标来自用户提供并在仓库外归档的 `VALOFRAME_正规图标格式包.zip`，并以包内透明 1024 px 主图作为唯一派生源。原始 ZIP 不纳入 Git；仓库通过下列哈希、受审主图副本和派生说明保留可追溯性。

## 来源与完整性

- 来源包 SHA-256：`F841A59AD8F07DCF67638F7713BAFDE09A85784FD0DCC09769DB9DADF964A604`
- 包内 `manifest.json`：24/24 个声明文件的字节数和 SHA-256 已于 2026-07-18 校验通过。
- 透明主图：`VALOFRAME_Icon_Package/VALOFRAME_AppIcon_1024_Transparent.png`
- 透明主图 SHA-256：`C825439F09EBF5BDBD13F82E06044F86D75735A9651EEEE5DF667A360BB7B67E`
- 仓库主图副本：`valoframe-source.png`，与包内透明主图逐字节一致。

图标包是由位图主图整理出的派生包，不包含可编辑的 SVG、AI 或 EPS 矢量源文件。

## 派生方式

桌面、Windows Store、Android 和 iOS 派生图由 Tauri CLI 从透明主图统一生成：

```powershell
npm run tauri -- icon src-tauri/icons/valoframe-source.png
```

重新生成时应保留源图并检查差异，不要用 ZIP 内的 opaque 版本覆盖透明主图。

`src-tauri/tauri.conf.json` 当前映射如下：

| Tauri 配置项 | 用途 |
| --- | --- |
| `icons/32x32.png` | 32 px 桌面图标 |
| `icons/128x128.png` | 128 px 桌面图标 |
| `icons/128x128@2x.png` | 256 px 高 DPI 桌面图标 |
| `icons/icon.icns` | macOS 图标容器 |
| `icons/icon.ico` | Windows/NSIS 应用图标 |

当前 `icon.ico` 保留 Tauri CLI 生成版，SHA-256 为 `4ED646898FF0F8C14FD6D2E6925AFE8C42D09BEF05C6481ED52A63CC4F89B3B6`，内含 16、24、32、48、64 和 256 px 图层。不要直接换成 ZIP 内的 `VALOFRAME_AppIcon.ico`；若要改变生成链路，应重新完成 Windows 安装器、任务栏、开始菜单、快捷方式和卸载项的视觉验收。

`icon.png`、`64x64.png`、`Square*Logo.png`、`StoreLogo.png`、`android/` 和 `ios/` 同样是由上述命令生成的派生物。当前 Windows NSIS 配置不会消费全部平台资产，但保留它们以维持可复现的 Tauri 图标输出。

仓库外层的网页资源采用同一套品牌资产：

- `public/favicon.ico` 与原始包内的 `favicon.ico` 逐字节一致。
- `public/valoframe-mark.png` 与原始包内的透明 64 px PNG 逐字节一致，用于应用顶栏。

## 为什么未采用 opaque 版本

桌面外壳、任务栏、快捷方式和应用内品牌标记需要透明边缘。opaque 版本会把背景固化进正方形画布，在不同 Windows 主题和表面上形成不必要的底色块，因此当前不用于应用图标。它仍保留在原始 ZIP 中，可供明确要求不透明画布的平台另行使用。

## 权利确认记录

2026-07-18，仓库负责人确认其拥有或已经取得将原始图标包及本目录派生资产复制、修改并发布到公开 GitHub 仓库的权利。该确认覆盖本仓库中的 `src-tauri/icons/**`、`public/favicon.ico` 与 `public/valoframe-mark.png`，原始 ZIP 仍只在仓库外归档。

此记录满足公开源代码审阅所需的品牌资产授权确认，但不单独批准安装器、应用商店或商业分发。若进入这些渠道，仍需按 Windows 发布清单归档适用的设计源文件、合同或许可证，并完成法律与品牌审核。
