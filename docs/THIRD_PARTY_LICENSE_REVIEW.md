# 第三方许可证正文补充审核

## 目的与边界

锁定的 npm/Cargo 依赖均声明了许可证，但其中 1 个 npm 包和 11 个 Cargo crate 的发布归档没有携带许可证正文。`third_party/licenses/license-text-overrides.json` 为这些精确版本提供离线、哈希固定的上游正文和来源证据，使生成的安装包能够包含对应条款。

这些 override 是技术证据，不是自动法律批准。正文、锁文件、组件版本、Cargo registry checksum、`.cargo_vcs_info.json` 提交、来源 URL、文件大小或 SHA-256 任一变化都会使合规生成失败。生成器还要求这些文件已经进入 Git index、工作区字节与 index 一致，且路径不经过 symlink/junction。

## 待审核映射

| 组件 | 声明 | 补充证据 | 当前结论 |
| --- | --- | --- | --- |
| `react-remove-scroll-bar@2.3.8` | MIT | npm cache 中的发布 tarball 同时匹配 lock SHA-512 与 registry SHA-1；发布包本身没有 LICENSE。registry `gitHead` 已记录但当前上游不可访问，正文来自后续官方仓库固定提交 | 待负责人确认“后续官方许可证澄清”可作为该版本通知文本；不能把不可访问的 `gitHead` 表述成已直接验证 |
| `alloc-stdlib@0.2.4` | BSD-3-Clause | `.cargo_vcs_info` 对应 Dropbox 上游提交根 LICENSE | 待审核 |
| `selectors@0.36.1` | MPL-2.0 | crate 源文件的 MPL-2.0 header，加 Mozilla 官方 MPL 2.0 正文 | 待审核；仍需确认 MPL 源代码形式义务 |
| `tauri-plugin@2.6.3` | Apache-2.0 OR MIT | `.cargo_vcs_info` 对应 Tauri 提交的 Apache/MIT 正文 | 待审核 |
| 五个 `unic-* @0.9.0` | MIT OR Apache-2.0 | 各 crate 的锁定 UNIC 提交、Apache/MIT 正文与 COPYRIGHT；两个上游提交的对应文件已记录为逐字节相同来源 | 待审核 |
| 三个 `webview2-com*` | MIT | 各 crate 的锁定 webview2-rs 提交和 MIT LICENSE；两个上游提交的 LICENSE 已记录为逐字节相同来源 | 待审核 |

精确组件、checksum、提交、路径、正文来源和哈希以 [`third_party/licenses/license-text-overrides.json`](../third_party/licenses/license-text-overrides.json) 为准。

## 审批决定

当前状态：**待负责人/法律审核**。`third_party/licenses/license-text-override-approvals.json` 当前没有任何批准记录，因此 12 项 override 均保持 pending。

批准某项映射时，不直接改 override 状态；应在独立 approval manifest 中增加结构化记录，绑定完整组件标识、声明的 SPDX 表达式、排序后的正文 SHA-256、明确的 `approved` 决定、审核人、UTC 时间和可追溯引用。缺字段、未知组件、重复/过期记录或正文哈希不一致都会使生成失败。在明确批准前，生成器会继续输出 `NPM_LICENSE_OVERRIDE_REVIEW_PENDING` 或 `CARGO_LICENSE_OVERRIDE_REVIEW_PENDING`。

即使这 12 项全部批准，也不得自动把 `release/public-release-policy.json` 中的 `thirdPartyCompliance.approved` 改为 `true`。FFmpeg 对应源码、第三方许可、专利和最终法律审批仍需独立闭合。
