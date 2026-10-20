# 贡献指南

## 开发环境

前端使用 Vue 3、Vite 和 Element Plus，桌面端使用 Tauri 2 / Rust。

准备 Node.js 20+（CI 使用 22）、Rust stable 和对应平台的 [Tauri 系统依赖](https://v2.tauri.app/start/prerequisites/)。

```bash
npm ci
npm run tauri dev
```

`npm run dev` 仅用于浏览器界面预览。没有桌面运行环境时，主机、制品源和镜像源列表为空；Docker、SSH、部署、文件操作与应用更新均提示暂不可用，不返回演示数据或成功结果。

浏览器预览与测试替身只能验证界面及调用约定，不能证明真实 Docker、SSH、数据库或目标环境部署成功。

## 提交前检查

```bash
npm run check:ui-text
npm run check:boundaries
npm run test:api
npm run build
cargo test --manifest-path src-tauri/Cargo.toml
```

涉及 Rust 代码时额外执行：

```bash
cargo check --manifest-path src-tauri/Cargo.toml
```

涉及 MCP 时执行：

```bash
npm --prefix mcp-server ci
npm run test:mcp
npm --prefix mcp-server run smoke
```

界面变更还需检查实际渲染和关键交互，确认提示没有开发说明或虚假的成功状态。文案检查脚本不能替代人工检查。

## 构建安装包

在对应平台安装系统依赖后执行：

```bash
npx tauri build --bundles dmg           # macOS Apple Silicon
npx tauri build --bundles nsis,msi      # Windows
npx tauri build --bundles appimage,deb  # Linux
```

Tauri 会先构建前端，安装包位于 `src-tauri/target/release/bundle/`。PR 检查与发布流程均构建 macOS Apple Silicon、Windows 和 Linux；不再构建 macOS Intel 安装包。

### macOS 开发版

直接双击一个手工拼装的 `.app` 时，如果签名之后又覆盖了 `Contents/MacOS` 里的可执行文件，macOS 的 taskgated 会在启动瞬间以 `Code Signature Invalid` 直接杀掉进程（崩溃报告为 `SIGKILL (Code Signature Invalid)`）。因此开发版应用统一用脚本生成，脚本会先写入全部文件，最后再做 ad-hoc 签名并校验：

```bash
npm run dev:app         # 构建并打开 src-tauri/target/dev-app/DeployDev.app
npm run dev:app:build   # 只构建不启动
bash scripts/dev-app.sh --release --offline --launch
```

`dev:app` 会以 `com.deploydesktop.devtool` 作为 Bundle ID 单独运行，与已安装的 `com.deploydesktop.app` 互不影响。需要更新这个开发版时，重新执行脚本即可，不要手工把 `src-tauri/target/debug/deploy-desktop` 复制进已签名的 `.app`。

## 应用更新与发布

应用按需读取 GitHub Releases 的 `latest.json`，更新包须通过 `src-tauri/tauri.conf.json` 中的 updater 公钥校验。检查更新不会在启动时自动触发。

维护者首次配置签名时，用 `npx tauri signer generate -w ~/.tauri/deploy-desktop.key` 生成密钥，将公钥配置到 `plugins.updater.pubkey`。私钥及口令只保存在本机或 CI Secrets 中，不得提交。

发布流程见 [.github/workflows/release-desktop.yml](.github/workflows/release-desktop.yml)：推送 `v*` 标签后，使用 `TAURI_SIGNING_PRIVATE_KEY` 和 `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` 两项 Secrets 构建安装包、签名更新产物及 `latest.json`，创建草稿 Release。维护者检查资产后再正式发布。

[安装包检查流程](.github/workflows/build-desktop.yml) 只运行 PR / 手动构建，上传 Actions 产物；发布统一由上述标签流程负责。

## 代码约定

功能目录、前端调用层、Rust 部署模块与 MCP 注册边界见 [架构说明](docs/architecture.md)。`npm run build` 会先检查文案和模块依赖边界。

- 界面文案使用简体中文，不添加 emoji、颜文字或装饰性符号。
- 前端只通过 `src/api/tauri.js` 调用 Tauri 命令。
- 容器、端口、镜像与默认路径统一维护在 `profile.json`，不要在前端或 Rust 中重复硬编码。
- 镜像引用必须使用 `sha256` digest。
- 不在仓库中提交真实 IP、凭据、客户信息或内部路径。
- 不提交 `node_modules/`、`dist/`、`src-tauri/target/` 与安装包产物。

## 提交信息

沿用 `feat(scope): 描述`、`fix(scope): 描述`、`docs: 描述` 风格。
