# Git Workflow

本文档定义本项目的 Git 维护规则。目标是让仓库历史可追溯、提交可回滚、隐私数据不误入版本库。

## 1. 仓库基线

- `main` 是稳定基线分支，应始终保持可构建、可测试。
- 首次提交只纳入项目资产：应用源码、测试、文档、锁文件、Tauri/Rust 配置、必要图标。
- 构建产物、缓存、本地工具状态、真实游戏日志、真实账号数据、一次性分析结果不得提交。
- 可复用脚本应放入 `scripts/`，附用途说明；临时调查脚本留在本地并由 `.gitignore` 忽略。

## 2. 分支规则

日常开发从 `main` 拉新分支：

```powershell
git switch main
git pull --ff-only
git switch -c feature/<short-topic>
```

推荐分支名前缀：

- `feature/`：新功能。
- `fix/`：缺陷修复。
- `docs/`：文档更新。
- `chore/`：构建、依赖、仓库维护。
- `codex/`：Codex 辅助完成的任务分支。

避免在 `main` 上直接做较大开发。紧急小改也应先确认工作区干净。

## 3. 提交规范

提交信息采用 Conventional Commits 风格：

```text
<type>: <short summary>
```

常用 `type`：

- `feat`：用户可见的新能力。
- `fix`：缺陷修复。
- `docs`：文档。
- `test`：测试。
- `refactor`：不改变行为的重构。
- `build`：构建系统或依赖。
- `chore`：仓库维护、工具配置。

示例：

```text
docs: add git workflow and maintenance rules
chore: establish initial project baseline
fix: preserve account label when parsing metadata
```

每个提交应尽量只表达一个意图。不要把功能、格式化、实验输出和本地数据混在同一个提交里。

## 4. 提交前检查

涉及前端时运行：

```powershell
npm test
npm run build
```

涉及 Rust/Tauri 后端时运行：

```powershell
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo clippy --manifest-path src-tauri/Cargo.toml --locked --all-targets --all-features -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml --locked --all-targets --all-features
```

只改文档或 Git 配置时，至少运行：

```powershell
git status --short --branch
git diff --check
```

如果某项检查因环境缺失无法运行，应在提交或交付说明中写明原因。

## 5. 暂存规则

提交前先看状态：

```powershell
git status --short
```

优先按路径精确暂存：

```powershell
git add README.md docs/GIT_WORKFLOW.md .gitignore .gitattributes
```

在确认 `.gitignore` 已覆盖本地产物后，才使用：

```powershell
git add .
```

提交前复查暂存内容：

```powershell
git diff --cached --stat
git diff --cached --check
```

## 6. 隐私与数据边界

不得提交：

- `AppData/ACLOS` 真实日志、LevelDB、WonderfulDb、导出的对局事件。
- 包含真实账号、OpenID、RoleID、玩家标签或本机路径的原始数据。
- 本地数据库、缓存、截图比对、一次性 QA 输出。
- `.codex/`、`.claude/`、`.agents/`、`.superpowers/` 等个人工具状态。

测试需要样本时，使用脱敏 fixture，放在 `tests/fixtures/`，并在文件头或 README 中说明字段来源和脱敏方式。

## 7. 锁文件规则

- 前端统一使用 npm；`package-lock.json` 必须随依赖变化提交，不得新增其他包管理器锁文件。
- `src-tauri/Cargo.lock` 必须提交，因为这是桌面应用，不是 Rust library crate。
- 依赖升级单独成提交，提交说明中写明升级原因和验证命令。

## 8. 评审与合并

合并前应提供：

- 变更摘要。
- 已运行的验证命令及结果。
- UI 变更的截图或说明。
- 数据迁移、隐私边界、兼容性影响。

合并时优先使用 squash 或线性历史，保持 `main` 简洁可读。
