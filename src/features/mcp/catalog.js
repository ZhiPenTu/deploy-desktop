// User-facing capability copy. Tool IDs are keys, not UI instructions.
// mcp-server/test/catalog.test.mjs checks coverage and safety labels against registration.
export const mcpCategories = [
  {
    id: 'profile', title: '部署配置', description: '查看当前项目与交付包提供的部署配置',
    tools: [
      { id: 'deploy_profile_get', title: '读取当前部署目标', description: '查看当前项目绑定的主机、部署目录与交付包中的服务、数据库与镜像。', access: 'read' }
    ]
  },
  {
    id: 'projects', title: '项目管理', description: '管理项目的目标主机、部署目录与交付包绑定',
    tools: [
      { id: 'deploy_project_list', title: '查看项目', description: '查看已保存的项目、当前项目以及每个项目的绑定信息。', access: 'read' },
      { id: 'deploy_project_create', title: '新建项目', description: '创建项目并指定目标主机、部署目录与可选制品源。', access: 'write' },
      { id: 'deploy_project_update', title: '编辑项目', description: '调整项目的名称、主机、部署目录或制品源。', access: 'write' },
      { id: 'deploy_project_remove', title: '删除项目', description: '移除项目记录；已下载的交付包与目标机文件保留。', access: 'write', confirmation: '需确认' },
      { id: 'deploy_project_activate', title: '切换当前项目', description: '把指定项目设为后续部署与运维使用的当前项目。', access: 'write' },
      { id: 'deploy_project_bind_bundle', title: '绑定交付包', description: '把已下载的交付包绑定到项目，并核对版本与产品。', access: 'write' }
    ]
  },
  {
    id: 'hosts', title: 'SSH 主机', description: '管理连接目标，检查主机与运行环境',
    tools: [
      { id: 'deploy_host_list', title: '查看主机配置', description: '查看 MCP 已保存的 SSH 主机与连接设置。', access: 'read' },
      { id: 'deploy_host_save', title: '保存主机配置', description: '新增或更新主机地址、SSH 密钥与部署目录。', access: 'write' },
      { id: 'deploy_host_remove', title: '删除主机配置', description: '删除 MCP 保存的主机连接配置，目标机不受影响。', access: 'write', confirmation: '需确认' },
      { id: 'deploy_ssh_test', title: '测试 SSH 连接', description: '验证目标机的密钥登录并查看连接耗时。', access: 'read' },
      { id: 'deploy_ssh_setup_key', title: '配置 SSH 密钥', description: '生成登录密钥并下发公钥，必要时提供公钥供手动安装。', access: 'write' },
      { id: 'deploy_env_check', title: '检查主机环境', description: '检查本机或远程主机的资源、Docker、Compose 与端口占用。', access: 'read' }
    ]
  },
  {
    id: 'registries', title: '镜像仓库', description: '管理仓库连接、认证与镜像版本信息',
    tools: [
      { id: 'deploy_registry_list', title: '查看镜像仓库', description: '查看 MCP 保存的镜像仓库与认证配置。', access: 'read' },
      { id: 'deploy_registry_save', title: '保存镜像仓库', description: '新增或更新仓库地址、命名空间与账号设置。', access: 'write' },
      { id: 'deploy_registry_remove', title: '删除镜像仓库配置', description: '移除 MCP 保存的仓库配置。', access: 'write', confirmation: '需确认' },
      { id: 'deploy_registry_probe', title: '检测仓库连接', description: '检查镜像仓库的网络、安全连接与认证是否可用。', access: 'read' },
      { id: 'deploy_registry_tags', title: '查看镜像标签', description: '列出仓库中指定镜像的版本标签。', access: 'read' },
      { id: 'deploy_registry_digest', title: '查询镜像摘要', description: '获取版本标签对应的 sha256 摘要，确定镜像内容。', access: 'read' },
      { id: 'deploy_registry_login', title: '登录镜像仓库', description: '在本机或远程主机上登录指定仓库。', access: 'write' },
      { id: 'deploy_registry_logout', title: '退出镜像仓库', description: '退出本机或远程主机的仓库登录。', access: 'write' }
    ]
  },
  {
    id: 'manifests', title: '镜像清单', description: '检查镜像内容与版本，准备部署清单',
    tools: [
      { id: 'deploy_manifest_load', title: '加载镜像清单', description: '读取本地或在线清单，查看镜像地址、平台与摘要。', access: 'read' },
      { id: 'deploy_manifest_validate', title: '校验镜像清单', description: '检查清单格式、镜像摘要与平台信息。', access: 'read' },
      { id: 'deploy_manifest_write', title: '保存镜像清单', description: '校验后保存清单；覆盖已有文件时需明确允许覆盖。', access: 'write' }
    ]
  },
  {
    id: 'images', title: '镜像操作', description: '拉取、导入、导出与清理 Docker 镜像',
    tools: [
      { id: 'deploy_image_pull', title: '拉取镜像', description: '将所选镜像拉取到本机或远程主机，可同时导出离线包。', access: 'write' },
      { id: 'deploy_image_save', title: '导出镜像', description: '将本机或远程主机的指定镜像导出为 tar 文件。', access: 'write' },
      { id: 'deploy_image_load', title: '导入离线镜像', description: '将离线包中的镜像导入本机或远程 Docker。', access: 'write' },
      { id: 'deploy_image_list', title: '查看 Docker 镜像', description: '查看本机或远程主机上已有的镜像。', access: 'read' },
      { id: 'deploy_image_remove', title: '删除 Docker 镜像', description: '从指定主机删除所选镜像。', access: 'write', confirmation: '需确认' }
    ]
  },
  {
    id: 'bundles', title: '离线包', description: '检查、分发离线镜像包并完成首次部署',
    tools: [
      { id: 'deploy_bundle_scan', title: '扫描离线镜像包', description: '检查本机或远程镜像包的文件、总大小与缺失项。', access: 'read' },
      { id: 'deploy_bundle_upload', title: '上传离线镜像包', description: '向目标机传输所选镜像包，默认跳过大小一致的文件。', access: 'write' },
      { id: 'deploy_bundle_import', title: '导入目标机镜像', description: '将目标机已有的离线镜像包导入 Docker。', access: 'write' },
      { id: 'deploy_bundle_deploy', title: '一键离线部署', description: '上传并导入镜像、启动服务，可选择同时导入数据库。', access: 'write', confirmation: '需确认' },
      { id: 'deploy_bundle_remove', title: '删除目标机镜像包', description: '清理目标机上的指定包文件，已导入的 Docker 镜像保留。', access: 'write', confirmation: '需确认' }
    ]
  },
  {
    id: 'artifacts', title: '制品中心', description: '管理制品源与版本，下载并校验交付包',
    tools: [
      { id: 'deploy_artifact_source_list', title: '查看制品源', description: '查看云效流水线与直接下载源，令牌内容不会显示。', access: 'read' },
      { id: 'deploy_artifact_source_save', title: '保存制品源', description: '新增或更新制品源；令牌只保存到本机受保护配置。', access: 'write' },
      { id: 'deploy_artifact_source_remove', title: '删除制品源', description: '移除制品源、令牌与版本缓存；已下载交付包保留。', access: 'write', confirmation: '需确认' },
      { id: 'deploy_artifact_releases', title: '查看制品版本', description: '读取可用版本与运行信息，可只查看本机已缓存版本。', access: 'read' },
      { id: 'deploy_artifact_download', title: '下载交付包', description: '下载并校验交付包，保存到本机制品缓存。', access: 'write' }
    ]
  },
  {
    id: 'operations', title: '部署运维', description: '管理服务运行，查看日志与数据库状态',
    tools: [
      { id: 'deploy_compose', title: '管理 Compose 服务', description: '启动、停止、重启服务，查看状态或拉取服务镜像。', access: 'write', confirmation: '删除容器需确认' },
      { id: 'deploy_inspect', title: '检查部署状态', description: '查看部署目录、交付文件与容器健康状态。', access: 'read' },
      { id: 'deploy_remote_run', title: '远程构建并部署', description: '构建并同步全部服务，或仅部署选定组件。', access: 'write', confirmation: '需确认', requirement: '需额外配置部署脚本；未配置时暂不可用，可使用一键离线部署或远程更新。' },
      { id: 'deploy_logs', title: '读取服务日志', description: '查看本机或远程容器最近的运行日志。', access: 'read' },
      { id: 'deploy_db_check', title: '检查数据库状态', description: '检查数据库容器与连接是否就绪。', access: 'read' },
      { id: 'deploy_db_import', title: '导入数据库', description: '向指定主机的数据库导入 SQL 文件，会写入业务数据。', access: 'write', confirmation: '需确认' },
      { id: 'deploy_docker_install', title: '离线安装 Docker', description: '从离线包安装 Docker 与 Compose，会修改目标系统文件。', access: 'write', confirmation: '需确认' }
    ]
  },
  {
    id: 'updates', title: '更新', description: '更新远程部署，管理与检查应用更新',
    tools: [
      { id: 'deploy_update_remote', title: '更新远程部署', description: '预检更新内容，按组件替换文件并重启服务，支持更新前备份。', access: 'write', confirmation: '执行更新需确认' },
      { id: 'deploy_update_settings', title: '管理应用更新源', description: '查看或保存 MCP 使用的应用更新地址。', access: 'write', confirmation: '保存时需确认' },
      { id: 'deploy_app_update_check', title: '检查应用更新', description: '查询新版本与下载信息；安装更新请在桌面应用中操作。', access: 'read' }
    ]
  },
  {
    id: 'logs', title: '日志', description: '读取本机运行日志，定位下载、部署与连接问题',
    tools: [
      { id: 'deploy_runtime_logs_read', title: '读取运行日志', description: '按级别与关键词读取本机运行日志，最新记录在前。', access: 'read' },
      { id: 'deploy_runtime_logs_summary', title: '运行日志概览', description: '汇总日志占用、各级别条数与最近的警告和错误。', access: 'read' }
    ]
  }
];

/** 全部能力条目，按目录顺序展开。 */
export const mcpTools = mcpCategories.flatMap((category) => category.tools);

/**
 * 按分类、关键词与操作类型筛选能力目录，结果只保留仍有条目的分类。
 * 关键词只匹配展示文案，不匹配工具标识，避免筛选暴露注册层信息。
 */
export function filterMcpCategories({ category = 'all', keyword = '', access = 'all' } = {}) {
  const text = String(keyword ?? '').trim().toLocaleLowerCase();
  return mcpCategories
    .filter((item) => category === 'all' || item.id === category)
    .map((item) => ({
      ...item,
      tools: item.tools.filter((tool) => {
        const matchesAccess =
          access === 'all' ||
          tool.access === access ||
          (access === 'confirm' && Boolean(tool.confirmation));
        const content = `${item.title} ${item.description} ${tool.title} ${tool.description} ${tool.requirement || ''}`;
        return matchesAccess && content.toLocaleLowerCase().includes(text);
      })
    }))
    .filter((item) => item.tools.length > 0);
}
