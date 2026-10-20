# 部署配置与主机管理

## 部署拓扑

仓库内的 [`profile.example.json`](../profile.example.json) 只保留通用示例，不包含任何真实产品拓扑。桌面端启动时优先读取应用配置目录下的 `profile.json`，不存在时使用内置示例：

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

## 凭证与主机

侧边栏「连接」下提供凭证管理与主机管理：

- 凭证保存凭证名称、登录用户名、私钥路径和可选的初始密码。私钥保留在本机原位置，应用只记录路径，不复制私钥内容。
- 主机分为「本机」与「远程主机（SSH）」两类。本机代表运行部署助手的当前电脑，不需要地址与登录凭证；远程主机保存 SSH 地址、端口与绑定凭证。
- 首次启动会自动加入一台「本机」主机，新配置会将其设为当前主机；删除后不会再次自动添加，可在新增主机时重新选择本机类型。
- 主机保存名称、部署目录和备注，远程主机额外保存 SSH 信息。列表中的「当前主机」会同步到部署向导、主机状态、集群监控与日志诊断。
- 初始密码只用于首次下发免密公钥；配置完成后可保留或清除，界面与列表不会回显密码。
- 删除仍被主机引用的凭证前，需要先为相关主机更换凭证。

连接配置与 `profile.json` 位于同一应用配置目录：

| 文件 | 内容 |
| --- | --- |
| `connections.json` | 凭证与主机信息，不含密码 |
| `connection-secrets.json` | 凭证的初始 SSH 密码，Unix 权限为 0600 |

本地凭据文件未做额外加密，依赖操作系统账户与文件权限保护；不要在共享账户或不受信任的电脑上保存生产密码。

## 其他本地配置

- 镜像源与仓库密码：[镜像源管理](registry-sources.md#存储与安全)。
- 制品源、云效令牌与交付包缓存：[制品源与交付包](artifact-source.md)。
- 离线包目录在「设置」中维护，默认位于用户文档目录的 `部署舱/离线镜像包`。升级用户若仍在使用旧目录 `离线部署助手/离线镜像包`，会继续沿用旧目录，不会自动迁移或重新扫描。导出时生成的 `images.json` 用于记录离线包对应的镜像。
- MCP 使用独立配置文件，详见 [MCP 本地配置](../mcp-server/README.md#本地配置)。
