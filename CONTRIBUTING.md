# 贡献指南

## 开发环境

```bash
npm install
npm run tauri dev
```

## 提交前检查

```bash
npm run check:ui-text
npm run build
cargo test --manifest-path src-tauri/Cargo.toml
```

涉及 Rust 代码时额外执行：

```bash
cargo check --manifest-path src-tauri/Cargo.toml
```

## 代码约定

- 界面文案使用简体中文，不添加 emoji、颜文字或装饰性符号。
- 前端只通过 `src/api/tauri.js` 调用 Tauri 命令。
- 容器、端口、镜像与默认路径统一维护在 `profile.json`，不要在前端或 Rust 中重复硬编码。
- 镜像引用必须使用 `sha256` digest。
- 不在仓库中提交真实 IP、凭据、客户信息或内部路径。
- 不提交 `node_modules/`、`dist/`、`src-tauri/target/` 与安装包产物。

## 提交信息

沿用 `feat(scope): 描述`、`fix(scope): 描述`、`docs: 描述` 风格。
