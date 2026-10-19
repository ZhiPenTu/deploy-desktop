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
- 检查更新：按需检查更新源，下载安装后自动重启应用

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

## 项目配置

根目录的 `profile.json` 是前后端共用的部署配置：

- `containerPrefix`：容器名前缀，实际容器名为 `<prefix>-<serviceKey>`
- `containers`：服务键、显示名、端口、协议与说明
- `database`：数据库容器、客户端命令、端口与库名
- `images`：离线镜像包文件名与镜像引用
- `installDir` / `deployDir` / `bundleDir`：默认部署路径
- `backendArtifact`：后端 Fat Jar 在部署目录中的文件名

修改 `profile.json` 后重新构建即可适配其他 Compose 项目。默认 profile 是一个可直接运行的参考配置。

## 应用更新

顶部“检查更新”按钮按需读取更新源地址，只在点击时检查，不会在启动时联网。地址与是否允许 HTTP 都保存在本机偏好设置中，不会上传。更新源应返回 Tauri updater 的 `latest.json`：

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

默认只接受 HTTPS 更新源。若更新镜像部署在内网、只能使用 HTTP，需在对话框中勾选“允许 HTTP 更新源（内网镜像）”后再检查。无论使用哪种协议，更新包都必须通过公钥签名校验，未签名的包不会被安装。

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

macOS 的更新产物是 `deploy-desktop.app.tar.gz` 与 `.sig`；Windows 对应 NSIS/MSI，Linux 对应 AppImage/DEB。GitHub Actions 发布流程已配置为自动生成并上传 `latest.json`，需要在仓库 Secrets 中设置 `TAURI_SIGNING_PRIVATE_KEY` 与 `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`。私钥只保存在本地或 CI Secrets 中，不得提交到仓库。

## 安全说明

- 仓库密码只存在于内存，不持久化、不写日志。
- 清单必须使用 `sha256` digest 固定镜像版本，不接受仅 tag 的引用。
- 清单来源支持 HTTP(S) 与本地文件，最大 5 MB；生产环境建议使用 HTTPS。
- 部署与更新通过 SSH 公钥执行，应用不会保存 SSH 私钥。
- 本工具不采集遥测，不上传任何部署数据。

发现安全问题请按 [SECURITY.md](SECURITY.md) 私下报告。

## 目录结构

```text
src/                 Vue 前端
src-tauri/           Tauri / Rust 桌面端
  src/commands/      环境检测、Docker、SSH、清单、更新等命令
profile.json         前后端共用的部署配置
examples/            清单示例
docs/                文档
scripts/             UI 文本检查等脚本
```

## 贡献

见 [CONTRIBUTING.md](CONTRIBUTING.md)。提交前请确保 `npm run check:ui-text`、`npm run build`、`cargo test --manifest-path src-tauri/Cargo.toml` 通过。

## 许可证

Apache License 2.0，见 [LICENSE](LICENSE)。
