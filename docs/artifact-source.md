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
  "version": "8-a1b2c3d",
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

下载完成后桌面端会依次校验：源提供的 MD5、压缩包路径安全（绝对路径、`..`、符号链接）、必要文件、包内版本与所选版本是否一致。先在临时目录解包，全部校验通过后再替换该版本的本地缓存；校验失败不覆盖已有交付包。

## 按制品源查看

制品中心先列出可管理的制品源，点击名称或「查看制品」进入该来源的制品列表，也可在列表上方切换来源。每行显示制品名、版本号、构建信息、大小与本地状态，支持下载、使用已下载包和查看运行详情。

最新制品按构建时间倒序排列，不按版本号字典序排序。可加载更早的构建记录；当前页没有成功构建时仍可继续加载历史记录。只有名称匹配的上传结果才属于该源，不会回退到其他制品。

## 制品源类型

### 云效流水线

填写组织 ID、流水线 ID、制品名与个人访问令牌。桌面端会：

1. 分页请求运行列表，筛选 `SUCCESS` 记录。
2. 请求运行详情，按制品名匹配上传结果，兼容 `.tgz` / `.tar.gz` 后缀。
3. 读取 `downloadUrl`、`md5`、`version`、`fileSize`；版本为必填字段，兼容字符串和数字类型的历史构建号。
4. 下载并按上述规则校验、解包。

配置文件 `artifact-sources.json` 记录组织、流水线、制品名与版本信息，不保存令牌。当前令牌由本机凭据文件单独管理，Unix 权限为 `0600`，旧钥匙串凭据会迁移到当前存储。

### 直接下载地址

适用于自建制品服务器、对象存储或内网文件服务。需填写制品名、版本号和下载地址，版本号必须与 `bundle.json.version` 完全一致。版本号以字母或数字开头，只接受字母、数字、点、短横线和下划线，最多 128 个字符。地址在保存时写入本地配置，下载时只接受与该配置完全一致的 URL，不接受前端临时传入的其他地址。

旧配置未填写版本号时，可沿用最近下载且已标注的版本。若没有有效版本，则提示编辑制品源补充；不再使用 `manual` 作为可下载版本。修改下载地址时应同时更新版本号，本地旧版本不会被误标为新版本的缓存。

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

部署向导第 3 步的「分发交付包」会把 `app/` 与 `deploy/` 写入部署目录，形成 Compose 期望的挂载结构（`backend-java/server/target/`、`backend-java/gateway-app/target/`、`deploy/frontend_dist/`），之后再启动容器。远程模式通过 SSH 上传，本地模式直接复制；`deploy/` 采用合并复制，已有的 `.env.local` 不会被删除。

部署向导的「交付包目录」与「离线介质目录」是两个独立输入：前者来自制品中心，后者由现场准备。两者可以指向同一个目录，但不要把只含 `app/` 与 `deploy/` 的交付包当作镜像目录。

文件模式下向导第 3 步是「准备基础镜像并分发交付包」：镜像只作为基础底座按需准备，不影响产物分发；第 5 步是「数据初始化」，后端启动时由 Flyway 自动迁移，手工导入仅用于覆盖历史全量数据。

## 在流水线侧产出

参考 `flow.yml` 的 `assemble_bundle` 步骤：构建后端 Fat Jar、前端静态资源与网关 classpath，按上面的目录结构汇总后打包上传为单制品。制品名需与桌面端填写的一致（默认 `deploy-bundle`）。

流水线上传的 `version` 与 `bundle.json.version` 必须一致，推荐 `${BUILD_NUMBER}-${CI_COMMIT_ID}`，例如 `8-a1b2c3d`。缺少构建号、提交号时直接使打包失败，不能回退到 `dev` 或空值。已有仅构建号版本的历史交付包，在两处版本相同的情况下仍可使用。桌面端不根据提交号或运行编号伪造缺失的制品版本。
