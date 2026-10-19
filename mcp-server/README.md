# deploy-desktop MCP 服务

`deploy-desktop-mcp` 是一个基于 stdio 的 [Model Context Protocol](https://modelcontextprotocol.io/) 服务，把 `deploy-desktop` 的仓库、镜像、离线包、部署与更新能力暴露给支持 MCP 的 AI 客户端。它复用系统 `docker`、`ssh` 与本机部署配置（默认读取桌面端用户配置目录下的 `profile.json`），不依赖桌面 UI。

## 环境要求

- Node.js 20+（推荐 22/24）
- 本地操作需要 `docker`；远程操作需要 `ssh` 与目标机 SSH 公钥登录
- 远程部署/更新需要目标机已安装 Docker Compose

安装依赖：

```bash
npm --prefix mcp-server install
```

## 启动

服务使用 stdin/stdout 通信，通常由 MCP 客户端拉起：

```bash
node mcp-server/index.mjs
```

检查版本：

```bash
node mcp-server/index.mjs --version
```

### Codex

在 `~/.codex/config.toml` 中增加：

```toml
[mcp_servers.deploy-desktop]
command = "node"
args = ["/absolute/path/to/deploy-desktop/mcp-server/index.mjs"]
env = { DEPLOY_DESKTOP_ROOT = "/absolute/path/to/deploy-desktop" }
```

### Claude Desktop / Claude Code

在 `claude_desktop_config.json` 或项目 `.mcp.json` 中增加：

```json
{
  "mcpServers": {
    "deploy-desktop": {
      "command": "node",
      "args": ["/absolute/path/to/deploy-desktop/mcp-server/index.mjs"],
      "env": {
        "DEPLOY_DESKTOP_ROOT": "/absolute/path/to/deploy-desktop"
      }
    }
  }
}
```

### Cursor

在项目 `.cursor/mcp.json` 中使用与 Claude 相同的 `mcpServers` 结构。

## 环境变量

| 变量 | 说明 | 默认值 |
| --- | --- | --- |
| `DEPLOY_DESKTOP_ROOT` | 仓库根目录，用于读取 `package.json`、`profile.example.json` 与示例清单 | `mcp-server/..` |
| `DEPLOY_DESKTOP_PROFILE` | 指定 `profile.json` 路径，优先级最高 | 空 |
| `DEPLOY_DESKTOP_CONFIG_DIR` | MCP 配置目录 | `~/.deploy-desktop` |
| `DEPLOY_DESKTOP_CONFIG_FILE` | MCP 配置文件路径 | `<CONFIG_DIR>/mcp.json` |
| `DEPLOY_DESKTOP_SSH_KEY` | 默认 SSH 私钥路径 | 自动查找 `~/.ssh/id_ed25519` 等 |
| `DEPLOY_REGISTRY_PASSWORD` | 仓库密码兜底来源 | 空 |
| `DEPLOY_MCP_DEBUG` | 设为 `1` 时向 stderr 输出启动信息 | 空 |

`profile.json` 的解析顺序：`DEPLOY_DESKTOP_PROFILE` → `mcp.json` 的 `profilePath` → 桌面端用户配置目录（macOS `~/Library/Application Support/com.deploydesktop.app/profile.json`，Windows `%APPDATA%\com.deploydesktop.app\profile.json`，Linux `~/.config/com.deploydesktop.app/profile.json`）→ `<DEPLOY_DESKTOP_ROOT>/profile.json` → `profile.example.json`。

## 本地配置

配置默认写入 `~/.deploy-desktop/mcp.json`。目录权限为 `0700`，文件权限为 `0600`。

```json
{
  "version": 1,
  "profilePath": null,
  "registries": {
    "acr": {
      "name": "acr",
      "url": "registry.cn-hangzhou.aliyuncs.com",
      "namespace": "your-namespace",
      "username": "your-account",
      "passwordEnv": "ACR_PASSWORD",
      "insecure": false,
      "isDefault": true
    }
  },
  "hosts": {
    "lan-01": {
      "name": "lan-01",
      "host": "192.168.1.20",
      "port": 22,
      "user": "root",
      "keyPath": "~/.ssh/id_ed25519",
      "installDir": "/opt/deploy",
      "deployDir": "/opt/deploy/deploy",
      "bundleDir": "/opt/deploy/bundle",
      "strictHostKeyChecking": "accept-new"
    }
  },
  "update": {
    "endpoint": "",
    "allowInsecure": false
  }
}
```

仓库密码不会写入该文件。保存 `passwordEnv` 后，在启动 MCP 服务的环境中设置对应变量；也可以在单次工具调用中传入 `password`，密码只存在于本次进程内存。

`update.endpoint` 留空时直接使用 GitHub Releases 的默认地址，无需额外配置。

## 工具覆盖

共 39 个工具，按能力分组：

| 分类 | 工具 |
| --- | --- |
| 部署配置 | `deploy_profile_get`、`deploy_profile_update` |
| SSH 主机 | `deploy_host_list`、`deploy_host_save`、`deploy_host_remove`、`deploy_ssh_test`、`deploy_ssh_setup_key`、`deploy_env_check` |
| 镜像仓库 | `deploy_registry_list`、`deploy_registry_save`、`deploy_registry_remove`、`deploy_registry_probe`、`deploy_registry_tags`、`deploy_registry_digest`、`deploy_registry_login`、`deploy_registry_logout` |
| 镜像清单 | `deploy_manifest_load`、`deploy_manifest_validate`、`deploy_manifest_write` |
| 镜像操作 | `deploy_image_pull`、`deploy_image_save`、`deploy_image_load`、`deploy_image_list`、`deploy_image_remove` |
| 离线包 | `deploy_bundle_scan`、`deploy_bundle_upload`、`deploy_bundle_import`、`deploy_bundle_deploy`、`deploy_bundle_remove` |
| 部署运维 | `deploy_compose`、`deploy_inspect`、`deploy_remote_run`、`deploy_logs`、`deploy_db_check`、`deploy_db_import`、`deploy_docker_install` |
| 更新 | `deploy_update_remote`、`deploy_update_settings`、`deploy_app_update_check` |

大多数工具接受 `target: "local" | "remote"`；远程模式使用 `host`（已保存的主机配置名或 IP/域名）以及可选的 `port`、`user`、`keyPath`。

## 常用流程

### 1. 配置仓库与主机

```text
deploy_registry_save  { name: "acr", url: "registry.cn-hangzhou.aliyuncs.com", namespace: "your-namespace", username: "robot", passwordEnv: "ACR_PASSWORD", isDefault: true }
deploy_host_save      { name: "lan-01", host: "192.168.1.20", user: "root", installDir: "/opt/deploy" }
deploy_ssh_test       { host: "lan-01" }
deploy_env_check      { target: "remote", host: "lan-01" }
```

### 2. 拉取镜像并导出离线包

```text
deploy_manifest_validate { source: "./examples/manifest.example.yaml" }
deploy_image_pull        { target: "local", source: "./examples/manifest.example.yaml", exportDir: "/data/bundle/images" }
deploy_bundle_scan       { dir: "/data/bundle" }
```

### 3. 上传并部署到目标机

```text
deploy_bundle_upload { host: "lan-01", localDir: "/data/bundle", remoteBundleDir: "/opt/deploy/bundle" }
deploy_bundle_deploy { target: "remote", host: "lan-01", bundleDir: "/data/bundle", confirm: true }
deploy_inspect       { target: "remote", host: "lan-01" }
```

`deploy_bundle_deploy` 会依次完成镜像上传（可选）、`docker load`、`docker compose up -d`、可选 SQL 导入，并返回部署状态。首次部署或整机重建推荐使用它。

### 4. 增量更新

```text
deploy_update_remote { host: "lan-01", bundleDir: "/data/update", components: ["backend", "frontend"], dryRun: true }
deploy_update_remote { host: "lan-01", bundleDir: "/data/update", components: ["backend", "frontend"], confirm: true }
```

更新包目录需要包含 `app/` 与 `deploy/` 产物。组件可选 `backend`、`gateway`、`frontend`、`config`。

### 5. 应用更新检查

```text
deploy_app_update_check  {}
```

默认读取 `https://github.com/ZhiPenTu/deploy-desktop/releases/latest/download/latest.json`，无需配置。该工具只比较版本并返回当前平台下载信息，不会下载或替换正在运行的桌面应用。只有需要改用内网镜像等特殊场景时，才使用 `deploy_update_settings` 覆盖默认地址。

## 安全边界

- 仓库密码不落盘；配置文件权限为 `0600`，目录权限为 `0700`。
- 镜像清单必须使用 `sha256` digest，拒绝仅 tag 的引用。
- 远程命令使用 `ssh -o BatchMode=yes`，依赖密钥登录；不会保存 SSH 私钥。
- 下列操作要求 `confirm=true`：修改 `profile.json`、删除主机/仓库/镜像/离线包、一键部署、`compose down`、远程构建部署、数据库导入、离线安装 Docker、实际执行远程更新、保存自定义更新源。
- `deploy_remote_run` 调用仓库根目录的 `deploy-lan.sh`；本仓库默认不包含该脚本，缺失时会返回明确错误。没有脚本时请使用 `deploy_bundle_deploy` 或 `deploy_update_remote`。
- `deploy_app_update_check` 不执行安装；应用更新仍由桌面端完成签名校验与替换。

## 测试

```bash
npm --prefix mcp-server test
npm --prefix mcp-server run smoke
```

单元测试覆盖清单校验、profile 校验、配置权限、仓库认证头解析、本地 Registry 探测、镜像包扫描、更新检查与工具注册/确认边界。`smoke` 会用真实 MCP Client 通过 stdio 拉起服务，验证工具列表、profile、仓库保存、清单解析、离线包扫描与确认边界。两者都不会访问真实 ACR 或 SSH 目标机。
