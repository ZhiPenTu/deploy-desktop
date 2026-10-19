# 制品源与交付包

「制品中心」用于从流水线获取交付包，校验后作为部署向导与服务更新的输入。交付包必须由目标项目按约定布局产出，桌面端只消费、不生成。

## 交付包布局

交付包是单个 `.tgz` 压缩包，解包后必须是以下结构：

```text
bundle.json
app/
  <backendArtifact>       后端 Fat Jar，文件名由 profile 的 backendArtifact 决定
  frontend-dist/
    index.html
  gateway-classes/        网关 classpath 目录
  gateway-lib/            网关依赖 jar
deploy/
  docker-compose.yml
  docker-compose.lan.yml  局域网部署使用，存在时优先
```

`bundle.json` 至少包含：

```json
{
  "version": "5",
  "commit": "40 位提交号",
  "commitShort": "短提交号",
  "branch": "master",
  "builtAt": "2026-10-19T00:00:00Z",
  "files": [
    { "path": "app/tt-net-manager-server.jar", "size": 123456, "sha256": "..." }
  ]
}
```

下载完成后桌面端会依次校验：MD5、压缩包路径安全（绝对路径、`..`、符号链接）、必要文件是否存在。任一环节失败会中止，不会覆盖已有交付包。

## 制品源类型

### 云效流水线

填写组织 ID、流水线 ID、制品名与个人访问令牌。桌面端会：

1. 请求运行列表，取最近一条 `SUCCESS` 记录。
2. 请求运行详情，按制品名匹配上传结果。
3. 读取 `downloadUrl`、`md5`、`version`、`fileSize`。
4. 下载并按上述规则校验、解包。

个人访问令牌保存在系统钥匙串（macOS 钥匙串、Windows 凭据管理器），配置文件 `artifact-sources.json` 只记录组织、流水线、制品名与本地版本，不保存令牌。Linux 无钥匙串时退化为权限 `0600` 的本地文件。

### 直接下载地址

适用于自建制品服务器、对象存储或内网文件服务。地址在保存时写入本地配置，下载时只接受与该配置完全一致的 URL，不接受前端临时传入的其他地址。

仅允许 `https://` 下载。云效流水线来源额外限制下载域名必须是 `aliyuncs.com` / `aliyun.com` 及其子域，避免恶意地址被用来读取任意主机内容。

下载体积上限为 5 GiB。

## 目录位置

| 平台 | 应用配置目录 |
| --- | --- |
| macOS | `~/Library/Application Support/com.deploydesktop.app` |
| Windows | `%APPDATA%\com.deploydesktop.app` |
| Linux | `~/.config/com.deploydesktop.app` |

- `profile.json`：部署拓扑，不存在时回退到内置示例。
- `artifact-sources.json`：制品源列表。
- `bundles/<sourceId>/<version>/`：解包后的交付包。

点击「设为当前部署包」会把该目录写入当前会话的 `bundleDir`，部署向导与服务更新随即使用这个交付包。

## 交付包与镜像包

交付包只包含程序产物，不包含 Docker 镜像。首次部署还需要一个离线介质目录，自行放置：

```text
<mediaDir>/
  images/               docker save 生成的镜像 tar
  docker-engine/        Docker 离线安装包
```

部署向导第 3 步的「分发交付包」会把 `app/` 与 `deploy/` 写入部署目录，形成 Compose 期望的挂载结构（`backend-java/server/target/`、`backend-java/gateway-app/target/`、`deploy/frontend_dist/`），之后再启动容器。远程模式通过 SSH 上传，本地模式直接复制；`deploy/` 采用合并复制，已有的 `.env.local` 不会被删除。

部署向导的「交付包目录」与「离线介质目录」是两个独立输入：前者来自制品中心，后者由现场准备。两者可以指向同一个目录，但不要把只含 `app/` 与 `deploy/` 的交付包当作镜像目录。

## 在流水线侧产出

参考 `flow.yml` 的 `assemble_bundle` 步骤：构建后端 Fat Jar、前端静态资源与网关 classpath，按上面的目录结构汇总后打包上传为单制品。制品名需与桌面端填写的一致（默认 `deploy-bundle`）。
