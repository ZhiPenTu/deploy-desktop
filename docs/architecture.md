# 功能目录与调用边界

## 前端

```text
src/
  App.vue                       页面切换与任务进行中的导航保护
  components/layout/            顶栏与侧栏
  config/                       部署拓扑与容器展示配置
  features/
    connections/                主机、凭证及当前连接状态
    artifacts/                  制品来源、版本选择、交付包和 Compose 预览
    deployment/
      views/                    部署向导界面
      composables/              向导状态、确认与界面反馈
      domain/                   路径、组件、目标快照、就绪判定和诊断脱敏
      services/                 本机 / SSH 更新的调用与结果检查
    images/                     Docker 镜像、离线包与分发
    registry/                   镜像源与清单拉取
    monitoring/                 环境检测、容器状态与日志
    mcp/                        MCP 能力目录、分类与搜索
    settings/                   本机设置与应用更新
  api/
    tauri.js                    功能模块使用的公共入口
    commands/                   按功能组织命令和参数映射
    desktop.js                  Tauri 调用、进度通道、事件与文件选择
  shared/
    async/                      请求有效期与异步监听释放
    composables/                Vue 生命周期适配
```

页面和功能模块只从 `api/tauri.js` 引入桌面能力。`commands/` 不依赖 Vue 或页面状态；`desktop.js` 是唯一直接引入 `@tauri-apps/*` 的位置。浏览器下保持空配置列表，其余桌面操作明确拒绝执行。

`domain/` 中的规则不访问磁盘、网络或桌面运行时。`services/` 负责副作用调用并检查执行结果；组件负责确认弹窗、加载状态和结果展示。新增功能优先放入所属功能目录；只有跨功能复用且不依赖具体业务的逻辑进入 `shared/`。

部署和更新在确认前复制目标、路径、版本、组件和备份选项，使用冻结快照执行；对话框和任务进行期间禁止重复触发及切换页面。`success: false` 或无有效结果均按失败处理，保留备份路径和脱敏后的诊断。部署向导还会读取容器实际状态，命令完成不等于服务已就绪。

异步读取使用请求版本号，目标变化、新请求或页面销毁都会使旧结果失效。进度监听即使在页面销毁后才完成注册，也会立即释放；进度不可用不能阻断独立的状态读取。

页面切换不等待只读 SSH 检查；导航保护仅覆盖部署、安装、导入和部署预检等受保护操作。桌面调用层合并参数相同的未完成读取，不缓存成功结果或失败。日志自动刷新在上次读取完成后才允许下一次读取，失败时停止自动刷新，离开页面后停止计时并忽略旧结果。

macOS 使用覆盖式原生标题栏并隐藏重复的窗口标题，保留系统窗口按钮。应用仅在 macOS 桌面端为侧栏顶部预留窗口按钮区域；Windows、Linux 与浏览器沿用原有布局。顶栏和侧栏品牌区支持拖动及双击缩放，顶栏操作区显式排除拖动，拖动权限由主窗口 capability 提供。

## Rust 部署模块

`src-tauri/src/commands/remote_update/mod.rs` 保留原有命令与类型导出，内部按职责拆分：

| 文件 | 职责 |
| --- | --- |
| `models.rs` | 部署状态、产物与结果类型 |
| `bundle.rs` | 包布局、所选组件与必要产物校验 |
| `plan.rs` | 部署链路、文件清单与镜像准备状态 |
| `config.rs` | 配置文件预览、Compose 解析和离线镜像匹配 |
| `inspect.rs` | 目标目录、容器、日志与磁盘检查 |
| `transfer.rs` | SSH 文件传输、替换与备份 |
| `install.rs` | 首次分发、本机目录复制 |
| `update.rs` | 远程增量更新编排 |
| `progress.rs` | 部署进度事件 |
| `tests.rs` | 包结构、路径、Compose 与本地分发回归 |

内部模块显式引用所需的同级函数。解析辅助函数仅在命令模块范围共享，传输与安装实现不作为新的 Tauri 命令开放。既有 Tauri 命令名、参数、序列化字段及进度事件保持兼容；本机更新继续由 `local_update.rs` 实现。

执行 SSH、Docker 等外部进程的检查入口使用异步命令，通过 `commands/blocking.rs` 将同步实现移到阻塞线程池，禁止在桌面主线程中等待外部进程。内部同步调用继续使用对应的 `_blocking` 函数；已有异步部署执行器保留原有调度方式。

## MCP

`mcp-server/src/tools.mjs` 只负责组装工具注册。`tools/` 按配置、主机、镜像仓库、清单、镜像、离线包、运维和更新划分注册函数；每组保留原有工具名称、输入模式、默认值及确认要求。

`tools/shared.mjs` 提供复用的输入模式和配置解析，`tools/runtime.mjs` 统一结果与错误包装以及尽力发送的进度通知。具体 Docker、SSH、文件和配置操作继续由原有服务模块实现，注册层不维护另一份部署配置。

桌面侧栏的「高级 / MCP 服务」提供能力概览，支持按分类、操作类型和用途搜索。`src/features/mcp/catalog.js` 维护面向用户的能力说明；工具名称仅用于关联注册项，不直接展示注册层的开发说明。`mcp-server/test/catalog.test.mjs` 核对目录与实际注册工具完整对应，并检查只读和确认标记，新增或调整工具时需同步维护目录。

该页面不启动服务或执行工具，也不推断 MCP 连接状态；服务仍由客户端通过 stdio 拉起，状态由客户端查看。MCP 的主机、镜像仓库与更新源配置独立保存，不使用桌面当前主机选择。远程构建部署依赖额外脚本，页面说明其使用前提与离线部署、远程更新替代方式。

## 验证

```bash
npm run check:boundaries
npm run check:ui-text
npm run test:api
npm run build
cargo test --manifest-path src-tauri/Cargo.toml
cargo check --manifest-path src-tauri/Cargo.toml
npm run test:mcp
npm --prefix mcp-server run smoke
```

`check:boundaries` 检查前端的越界导入、缺失引用、循环依赖及 Vue 模板中未定义的绑定，并作为构建前置检查。前端测试覆盖浏览器禁用、桌面参数映射、执行目标快照、失败结果、路径校验、请求乱序和异步监听释放；MCP 测试验证工具注册、确认边界、错误包装及进度失败隔离。

界面回归至少覆盖：取消确认不执行、失败不显示成功、执行中目标不可编辑、执行中不能导航、失败后可重试、旧请求不能覆盖新选择。浏览器测试替身不等同于真实桌面、Docker、SSH 或生产部署验证。
