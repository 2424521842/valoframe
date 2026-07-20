# Contributing

感谢你帮助改进瓦刻。当前仓库处于开发预览，优先接受可复现缺陷、测试证据和范围明确的小改动。

## 开始前

- 先搜索现有 Issue 和 Draft PR，避免重复工作。
- 功能建议先创建 Issue，说明用户问题、边界和验收方式。
- 不要提交真实用户素材、数据库、日志、路径或身份信息。
- 不要加入来源不明、许可不清或缺少可复现 provenance 的二进制、字体、图片、游戏素材或其他第三方内容。

## 本地验证

```powershell
npm ci
npm test
npm run build
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo clippy --manifest-path src-tauri/Cargo.toml --locked --all-targets --all-features -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml --locked --all-targets --all-features
git diff --check
```

真实 ACLOS 数据回归保持手工执行；公开提交只能使用脱敏或合成 fixture。

## Pull Request

- 保持一次 PR 只解决一个清晰问题。
- 描述动机、用户影响、数据/文件安全影响和验证结果。
- 涉及扫描、数据库、永久删除、安装器、FFmpeg 或第三方素材时，列出失败模式与回滚方式。
- 新增依赖或素材时，提供来源、版本、许可证、哈希和适用分发范围；“能下载”不等于“允许再分发”。

提交贡献即表示你有权按仓库适用许可提供该改动。项目自有代码和文档采用 MIT；第三方与品牌内容仍受各自条款约束。
