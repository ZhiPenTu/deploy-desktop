# 功能目录与调用边界

## 前端

```text
src/
  App.vue                       页面切换与任务进行中的导航保护
  components/layout/            顶栏与侧栏
  config/                       部署拓扑、容器展示配置与页面元数据
  features/
    connections/                主机、凭证及当前连接状态
    artifacts/                  制品来源、版本选择、交付包和 Compose 预览
      domain/                   版本文案、本地状态与路径摘要等纯展示规则
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
    errors.js                   面向使用者的错误文案
    format.js                   文件大小与时间显示格式
```

`config/pages.js` 是页面的唯一定义处：标题、分组、侧栏图标和是否出现在侧栏都在这里声明，`App.vue` 取页头信息，`Sidebar.vue` 取分组菜单。新增页面必须在 `pages` 与 `pageOrder` 同时登记，`check:boundaries` 与 e2e 的导航用例会拦下漏项；只从其他页面操作进入的页面用 `hidden: true` 标记，不占用侧栏位置。

页面和功能模块只从 `api/tauri.js` 引入桌面能力。`commands/` 不依赖 Vue 或页面状态；`desktop.js` 是唯一直接引入 `@tauri-apps/*` 的位置。浏览器下保持空配置列表，其余桌面操作明确拒绝执行。

页面展示异常时统一使用 `shared/errors.js` 的 `errorText`，只显示异常消息本身，缺失时给出可操作的中文提示，不要直接拼接异常对象。

`shared/format.js` 只收纳输出结果完全一致的格式化逻辑：文件大小、完整时间戳与列表摘要时间。显示文案不同（例如空值显示「-」「未记录」或「未知大小」）的页面保留各自的空值处理，不要为了复用而改变既有显示结果。

`domain/` 中的规则不访问磁盘、网络或桌面运行时。`services/` 负责副作用调用并检查执行结果；组件负责确认弹窗、加载状态和结果展示。新增功能优先放入所属功能目录；只有跨功能复用且不依赖具体业务的逻辑进入 `shared/`。

视图负责页面级编排，弹窗、步骤条、日志流这类独立交互单元放到所属功能的 `components/`，由视图传入数据并通过事件回报操作。步骤内容与视图共用步骤卡片、页脚等布局样式时保留在同一视图内，不为拆分复制样式。

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

交付包描述符的校验用例集中在 `fixtures/bundles/cases.json`，Rust `bundle.rs` 与 MCP `bundle.mjs` 两侧共用同一份用例表，只比较「接受 / 拒绝」，不比较错误文案。用例表通过 `patch` 递归合并到 `base`，便于新增单个规则用例。两侧结论不同的用例必须填写 `divergence` 说明原因：目前 Rust 不限制版本号长度、不校验服务端口为 0、不校验服务协议取值，也不校验数据库名称是否为空，这些差异属于待收敛项，改动任一侧时需同步更新用例表。

执行 SSH、Docker 等外部进程的检查入口使用异步命令，通过 `commands/blocking.rs` 将同步实现移到阻塞线程池，禁止在桌面主线程中等待外部进程。内部同步调用继续使用对应的 `_blocking` 函数；已有异步部署执行器保留原有调度方式。

`npm run check:commands` 会扫描全部 `#[tauri::command]`：命令注册、进度事件发射点和同步命令体一并检查。同步命令体内出现 `Command::new`、`reqwest::blocking`、SSH 脚本执行或 TCP 连接会直接报错，新增命令需先包装为异步入口。

## 本机配置与文件写入

`src-tauri/src/core/` 收拢与具体部署功能无关的本机基础能力：

| 文件 | 职责 |
| --- | --- |
| `config.rs` | 解析应用配置目录，缺失时创建 |
| `atomic_file.rs` | 唯一临时文件 + fsync + rename 覆盖写入，Unix 私密文件权限 0600 |
| `file_lock.rs` | 相邻 `.lock` 文件实现跨进程互斥，残留锁超时回收 |
| `json_store.rs` | JSON 配置读写；内容损坏时先备份原文件再返回可操作错误 |
| `backup.rs` | 暂存替换与回滚、备份目录唯一后缀、文件或目录删除 |
| `compose.rs` | Compose 清单候选文件名、服务字段解析与镜像行扫描 |
| `path_guard.rs` | 相对路径、上级跳转与目录包含关系判断 |
| `version.rs` | 版本号比较；与 MCP 共用 `fixtures/versions.json` |

配置一律通过 `json_store::save` 写入，不再使用 `fs::write` 直接覆盖目标文件。凭据、密码与主机连接配置使用 `FilePermission::OwnerOnly`，其余配置使用默认权限。文件不存在与内容损坏必须区分：前者交由调用方决定默认值，后者先备份原文件再报错，不得静默当作空配置，否则用户下一次保存会覆盖掉仅存的现场。

项目、制品源、令牌与版本缓存由桌面端和 MCP 服务共同改写，读改写必须走 `json_store::update`：它在 `<文件>.lock` 上抢锁，锁内重新读取磁盘最新内容、修改后再写回，避免拿内存里的旧快照覆盖另一进程刚写入的变更。MCP 侧 `mcp-server/src/file-lock.mjs` 遵守同一套锁文件约定，因此任意一端修改后，另一端下一次读改写都会基于最新内容。锁被异常退出遗留超过 30 秒会被视为残留并回收；抢锁超时返回「配置正在被另一个程序写入，请稍后重试」，不静默跳过写入。

更新部署目录、安装交付包这类落地动作统一走 `backup::swap_staged`：先在同级暂存，再让旧内容改名让位，最后把暂存内容落位。落位失败必须回滚并把回滚结果交给命令层，不能只在成功分支清理临时文件，否则目录会停在既没有旧版本也没有新版本的状态。

Compose 清单的候选文件名、服务字段与镜像扫描集中在 `core::compose`，部署预检、部署复核与远程运维从同一处取候选顺序。`parse_services` 用于需要完整服务信息的场景，`scan_image_references` 只按行寻找 `image:`，用于清单结构不完整但仍要给出镜像清单的场景。路径能否拼在目录之后由 `core::path_guard` 判断，命令层负责把拒绝原因翻译成用户能处理的操作。

版本比较只有 `core::version` 一套规则，MCP 侧 `compareVersions` 与它共用 `fixtures/versions.json`。规则按 `.`、`-`、`+` 分段比较数字，非数字段按 0 处理，因此预发布标识与构建元数据也会参与比较；改变这条规则会同时影响升级提示与最低版本校验，属于行为变更。

## MCP

`mcp-server/src/tools.mjs` 只负责组装工具注册。`tools/` 按配置、主机、镜像仓库、清单、镜像、离线包、运维和更新划分注册函数；每组保留原有工具名称、输入模式、默认值及确认要求。

`tools/shared.mjs` 提供复用的输入模式和配置解析，`tools/runtime.mjs` 统一结果与错误包装以及尽力发送的进度通知。具体 Docker、SSH、文件和配置操作继续由原有服务模块实现，注册层不维护另一份部署配置。

桌面侧栏的「高级 / MCP 服务」提供能力概览，支持按分类、操作类型和用途搜索。`src/features/mcp/catalog.js` 维护面向用户的能力说明；工具名称仅用于关联注册项，不直接展示注册层的开发说明。`mcp-server/test/catalog.test.mjs` 核对目录与实际注册工具完整对应，并检查只读和确认标记，新增或调整工具时需同步维护目录。

该页面不启动服务或执行工具，也不推断 MCP 连接状态；服务仍由客户端通过 stdio 拉起，状态由客户端查看。MCP 的主机、镜像仓库与更新源配置独立保存，不使用桌面当前主机选择。远程构建部署依赖额外脚本，页面说明其使用前提与离线部署、远程更新替代方式。

## 验证

```bash
npm run check:boundaries
npm run check:ui-text
npm run check:commands
npm run test:api
npm run test:e2e
npm run build
cargo test --manifest-path src-tauri/Cargo.toml
cargo check --manifest-path src-tauri/Cargo.toml
npm run test:mcp
npm --prefix mcp-server run smoke
```

`check:boundaries` 检查前端的越界导入、缺失引用、循环依赖及 Vue 模板中未定义的绑定，`check:commands` 核对前端调用的 Tauri 命令均已在 `src-tauri/src/lib.rs` 注册、监听的进度事件均有 Rust 发射点，两者都作为构建前置检查。前端测试覆盖浏览器禁用、桌面参数映射、执行目标快照、失败结果、路径校验、请求乱序和异步监听释放；MCP 测试验证工具注册、确认边界、错误包装及进度失败隔离。

`test:e2e` 使用 Playwright 对生产构建的浏览器渲染做端到端检查：导航与面包屑、空状态、表单校验、桌面能力被禁用时的提示，以及页面文本中是否混入开发说明。它运行在 Chromium 中，不覆盖真实桌面壳、Docker、SSH 与目标环境，这些仍需在桌面应用中验证。

界面回归至少覆盖：取消确认不执行、失败不显示成功、执行中目标不可编辑、执行中不能导航、失败后可重试、旧请求不能覆盖新选择。浏览器测试替身不等同于真实桌面、Docker、SSH 或生产部署验证。
