# deploy-desktop

跨平台桌面部署助手：面向 Docker Compose 项目的离线部署、远程更新、镜像清单拉取与离线导出工具。

- 前端：Vue 3 + Vite + Element Plus
- 桌面端：Tauri 2（Rust）
- 支持平台：macOS（DMG）、Windows（NSIS / MSI）、Linux（AppImage / DEB）
- 镜像仓库：任意兼容 Docker Registry V2 的仓库，例如阿里云 ACR、Harbor、GHCR

## 功能

- 本地或 SSH 远程环境自检：CPU、内存、磁盘、Docker、Compose、端口占用
- 一键离线部署：加载本地镜像包、拉起 Compose 服务、导入数据库
- 服务更新：按组件增量更新已部署机器，更新前可选备份
- 镜像清单：按 `deploy.desktop/v1` 清单从镜像仓库拉取指定 digest 的镜像
- 离线导出：拉取后执行 `docker save`，生成可拷贝到内网机器的 tar 包
- 集群监控与日志诊断
- 检查更新：按需从 GitHub Releases 检查最新版本，下载安装后自动重启应用
- 制品中心：从云效流水线或直链获取交付包，校验 MD5 与包结构后作为部署输入

## MCP 服务

仓库内置独立的 stdio MCP 服务 `mcp-server/`，可让 Codex、Claude、Cursor 等支持 MCP 的客户端直接完成仓库配置、镜像拉取、离线包管理、部署与远程更新，无需打开桌面 UI。

```bash
npm --prefix mcp-server install
npm run mcp                 # 直接启动 stdio 服务
npm run test:mcp            # 运行 MCP 单元测试
```

客户端配置示例（Codex `~/.codex/config.toml`）：

```toml
[mcp_servers.deploy-desktop]
command = "node"
args = ["/absolute/path/to/deploy-desktop/mcp-server/index.mjs"]
env = { DEPLOY_DESKTOP_ROOT = "/absolute/path/to/deploy-desktop" }
```

MCP 服务提供 39 个工具，覆盖 SSH 主机、镜像仓库、`deploy.desktop/v1` 清单、镜像拉取/导出/导入、离线包上传、一键离线部署、Compose 运维、数据库导入和远程增量更新。配置默认保存在 `~/.deploy-desktop/mcp.json`，仓库密码通过环境变量或单次调用传入，不写入磁盘。完整工具列表、环境变量、常用流程与安全边界见 [mcp-server/README.md](mcp-server/README.md)。

## 快速开始

环境要求：

- Node.js 20+
- Rust 1.77.2+
- 各平台 Tauri 系统依赖，见 [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/)

```bash
npm install
npm run dev          # 浏览器预览，后端调用返回模拟数据
npm run tauri dev    # 桌面端开发模式
```

构建安装包：

```bash
npm ci
npm run build
npx tauri build --bundles dmg            # macOS
npx tauri build --bundles nsis,msi       # Windows
npx tauri build --bundles appimage,deb   # Linux
```

产物位于 `src-tauri/target/release/bundle/`。

### macOS 本地运行开发版

直接双击一个手工拼装的 `.app` 时，如果签名之后又覆盖了 `Contents/MacOS` 里的可执行文件，macOS 的 taskgated 会在启动瞬间以 `Code Signature Invalid` 直接杀掉进程（崩溃报告为 `SIGKILL (Code Signature Invalid)`）。因此开发版应用统一用脚本生成，脚本会先写入全部文件，最后再做 ad-hoc 签名并校验：

```bash
npm run dev:app         # 构建并打开 src-tauri/target/dev-app/DeployDev.app
npm run dev:app:build   # 只构建不启动
bash scripts/dev-app.sh --release --offline --launch
```

`dev:app` 会以 `com.deploydesktop.devtool` 作为 Bundle ID 单独运行，与已安装的 `com.deploydesktop.app` 互不影响。需要更新这个开发版时，重新执行脚本即可，不要手工把 `src-tauri/target/debug/deploy-desktop` 复制进已签名的 `.app`。

## 镜像清单与仓库

1. 打开“镜像仓库”页面。
2. 填写清单地址（HTTP/HTTPS）或选择本地 YAML/JSON 文件。
3. 点击“加载清单”，确认产品、版本、仓库地址、平台与镜像列表。
4. 如需登录私有仓库，填写仓库用户名与密码。凭据仅保存在当前运行内存中，不写入磁盘。
5. 选择镜像后点击“拉取到目标机”或“拉取到本机”。
6. 可选填写导出目录，拉取完成后自动生成 `docker save` tar 包。

清单示例见 [examples/manifest.example.yaml](examples/manifest.example.yaml)，字段说明见 [docs/manifest-schema.md](docs/manifest-schema.md)。

### 阿里云 ACR 示例

```yaml
apiVersion: deploy.desktop/v1
kind: ImageBundle
metadata:
  name: demo-stack
  version: 1.2.0
spec:
  registry: registry.cn-hangzhou.aliyuncs.com
  namespace: your-namespace
  platform: linux/amd64
  images:
    - name: backend
      repository: backend
      digest: sha256:0000000000000000000000000000000000000000000000000000000000000000
      sizeBytes: 268435456
```

推送镜像后，用以下命令获取真实 digest：

```bash
docker buildx imagetools inspect registry.cn-hangzhou.aliyuncs.com/your-namespace/backend:1.2.0
```

将输出的 `Digest` 填入清单。工具拉取后会校验 `RepoDigests`，摘要不一致会直接失败。

## 镜像与离线包

「镜像与离线包」页面直接读取本机 Docker，不再依赖交付包目录结构：

- 「镜像列表」展示本机镜像地址、ID、大小与创建时间，可勾选后批量导出 tar 离线包或删除；「拉取镜像」支持公开仓库与私有仓库，私有仓库凭据只用于本次拉取，写入临时 Docker 配置，完成后自动清除。
- 「离线包列表」展示离线包目录中的 tar/tgz 文件，可勾选导入本机 Docker，或批量删除；删除只移除文件，不影响已经导入的镜像。
- 离线包目录在「设置」中维护，默认是 `~/Documents/离线部署助手/离线镜像包`（Windows 为 `%USERPROFILE%\Documents\离线部署助手\离线镜像包`）。
- 导出时会在目录中维护 `images.json`，记录每个离线包对应的镜像，远程分发页据此识别镜像。

## 项目配置

仓库内的 `profile.example.json` 只保留通用示例，不包含任何真实产品拓扑。桌面端启动时优先读取应用配置目录下的 `profile.json`，不存在时使用内置示例：

| 平台 | 配置文件位置 |
| --- | --- |
| macOS | `~/Library/Application Support/com.deploydesktop.app/profile.json` |
| Windows | `%APPDATA%\com.deploydesktop.app\profile.json` |
| Linux | `~/.config/com.deploydesktop.app/profile.json` |

字段说明：

- `containerPrefix`：容器名前缀，实际容器名为 `<prefix>-<serviceKey>`
- `containers`：服务键、显示名、端口、协议与说明
- `database`：数据库容器、客户端命令、端口与库名
- `images`：离线镜像包文件名与镜像引用
- `installDir` / `deployDir` / `bundleDir`：默认部署路径
- `backendArtifact`：后端 Fat Jar 在部署目录中的文件名
- `components`：更新时各组件对应的 Compose 服务键，未声明时回退到旧默认值

真实环境把 `profile.example.json` 复制到上表位置并改为自己的拓扑即可，不需要重新编译，也不会污染开源仓库。

## 制品中心

「制品中心」从云效流水线或直接下载地址获取交付包，校验 MD5 与包结构后解包到本地，并用「设为当前部署包」交给部署向导与服务更新使用。

- 云效个人访问令牌保存在应用配置目录的 `artifact-tokens.json`（权限 0600），不写日志；
  首次启动会尝试从旧钥匙串迁移一次，之后直接在「设置」中维护。
- 交付包布局、校验规则、目录位置与流水线侧打包要求见 [docs/artifact-source.md](docs/artifact-source.md)。

### 文件模式部署

文件模式交付包（仅含 `app/` 与 `deploy/`）在启动容器栈时会自动：

- 同时识别部署目录根部和 `deploy/` 子目录下的 compose 文件，并叠加 `.env` / `.env.local`；
- 若 compose 未挂载 `frontend_dist`，生成 `docker-compose.frontend-bind.yml`，用 nginx 只读挂载前端产物；
- `up -d` 完成后重启后端与网关容器，使 bind mount 的新 jar / classes 立即生效。

上述前端覆盖依赖交付包自带 `deploy/nginx/frontend-*.conf`；缺失时不会静默回退到预构建前端镜像。

## 应用更新

顶部“检查更新”按钮按需读取本仓库 GitHub Releases 的最新 `latest.json`，只在点击时检查，不会在启动时联网，也不需要在应用内配置更新源：

`https://github.com/ZhiPenTu/deploy-desktop/releases/latest/download/latest.json`

该文件由 Tauri updater 读取，格式如下：

```json
{
  "version": "0.1.2",
  "notes": "更新说明",
  "pub_date": "2026-10-19T00:00:00Z",
  "platforms": {
    "darwin-aarch64": {
      "signature": "<.app.tar.gz.sig 内容>",
      "url": "https://example.com/deploy-desktop.app.tar.gz"
    }
  }
}
```

更新包必须通过公钥签名校验，未签名的包不会被安装。

发布前生成签名密钥，并把公钥写入 `src-tauri/tauri.conf.json` 的 `plugins.updater.pubkey`：

```bash
npx tauri signer generate -w ~/.tauri/deploy-desktop.key
```

构建更新产物时传入私钥：

```bash
TAURI_SIGNING_PRIVATE_KEY="$(cat ~/.tauri/deploy-desktop.key)" \
TAURI_SIGNING_PRIVATE_KEY_PASSWORD="" \
npx tauri build --bundles app,dmg \
  --config '{"bundle":{"createUpdaterArtifacts":true}}'
```

macOS 的更新产物是 `deploy-desktop.app.tar.gz` 与 `.sig`；Windows 对应 NSIS/MSI，Linux 对应 AppImage/DEB。推送 `v*` 标签并发布 GitHub Release 后，GitHub Actions 会自动生成并上传 `latest.json` 与各平台更新产物。需要在仓库 Secrets 中设置 `TAURI_SIGNING_PRIVATE_KEY` 与 `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`。私钥只保存在本地或 CI Secrets 中，不得提交到仓库。

## 安全说明

- 仓库密码、云效个人访问令牌只存在于内存或系统钥匙串，不写入配置文件、不写日志。
- 交付包下载仅允许 HTTPS；云效来源限制阿里云域名，解包前拒绝绝对路径、`..` 与符号链接。
- 清单必须使用 `sha256` digest 固定镜像版本，不接受仅 tag 的引用。
- 清单来源支持 HTTP(S) 与本地文件，最大 5 MB；生产环境建议使用 HTTPS。
- 部署与更新通过 SSH 公钥执行，应用不会保存 SSH 私钥。
- 本工具不采集遥测，不上传任何部署数据。

发现安全问题请按 [SECURITY.md](SECURITY.md) 私下报告。

## 目录结构

```text
src/                 Vue 前端
src-tauri/           Tauri / Rust 桌面端
  src/commands/      环境检测、Docker、SSH、清单、制品源、更新等命令
mcp-server/          stdio MCP 服务（Node.js，工具实现与测试）
profile.example.json 通用示例部署配置（真实拓扑放在用户配置目录）
examples/            清单示例
docs/                文档
scripts/             UI 文本检查等脚本
```

## 贡献

见 [CONTRIBUTING.md](CONTRIBUTING.md)。提交前请确保 `npm run check:ui-text`、`npm run build`、`cargo test --manifest-path src-tauri/Cargo.toml` 通过。

## 许可证

Apache License 2.0，见 [LICENSE](LICENSE)。
