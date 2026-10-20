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

`bundle.json` 是交付包的唯一事实源，桌面端据此判断模式、版本与可更新组件：

```json
{
  "schemaVersion": 1,
  "product": "tt-net-manager",
  "mode": "file",
  "version": "5",
  "commit": "40 位提交号",
  "commitShort": "短提交号",
  "branch": "master",
  "builtAt": "2026-10-19T00:00:00Z",
  "components": ["backend", "gateway", "frontend", "config"],
  "files": [
    { "path": "app/tt-net-manager-server.jar", "size": 123456, "sha256": "..." }
  ]
}
```

`mode` 有两种：

| 模式 | 目录 | 含义 |
| --- | --- | --- |
| `file` | `app/` + `deploy/` | 产物通过 bind mount 挂进容器，交付包不含应用镜像 |
| `image` | `images/` + `deploy/` | 交付包自带 `docker save` 镜像包，需先导入再启动 |

`components` 决定「服务更新」默认勾选哪些组件（`backend` / `gateway` / `frontend` / `config`，镜像模式额外有 `image`）。

## 预制流程说明

文件模式交付包可以再带两个可选文件，用来把「执行流程」写成配置而不是靠人记：

- `deploy/deployment-plan.json`：`steps[]` 提供每一步的 `title` 与 `hint`，桌面端按 `key` 合入推导出的链路。步骤顺序、实时数量与状态始终由程序按实际文件计算，清单只补说明，不会覆盖真实状态。
- `deploy/base-images.txt`：目标机所需基础镜像的唯一清单。桌面端据此列出依赖镜像，并比对目标机是否已具备；流水线里应校验它与 compose 的 `image:` 行一致。

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

制品中心在选定交付包后会展示「交付包链路」：获取交付包 → 准备基础镜像 → 分发交付包 → 启动容器栈 → 验证与验收，并为每一步标注就绪 / 按需 / 待补齐。链路下面是 `app/` 与 `deploy/` 的实际文件清单和依赖镜像表，镜像表标注来源（随包镜像 / 基础镜像清单 / compose 推导）以及目标机是否已具备。

镜像有两种来源，含义不同：

- 镜像模式交付包里的 `images/`：应用自身的镜像，必须导入，缺一个就无法启动。
- 文件模式依赖的基础镜像：OceanBase、Kafka、Redis、Nginx 等底座，不在交付包里，只在首次部署准备一次。

首次部署还需要一个离线介质目录，自行放置：

```text
<mediaDir>/
  images/               docker save 生成的镜像 tar
  docker-engine/        Docker 离线安装包
```

部署向导执行首次部署时会把 `app/` 与 `deploy/` 写入部署目录，形成 Compose 期望的挂载结构（`backend-java/server/target/`、`backend-java/gateway-app/target/`、`deploy/frontend_dist/`），之后再启动容器。远程模式通过 SSH 上传，本地模式直接复制；`deploy/` 采用合并复制，已有的 `.env.local` 不会被删除。

部署向导的「交付包目录」与「离线介质目录」是两个独立输入：前者来自制品中心，后者由现场准备。两者可以指向同一个目录，但不要把只含 `app/` 与 `deploy/` 的交付包当作镜像目录。

## 部署向导

部署向导按以下顺序执行，不再单独选择本地或远程模式：

1. **选择主机**：从主机管理中选择本机或 SSH 主机，自动使用其连接信息。确认目标部署根目录。
2. **检测环境**：检查所选主机的系统资源、Docker、Compose 与端口。Docker 或 Compose 未就绪时不能继续。
3. **选择制品**：选择制品源的最新版本、已下载包、本地 `.tgz` / `.tar.gz`，或已解压目录。此时只选择候选，不改变当前部署包。
4. **解压制品**：下载或解压所选制品，检查路径安全和必要文件。压缩包解压到应用配置目录下独立的 `bundles/local/` 子目录，不覆盖原文件；已解压目录直接检查。检查通过后更新当前部署包。
5. **清单比对**：读取制品中的 Compose，与目标部署清单对比服务新增、变化、移除情况，并按完整镜像引用检查目标 Docker。离线镜像包存在不等于镜像已导入；缺失或未确认的镜像会阻止进入执行步骤。可在本步骤选择本机离线介质目录，上传（远程主机）并导入后重新比对。
6. **选择更新**：空部署目录可以首次部署；已有 Compose 的目录只开放增量更新。增量更新支持选择组件，默认备份。执行前重新预检，再确认主机、目录、版本与影响范围。

首次部署会分发交付包、启动 Compose，并检查容器运行状态。文件模式下数据库迁移由应用启动时处理，向导不自动导入全量 SQL，避免覆盖已有业务数据。

增量更新通过 SSH 或本机专用更新命令执行；本机更新要求 Docker 连接指向本机，并确认当前部署与所选制品的 Compose 项目、服务和产物挂载一致。预检不替换文件、不创建备份、不重启服务；执行时才备份并替换所选内容。本机更新不删除数据卷，不隐式拉取镜像或构建镜像。

服务命令完成后仍会核对实际容器状态。未运行或健康检查尚未通过时显示待确认，不显示部署成功。浏览器预览不能执行向导，也不能作为真实部署结果。

### 比对边界

- 制品清单优先读取 `deploy/docker-compose.lan.yml`，不存在时读取 `deploy/docker-compose.yml`；目标清单沿用执行器的根目录 / `deploy/` 搜索顺序。
- 服务差异按 Compose 声明比较，不把服务移除自动解释成删除容器、数据或卷。
- 镜像按完整引用核实，包含 `@sha256:` 时不回退为 tag。尚含 `${...}` 的镜像地址视为未确定，需补齐制品后重试。
- 本地目录的检查覆盖路径安全和结构；本地压缩包无上游摘要时不声称完成来源完整性验证。云端下载仍按制品源提供的 MD5 检查。
- 操作进行时暂不能切换页面，执行参数同时使用独立快照，避免任务途中切换目标。

## 在流水线侧产出

参考 `flow.yml` 的 `assemble_bundle` 步骤：构建后端 Fat Jar、前端静态资源与网关 classpath，按上面的目录结构汇总后打包上传为单制品。制品名需与桌面端填写的一致（默认 `deploy-bundle`）。
