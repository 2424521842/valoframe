# 应用内更新

瓦刻的稳定更新通道面向 Windows x64。`v0.2.1` 是用户需要手动安装一次的 updater 起始版本；`v0.2.2` 是第一次用于验证应用内更新的版本。此后每个稳定版本都通过 GitHub Release 的 `latest.json`、Tauri 更新包和签名完成升级。

Community Beta、内部 RC、prerelease 和未配置公钥的开发构建不进入稳定更新通道。

## 用户流程

- 设置中心的“更新”分类显示当前版本，并支持自动或手动检查更新。
- 发现更高的稳定版本后，用户确认下载；下载过程显示进度并允许取消或重试。
- 下载包不得超过 512 MiB，且必须通过 Tauri 签名验证。取消、断网、签名失败或安装器启动失败不会修改当前安装和用户数据。
- 下载完成后再次确认安装。扫描、永久删除、视频导出或来源根重新定位进行中时暂不安装，任务结束后可以重试。
- 不允许忽略签名、安装 prerelease 或降级。

`v0.1.0-beta.1` 没有可用的稳定 updater 配置，因此无法自行升级到 `v0.2.1`。旧用户必须从 GitHub Release 手动下载安装 `v0.2.1`；从该版本开始，后续 `v0.2.2` 及更高稳定版本可在应用内更新。

## 信任链和构建配置

稳定端点固定为：

`https://github.com/2424521842/valoframe/releases/latest/download/latest.json`

Tauri/Minisign 更新签名是稳定更新的强制安全要求：

- 构建时通过 `VALOFRAME_UPDATER_PUBLIC_KEY` 嵌入公钥。
- 发布任务通过 `TAURI_SIGNING_PRIVATE_KEY` 和 `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` 签署更新包。
- 私钥和密码不得写入仓库、Release、日志或构建产物。
- 没有注入公钥的普通开发构建保持 updater 未配置状态并拒绝检查。
- 本地 `start-dev.bat` 在 `release-secrets/valoframe-updater.key.pub` 存在时自动将该公钥注入当前开发构建；文件缺失时仍保持 updater 未配置，脚本绝不读取私钥或密码。
- 清单版本、GitHub 仓库/tag 下载路径和 ZIP 内安装器版本必须一致，避免把旧的合法签名包重放为更高版本。

Windows Authenticode 是可选的信誉增强，不是个人开发者稳定 updater 发布的前置条件。没有 Authenticode 时，Windows 可能显示“未知发布者”或 SmartScreen 提示；下载页和发布说明必须如实提醒用户。以后购买代码签名证书时可以补充 Authenticode，而无需更换 Tauri updater 密钥。

## 一次性密钥配置

在可信的本地环境先创建仅供当前用户使用的密钥目录，再执行 Tauri signer 生成命令并设置强密码。不要把私钥内容输出到终端、聊天或日志：

```powershell
New-Item -ItemType Directory -Force "$env:USERPROFILE\.tauri" | Out-Null
npm run tauri signer generate -- -w "$env:USERPROFILE\.tauri\valoframe-updater.key"
```

随后完成以下配置：

1. 通过 GitHub 的 Secret 配置界面把 `.key` 文件的完整内容保存为仓库 Secret `TAURI_SIGNING_PRIVATE_KEY`，不要使用会把内容打印到控制台的命令。
2. 把私钥密码保存为 GitHub 仓库 Secret `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`。
3. 把 `.key.pub` 文件的完整内容保存为 GitHub 仓库 Variable `VALOFRAME_UPDATER_PUBLIC_KEY`。
4. 把私钥和密码另存一份加密离线备份，并验证备份可以读取；公钥可单独记录指纹。
5. 删除不再需要的临时明文副本，不要提交密钥文件。

仓库不包含真实密钥。本节只是配置说明；只有维护者完成上述步骤后，稳定发布任务才能运行。任一 Secret 或 Variable 缺失时，workflow 必须在构建前明确失败。

## 发布流程

发布稳定版本时：

1. 同时更新 `package.json`/`package-lock.json`、Cargo manifest/lockfile 和 `tauri.conf.json` 的应用版本，确保完全一致。
2. 为该版本准备 `release/notes/vX.Y.Z.md`；文件不存在时 workflow 使用默认更新说明。
3. 提交并推送代码，再创建并推送规范标签，例如 `git tag v0.2.2` 和 `git push origin v0.2.2`。
4. `stable-release` workflow 自动测试、构建和签名，创建 draft Release，上传安装器、`.nsis.zip`、`.sig` 与 `latest.json`。
5. workflow 从 draft 重新下载并验证远端资产，通过后才发布为稳定 latest；公开后二次验证失败时恢复为 draft。

标签必须是精确的 `vMAJOR.MINOR.PATCH`，且版本高于已有稳定 Release。标签与任一 manifest 不一致、Release 已存在、密钥缺失、包超过 512 MiB、签名或下载地址验证失败时均拒绝发布。

## 失败和重试

- 构建或验证失败：修复原因后删除失败 run 留下的同版本 draft Release；确认标签仍指向预期提交，再在 GitHub Actions 中重新运行失败任务。
- 源码需要修改：不要移动已经公开的标签。删除尚未发布的标签和 draft，提交修复后重新创建同名标签；若版本已公开，则递增补丁版本。
- 已公开后的验证失败：确认 workflow 已把精确 Release 恢复为 draft，并检查 `/releases/latest/` 已离开该版本，再调查后重试或发布更高补丁版本。
- updater 私钥丢失且无可用备份时，已安装版本无法信任新密钥签署的更新；需要用户再次手动安装嵌入新公钥的版本。因此离线备份是发布前必做项。

## 首次验收

1. 手动安装嵌入正式公钥的 `v0.2.1`。
2. 发布 `v0.2.2`，确认应用发现新版本、下载、验签、启动安装器并在重启后显示 `0.2.2`。
3. 验证取消、断网、错误公钥、篡改包、安装器启动失败和禁止降级，确保当前安装及用户数据保持可用。
4. 在无 Authenticode 的机器上确认“未知发布者”提示与发布说明一致。

现有前端 updater 命令、Rust 状态机、关键任务门禁和 endpoint 不因发布流程精简而改变。
