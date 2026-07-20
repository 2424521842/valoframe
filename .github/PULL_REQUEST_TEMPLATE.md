## 变更概述

<!-- 说明改了什么，以及它解决的用户或工程问题。 -->

## 影响范围

- [ ] 用户界面
- [ ] 扫描或元数据
- [ ] 数据库或迁移
- [ ] 文件操作、回收或永久删除
- [ ] 构建、CI 或安装器
- [ ] 依赖、许可或第三方素材
- [ ] 文档

## 数据与文件安全

<!-- 说明是否读取、写入、移动、导出或删除本地文件；涉及失败场景时说明如何安全退出和恢复。 -->

## 验证

- [ ] `npm test`
- [ ] `npm run build`
- [ ] `cargo fmt --manifest-path src-tauri/Cargo.toml -- --check`
- [ ] `cargo clippy --manifest-path src-tauri/Cargo.toml --locked --all-targets --all-features -- -D warnings`
- [ ] `cargo test --manifest-path src-tauri/Cargo.toml --locked --all-targets --all-features`
- [ ] `git diff --check`
- [ ] 已完成与本改动相关的手工验证

## 第三方与隐私确认

- [ ] 提交不含真实用户素材、数据库、日志、路径或身份信息。
- [ ] 新增依赖或素材已记录来源、版本、许可、哈希和允许的分发范围；不适用时说明原因。
- [ ] 没有把内部验证结果描述成公开发布批准。
