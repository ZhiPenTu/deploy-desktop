# 外部命令与跨平台清单

本文记录项目在本机执行的外部命令、平台可用性与处理方案。新增或修改外部命令时同步更新本表，并按 [AGENTS.md](../AGENTS.md) 的「外部命令与跨平台」要求处理。

## 判定标准

| 类别 | 要求 |
| --- | --- |
| 三平台通用 | Windows 10+、macOS、主流 Linux 均自带或随产品依赖提供，可直接使用 |
| 平台分支 | 只在单一平台存在，必须按平台分支，并在不支持的平台给出明确提示 |
| 应改进程内 | 关键步骤不应依赖外部工具，改为 Rust / Node 实现，避免平台差异导致失败 |
| 仅远程 | 只在 Linux 目标机通过 SSH 执行，可用 POSIX 工具 |

## 桌面端（Rust）本机命令

| 命令 | 位置 | 用途 | 平台情况 | 结论 |
| --- | --- | --- | --- | --- |
| `docker` / `docker-compose` | `docker_setup.rs`、`docker_ops.rs`、`deployment_images.rs`、`registry.rs` | 镜像与容器操作 | 三平台均需用户安装 | 保留，作为产品前置依赖 |
| `docker-credential-*` | `deployment_images.rs` | 读取凭据助手 | 随 Docker 提供 | 保留 |
| `ssh`、`ssh-keygen` | `remote_ssh/client.rs`、`remote_ssh/auth.rs` | SSH 连接与密钥 | Windows 10+ 自带 OpenSSH 客户端 | 保留，缺少时提示安装 |
| `sshpass` | `remote_ssh/auth.rs` | 密码方式下发公钥 | 仅 Unix | 已按文件存在性判断；Windows 返回「请改用密钥登录」提示 |
| `expect` | `remote_ssh/auth.rs` | 密码交互回退 | 仅 Unix | 同上 |
| `open` / `xdg-open` / `rundll32.exe` / `explorer` | `projects.rs`、`logs.rs`、`docker_setup.rs`、`artifact/mod.rs` | 打开目录、链接 | 已按平台分支 | 保留；Windows 分支统一隐藏控制台窗口 |
| `sw_vers`、`sysctl` | `env.rs` | macOS 版本与内存 | 已在 macOS 分支内 | 保留 |
| `powershell` | `core/disk.rs` | Windows 磁盘空间 | 已在 Windows 分支内 | 保留，隐藏窗口 |
| `tar`、`cp`、`chmod`、`systemctl` | `docker_engine.rs` | 离线安装 Docker 引擎 | 函数开头已限定 Linux | 保留 |
| `bash` | `remote_ssh/env.rs`（`find_deploy_script` 路径） | 执行局域网部署脚本 | Windows 默认没有 bash | 该功能应仅对 macOS / Linux 开放，界面明确说明 |

## MCP 服务（Node）

| 命令 | 位置 | 平台情况 | 结论 |
| --- | --- | --- | --- |
| `docker`、`docker-compose` | `deploy.mjs` | 三平台 | 保留，缺失时提示安装 |
| `sw_vers` | `deploy.mjs` | 仅 macOS，已在 `darwin` 分支 | 保留 |
| `tar`、`cp`、`systemctl` | `deploy.mjs` | 仅 Linux 且已限定 | 保留 |
| `bash`（`runBashFile`） | `shell.mjs`、`deploy.mjs` | Windows 默认没有 | 与桌面端一致，局域网部署仅 macOS / Linux |
| `ssh`、`sshpass` | `ssh.mjs` | `sshpass` 仅 Unix，已做存在性判断 | 保留 |

磁盘空间统一使用 `fs.statfs`，不再回退到仅 Unix 可用的 `df`。

## 远程脚本（`shell/*.sh`）

`shell/` 下的脚本只通过 SSH 在 Linux 目标机执行，可使用 `tar`、`gzip`、`sha256sum`、`base64`、`awk`、`find`、`mkdir`、`timeout` 等 POSIX 工具。新增依赖时同步更新目标机前置检查与界面提示。

## 进程内替代记录

| 能力 | 原实现 | 现实现 | 原因 |
| --- | --- | --- | --- |
| 交付包解包 | 系统 `tar` | Rust `flate2` + `tar`（`artifact/download.rs`）、Node `zlib`（`mcp-server/src/tar-extract.mjs`） | Windows tar 对 UTF-8 文件名与退出码处理不一致，导致解包失败 |
| Base64 编码 | `base64` 命令 | Rust 进程内实现（`remote_ssh/client.rs`） | Windows 默认没有 `base64` 命令 |
| 目录上传打包 | 本机 `tar` | `core/process.rs` 的 `Input::Directory` 流式打包 | Windows bsdtar 与 GNU tar 参数、路径处理不一致 |
| 磁盘空间 | `df` / PowerShell 重复实现 | `core/disk.rs` 统一实现 | 避免各入口重复且 Windows 上 `df` 不可用 |
| 端口占用进程 | `lsof` | Unix `lsof`，Windows `Get-NetTCPConnection` | Windows 没有 `lsof` |
| 打开外部链接 | `cmd /C start` | `rundll32 url.dll,FileProtocolHandler` | 避免 Windows 弹出控制台窗口 |

## 剩余事项

1. `remote_ssh/env.rs` 的本地局域网部署脚本仅支持 macOS / Linux，Windows 会返回明确提示；如需 Windows 支持，需把脚本能力迁移到 Rust。
2. Windows 密码方式下发公钥不可用，需改用密钥登录（界面已提示）。
3. Windows 平台分支的控制台隐藏与 PowerShell 查询需在 Windows 实机构建后复测。

## 验证方式

- 新增外部命令时，至少确认 macOS 与 Windows 两个平台的行为；仅 Linux 可用的命令必须在代码中显式限定并给出提示。
- 前端不直接执行外部命令；所有调用经 `src/api/tauri.js` 进入 Rust。
- 相关检查：`cargo check --manifest-path src-tauri/Cargo.toml`、`cargo test --manifest-path src-tauri/Cargo.toml`、`npm run test:mcp`。
