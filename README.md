# 部署舱（deploy-desktop）

面向 Docker Compose 项目的桌面部署助手，支持本机与 SSH 主机的离线部署、制品更新、镜像管理和运行诊断。另提供独立的 stdio MCP 服务，供支持 MCP 的客户端调用。

## 安装

从 [GitHub Releases](https://github.com/ZhiPenTu/deploy-desktop/releases) 下载对应平台的安装包：

| 平台 | 安装包 |
| --- | --- |
| macOS Apple Silicon | DMG |
| Windows | NSIS / MSI |
| Linux | AppImage / DEB |

当前构建与发布流程不提供 macOS Intel 安装包。应用内可点击「检查更新」获取新版本，更新包安装前会校验签名。

部署目标需要 Docker 和 Compose；SSH 主机需要配置密钥登录。离线环境需事先准备交付包、基础镜像及所需的 Docker 安装介质。

## 主要功能

| 入口 | 用途 |
| --- | --- |
| 项目管理 | 一个项目对应一套部署目标（主机 + 部署目录 + 制品源），以卡片展示，可直接进入部署、监控或日志 |
| 凭证管理 / 主机管理 | 管理本机与 SSH 主机，项目通过编辑选择目标主机，主机不再主动抢占当前项目 |
| 主机状态 | 检查系统资源、Docker、Compose 与端口占用 |
| 部署向导 | 选择项目与制品、核对服务和镜像，执行首次部署或增量更新 |
| 制品中心 | 从云效流水线或 HTTPS 直链获取交付包，按来源管理版本与本地缓存 |
| 镜像与离线包 | 拉取、导出、导入和分发镜像，管理本机离线包 |
| 镜像仓库 | 按 `deploy.desktop/v1` 清单拉取固定 digest 的镜像 |
| 集群监控 / 日志诊断 | 从项目卡片进入，查看当前项目的容器状态和日志 |

支持兼容 Docker Registry V2 的镜像仓库，例如 ACR、Harbor 和 GHCR。镜像拉取要求 `sha256` digest，拒绝仅 tag 的引用。

## 开始部署

1. 在「项目管理」新建项目，选择主机、部署目录与制品源；字段见 [部署配置与主机管理](docs/configuration.md)。生产与测试各建一个项目。
2. 在「主机管理」新增或编辑主机；项目在「项目管理」的编辑弹窗中选择目标主机。通过主机「状态」确认目标环境。
3. 在「制品中心」选择来源和版本，下载交付包后点「部署」进入部署向导；也可直接在部署向导选择本地压缩包或已解压目录。
4. 按向导完成「选择项目、检测环境、选择制品、解压制品、清单比对、选择更新」。基础镜像缺失时，先通过镜像仓库或离线包准备镜像。
5. 执行前确认目标目录、版本与影响范围。已有部署走增量更新，默认备份；完成后检查容器状态和日志。

交付包以 `bundle.json`（`deploy.desktop/bundle/v2`）声明产物映射、组件、服务、数据库、镜像与部署步骤，应用产物通过挂载方式交付，基础镜像独立准备。首次部署不会自动导入全量 SQL。字段、校验规则、首次部署与增量更新的边界见 [制品源与交付包](docs/artifact-source.md)。

## MCP 服务

需要 Node.js 20+，在仓库目录安装并启动：

```bash
npm --prefix mcp-server ci
npm run mcp
```

客户端配置示例：

```toml
[mcp_servers.deploy-desktop]
command = "node"
args = ["/absolute/path/to/deploy-desktop/mcp-server/index.mjs"]
env = { DEPLOY_DESKTOP_ROOT = "/absolute/path/to/deploy-desktop" }
```

MCP 提供主机、仓库、清单、镜像、离线包和部署运维工具，使用独立的本地配置，仓库密码不写入配置文件。工具列表、确认参数和使用限制见 [MCP 文档](mcp-server/README.md)。其中 `deploy_remote_run` 依赖外部项目的 `deploy-lan.sh`，本仓库不包含该脚本。

## 开发

前端为 Vue 3 + Vite + Element Plus，桌面端为 Tauri 2 / Rust。

```bash
npm ci
npm run tauri dev
```

`npm run dev` 仅预览界面；Docker、SSH、部署及更新等操作需在桌面应用中执行。环境要求、构建、验证和发布说明见 [贡献指南](CONTRIBUTING.md)。

## 文档

- [部署配置与主机管理](docs/configuration.md)
- [制品源、交付包与部署向导](docs/artifact-source.md)
- [镜像源管理](docs/registry-sources.md)
- [镜像清单规范](docs/manifest-schema.md)与[清单示例](examples/manifest.example.yaml)
- [交付包描述符示例](examples/bundle.v2.json)
- [MCP 服务](mcp-server/README.md)
- [功能目录与调用边界](docs/architecture.md)
- [项目协作规则](AGENTS.md)与[贡献指南](CONTRIBUTING.md)

## 数据与安全

- SSH 私钥保留在本机，应用只记录路径。桌面端保存的仓库密码、SSH 初始密码和云效令牌存放在独立的本机凭据文件中，Unix 权限为 `0600`，未额外加密。
- Docker 登录使用临时配置；仅供本次操作的仓库密码不持久保存。真实拓扑与凭据应保存在用户配置目录，不能提交到仓库。
- 交付包仅允许 HTTPS 下载，云效来源额外限制阿里云域名；下载与解包会检查源提供的摘要、路径安全、包结构和版本。
- 镜像清单支持 HTTP(S) 和本地文件，最大 5 MB，建议使用 HTTPS。拉取后校验镜像 digest。
- 本工具不采集遥测，不上传部署数据。安全问题请按 [SECURITY.md](SECURITY.md) 私下报告。

## 许可证

[Apache License 2.0](LICENSE)。
