# 瓦刻（VALOFRAME）

> **非官方社区项目。** 瓦刻是在遵循 Riot Games [“Legal Jibber Jabber”政策](https://www.riotgames.com/en/legal)的前提下创建的免费工具。Riot Games 不认可或赞助本项目；本项目也与 Riot Games、腾讯及其关联公司不存在隶属、赞助或认可关系。VALORANT、《无畏契约》及相关名称、商标和游戏内容归其各自权利人所有。

> **许可状态。** 本仓库目前未授予开源许可证；除另有声明的第三方材料外，默认保留全部权利。公开可见不等于允许复制、修改或再分发。项目许可仍是正式发布前的待确认项。

`valorant-highlight-manager` 是“瓦刻（VALOFRAME）”的代码仓库。这是一款 Tauri 2 + React + TypeScript + Rust 桌面应用，用于在默认不改动原始文件的前提下索引和管理无畏契约国服高光素材；只有用户在应用回收站再次明确确认“永久删除”时，才会删除所选本地视频。

当前核心链路已覆盖多来源扫描、四路元数据合并、稳定账号分组、分页与分层虚拟化浏览、按需详情、视频预览，以及收藏、标签、备注和批量整理。完整边界见 [PRD](docs/PRD.md)。

## 技术栈

- Tauri 2
- React 19
- TypeScript
- Rust
- SQLite via `rusqlite`
- Node.js/npm

## 本地运行

确保 Windows 环境已安装：

- Node.js 24 与 npm 11
- Rust 1.96.1 MSVC toolchain（由 `rust-toolchain.toml` 固定）
- Microsoft Visual Studio C++ Build Tools
- Microsoft Edge WebView2 Runtime

项目只使用 npm，并以 `package-lock.json` 作为唯一前端锁文件。安装锁定依赖：

```powershell
npm ci
```

启动桌面开发环境：

```powershell
npm run tauri -- dev
```

验证前端测试与构建：

```powershell
npm test
npm run build
```

验证 Rust 后端：

```powershell
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo clippy --manifest-path src-tauri/Cargo.toml --locked --all-targets --all-features -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml --locked --all-targets --all-features
```

GitHub Actions 使用同一组门槛：Ubuntu 上执行 `npm ci`、`npm test` 和前端构建，Windows 上执行 Rust 格式检查、严格 Clippy 和完整测试，配置见 [`.github/workflows/ci.yml`](.github/workflows/ci.yml)。

## 当前能力

- 来源与扫描：默认目录、手动目录和全电脑发现；`scan_roots` 统一处理多根目录、扫描互斥、进度、终态和取消。
- 四路元数据：WonderfulDb、`videoExportTmp/config-*.json`、`highlight.log` 和 Local Storage LevelDB；单路失败只降级并记录警告。
- 稳定账号：优先使用 `matchAccountId/openid` 识别账号，玩家名只用于展示和搜索。
- 真实来源：`list_sources` 从 `source_dirs` 返回路径、名称、状态、错误和素材数量，包括零素材来源。
- 大库浏览：后端分页摘要与全库 facets、详情按需加载、跨页分组，以及对局组与极端单组视觉行的分层虚拟化。
- 素材管理：搜索、筛选、排序、收藏、标签、备注和回收状态；批量收藏、标签和回收由后端单命令、单事务执行。
- 本地预览：自定义媒体协议提供最大 1 MiB 的有界 Range 响应；无 Range 时也不会把整个大视频读入内存。
- 缩略图：源目录封面优先；存在受控 FFmpeg 时，缺失封面由单 worker 持久队列生成到应用缓存，按视频指纹失效，并在 512 MiB 高水位时清理到 450 MiB；否则稳定回退。
- 文件安全：扫描、预览和常规整理只读原始目录；只有回收站中经过不可撤销确认的“永久删除视频”会删除所选原视频。
- 自动化验证：Node/React 测试、Rust 单元与集成测试、npm 构建和 GitHub Actions CI；Windows 另有固定 FFmpeg、npm/Cargo SPDX 与许可证据、未签名 NSIS 内部 RC、manifest 驱动静态 bundle 检查和 marker 隔离启动烟测。

## 尚未完成

- 锁定依赖的 SPDX/第三方材料生成器和最小自建 FFmpeg 候选链已经落地，但缺失许可证文本的受审 override、最小候选真实视频回归/长期源码镜像，以及项目许可、专利与法律审批尚未闭合，因此不能对外分发。
- 正式 publisher/identifier/项目许可和品牌资产、Authenticode 签名、一次性 VM 的安装升级卸载、公开分发与自动更新。

## 明确不做

- 不实时录制。
- 不读取游戏进程。
- 不读取游戏内存。
- 不绕过文件权限；WonderfulDb 仅只读读取并在内存中解密，不回写源文件或生成独立明文副本。
- 除用户在回收站明确确认永久删除外，不修改、移动、重命名或删除原始素材。
- 不上传素材或索引数据。

## 文档

- [PRD](docs/PRD.md)
- [Architecture](docs/ARCHITECTURE.md)
- [Data Model](docs/DATA_MODEL.md)
- [Metadata Ingestion](docs/METADATA_INGESTION.md)
- [Roadmap](docs/ROADMAP.md)
- [Tasks](docs/TASKS.md)
- [Git Workflow](docs/GIT_WORKFLOW.md)
- [Windows Release](docs/RELEASE.md)
- [Windows Release Checklist](docs/WINDOWS_RELEASE_CHECKLIST.md)
- [Database Recovery](docs/DATABASE_RECOVERY.md)
