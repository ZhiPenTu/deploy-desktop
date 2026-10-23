<template>
  <div class="setup-guide">
    <div class="guide-heading">
      <div>
        <h3>把部署舱接入 AI 助理</h3>
        <p>完成一次配置后，AI 助理会通过 MCP 读取项目资料并协助完成部署与运维操作。</p>
      </div>
      <span class="guide-badge">步骤 1 至 3</span>
    </div>

    <div class="guide-layout">
      <div class="guide-main">
        <section class="guide-step card-surface">
          <span class="step-number">1</span>
          <div class="step-content">
            <div class="step-title">
              <div>
                <h4>准备服务文件</h4>
                <p>在部署舱源码目录中安装 MCP 服务的运行依赖。运行服务的电脑需要 Node.js 20 或更高版本。</p>
              </div>
              <el-button link type="primary" @click="copyText(setupCommand, '安装命令')">
                <el-icon><CopyDocument /></el-icon>
                <span>复制命令</span>
              </el-button>
            </div>
            <pre class="command-block"><code>{{ setupCommand }}</code></pre>
            <p class="field-note">此方式需要部署舱源码目录；发布安装包不包含 MCP 服务文件。将第一行替换为已获取的目录，目录中应包含 mcp-server/index.mjs，安装完成后应显示服务版本号。</p>
          </div>
        </section>

        <section class="guide-step card-surface">
          <span class="step-number">2</span>
          <div class="step-content">
            <div class="step-title">
              <div>
                <h4>添加到 AI 助理</h4>
                <p>选择正在使用的客户端，把对应配置加入它的 MCP 设置。</p>
              </div>
            </div>

            <div class="client-options" role="group" aria-label="选择 AI 助理">
              <button
                v-for="client in clients"
                :key="client.id"
                type="button"
                class="client-option"
                :class="{ active: activeClient === client.id }"
                :aria-pressed="activeClient === client.id"
                @click="activeClient = client.id"
              >
                {{ client.label }}
              </button>
            </div>

            <div class="config-panel">
              <div class="config-toolbar">
                <div>
                  <span class="config-label">{{ currentClient.location }}</span>
                  <span class="config-format">{{ currentClient.format }}</span>
                </div>
                <el-button link type="primary" @click="copyText(currentClient.config, '配置内容')">
                  <el-icon><CopyDocument /></el-icon>
                  <span>复制配置</span>
                </el-button>
              </div>
              <pre class="config-block"><code>{{ currentClient.config }}</code></pre>
            </div>
            <p class="field-note">保留客户端中已有的其他服务配置，将这段内容合并到同一份配置文件。</p>
          </div>
        </section>

        <section class="guide-step card-surface">
          <span class="step-number">3</span>
          <div class="step-content">
            <div class="step-title">
              <div>
                <h4>重启并验证连接</h4>
                <p>保存配置后完全退出并重新打开 AI 助理，再发送下面的检查请求。</p>
              </div>
              <el-button link type="primary" @click="copyText(checkPrompt, '检查请求')">
                <el-icon><CopyDocument /></el-icon>
                <span>复制请求</span>
              </el-button>
            </div>
            <p class="check-prompt">{{ checkPrompt }}</p>
            <div class="success-check">
              <el-icon><CircleCheck /></el-icon>
              <span>连接成功时，AI 助理会返回当前项目与部署目标，并能发现 {{ mcpTools.length }} 项 MCP 能力。</span>
            </div>
          </div>
        </section>
      </div>

      <aside class="guide-side">
        <section class="card-surface side-card">
          <div class="side-title">
            <span class="icon-tile"><el-icon><Connection /></el-icon></span>
            <div>
              <h4>连接检查</h4>
              <p>连接状态由 AI 助理显示，部署舱不会代为启动服务。</p>
            </div>
          </div>
          <ul class="check-list">
            <li>AI 助理的 MCP 列表中显示 deploy-desktop</li>
            <li>能查看当前项目、主机与交付包</li>
            <li>修改或部署操作出现确认提示</li>
          </ul>
        </section>

        <section class="card-surface side-card">
          <div class="side-title compact">
            <span class="icon-tile warning"><el-icon><Warning /></el-icon></span>
            <div>
              <h4>连接不上时</h4>
              <p>按顺序检查下面的项目。</p>
            </div>
          </div>
          <dl class="troubleshoot-list">
            <div>
              <dt>AI 助理没有发现服务</dt>
              <dd>确认 Node.js 版本不低于 20，并检查配置中的服务文件路径是否存在。</dd>
            </div>
            <div>
              <dt>启动后立即退出</dt>
              <dd>先运行第一步的安装命令，确认 mcp-server/index.mjs 能显示版本号。</dd>
            </div>
            <div>
              <dt>找不到项目或交付包</dt>
              <dd>回到桌面端设置当前项目，并绑定已下载的交付包。</dd>
            </div>
            <div>
              <dt>看不到配置中的主机</dt>
              <dd>MCP 使用独立的主机与仓库配置；请在 AI 助理中重新保存目标主机。</dd>
            </div>
          </dl>
        </section>
      </aside>
    </div>
  </div>
</template>

<script setup>
import { computed, ref } from 'vue';
import { ElMessage } from 'element-plus';
import { CircleCheck, Connection, CopyDocument, Warning } from '@element-plus/icons-vue';
import { mcpTools } from '../catalog.js';

const servicePath = '/absolute/path/to/deploy-desktop/mcp-server/index.mjs';
const projectPath = '/absolute/path/to/deploy-desktop';
const setupCommand = `cd ${projectPath}
npm --prefix mcp-server ci
node mcp-server/index.mjs --version`;
const checkPrompt = '列出部署舱当前项目、绑定的交付包和部署目标。';

const clients = [
  {
    id: 'codex',
    label: 'Codex',
    location: '加入 ~/.codex/config.toml',
    format: 'TOML',
    config: `[mcp_servers.deploy-desktop]
command = "node"
args = ["${servicePath}"]
env = { DEPLOY_DESKTOP_ROOT = "${projectPath}" }`
  },
  {
    id: 'claude-desktop',
    label: 'Claude Desktop',
    location: '设置 > 开发者 > 编辑配置',
    format: 'JSON',
    config: `{
  "mcpServers": {
    "deploy-desktop": {
      "command": "node",
      "args": ["${servicePath}"],
      "env": { "DEPLOY_DESKTOP_ROOT": "${projectPath}" }
    }
  }
}`
  },
  {
    id: 'claude-code',
    label: 'Claude Code',
    location: '加入项目根目录的 .mcp.json',
    format: 'JSON',
    config: `{
  "mcpServers": {
    "deploy-desktop": {
      "command": "node",
      "args": ["${servicePath}"],
      "env": { "DEPLOY_DESKTOP_ROOT": "${projectPath}" }
    }
  }
}`
  },
  {
    id: 'cursor',
    label: 'Cursor',
    location: '加入项目根目录的 .cursor/mcp.json',
    format: 'JSON',
    config: `{
  "mcpServers": {
    "deploy-desktop": {
      "command": "node",
      "args": ["${servicePath}"],
      "env": { "DEPLOY_DESKTOP_ROOT": "${projectPath}" }
    }
  }
}`
  }
];

const activeClient = ref(clients[0].id);
const currentClient = computed(() => clients.find((client) => client.id === activeClient.value) || clients[0]);

async function copyText(content, label) {
  try {
    await navigator.clipboard.writeText(content);
    ElMessage.success(`${label}已复制`);
  } catch {
    ElMessage.error(`复制失败，请手动选择${label}`);
  }
}
</script>

<style scoped>
.setup-guide {
  height: 100%;
  overflow-y: auto;
  padding-right: 4px;
  scrollbar-gutter: stable;
}

.guide-heading {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 20px;
  padding: 2px 2px 16px;
}

.guide-heading h3 {
  font-size: 16px;
}

.guide-heading p {
  margin-top: 5px;
  color: var(--text-muted);
  font-size: 12px;
  line-height: 1.65;
}

.guide-badge {
  flex: none;
  padding: 5px 9px;
  border: 1px solid var(--border-color);
  border-radius: 999px;
  color: var(--text-secondary);
  background: var(--panel-bg);
  font-size: 11px;
}

.guide-layout {
  display: grid;
  grid-template-columns: minmax(0, 1fr) 286px;
  align-items: start;
  gap: 18px;
}

.guide-main,
.guide-side {
  display: flex;
  flex-direction: column;
  gap: 14px;
  min-width: 0;
}

.guide-step {
  display: flex;
  gap: 14px;
  padding: 18px;
}

.step-number {
  display: grid;
  place-items: center;
  flex: none;
  width: 28px;
  height: 28px;
  border-radius: 50%;
  color: var(--primary-color);
  background: var(--primary-soft);
  font-size: 12px;
  font-weight: 600;
}

.step-content {
  flex: 1;
  min-width: 0;
}

.step-title {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 16px;
}

.step-title h4,
.side-title h4 {
  font-size: 14px;
}

.step-title p,
.side-title p {
  margin-top: 4px;
  color: var(--text-muted);
  font-size: 12px;
  line-height: 1.6;
}

.step-title .el-button,
.config-toolbar .el-button {
  flex: none;
  margin-top: -5px;
}

.step-title .el-button span,
.config-toolbar .el-button span {
  margin-left: 4px;
}

.command-block,
.config-block {
  margin-top: 12px;
  padding: 12px 14px;
  overflow-x: auto;
  border: 1px solid var(--border-color);
  border-radius: var(--radius-sm);
  background: #f8fafc;
  color: var(--text-main);
  font-family: ui-monospace, "SFMono-Regular", Menlo, Monaco, Consolas, monospace;
  font-size: 11.5px;
  line-height: 1.75;
  tab-size: 2;
}

.command-block code,
.config-block code {
  padding: 0;
  border: 0;
  background: transparent;
  color: inherit;
  font: inherit;
}

.field-note {
  margin-top: 8px;
  color: var(--text-muted);
  font-size: 11px;
  line-height: 1.6;
}

.client-options {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  margin-top: 14px;
}

.client-option {
  min-height: 32px;
  padding: 5px 11px;
  border: 1px solid var(--border-color);
  border-radius: var(--radius-sm);
  background: var(--panel-bg);
  color: var(--text-secondary);
  font: inherit;
  font-size: 12px;
  cursor: pointer;
}

.client-option:hover {
  border-color: var(--border-strong);
}

.client-option.active {
  border-color: var(--brand-border);
  color: var(--primary-color);
  background: var(--primary-soft);
}

.client-option:focus-visible {
  outline: 2px solid var(--primary-color);
  outline-offset: 2px;
}

.config-panel {
  margin-top: 10px;
  overflow: hidden;
  border: 1px solid var(--border-color);
  border-radius: var(--radius-sm);
}

.config-toolbar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 14px;
  padding: 8px 12px;
  border-bottom: 1px solid var(--border-color);
  background: var(--panel-subtle);
}

.config-label {
  color: var(--text-secondary);
  font-size: 11px;
}

.config-format {
  margin-left: 8px;
  padding: 1px 5px;
  border: 1px solid var(--border-color);
  border-radius: var(--radius-xs);
  color: var(--text-muted);
  font-size: 10px;
}

.config-panel .config-block {
  margin-top: 0;
  border: 0;
  border-radius: 0;
}

.check-prompt {
  margin-top: 12px;
  padding: 11px 13px;
  border-left: 3px solid var(--primary-color);
  border-radius: 0 var(--radius-sm) var(--radius-sm) 0;
  background: var(--primary-soft);
  color: var(--text-secondary);
  font-size: 12px;
  line-height: 1.65;
}

.success-check {
  display: flex;
  align-items: flex-start;
  gap: 7px;
  margin-top: 10px;
  color: var(--success-color);
  font-size: 12px;
  line-height: 1.6;
}

.success-check .el-icon {
  flex: none;
  margin-top: 2px;
}

.side-card {
  padding: 17px;
}

.side-title {
  display: flex;
  align-items: flex-start;
  gap: 10px;
}

.side-title .icon-tile {
  width: 32px;
  height: 32px;
  border-radius: 8px;
  font-size: 15px;
}

.side-title.compact {
  align-items: center;
}

.side-title.compact p {
  margin-top: 2px;
}

.check-list {
  display: flex;
  flex-direction: column;
  gap: 9px;
  margin-top: 14px;
  padding-left: 17px;
  color: var(--text-secondary);
  font-size: 12px;
  line-height: 1.55;
}

.check-list li::marker {
  color: var(--primary-color);
}

.troubleshoot-list {
  margin-top: 14px;
}

.troubleshoot-list div + div {
  margin-top: 12px;
  padding-top: 12px;
  border-top: 1px solid var(--border-subtle);
}

.troubleshoot-list dt {
  color: var(--text-main);
  font-size: 12px;
  font-weight: 500;
  line-height: 1.5;
}

.troubleshoot-list dd {
  margin-top: 3px;
  color: var(--text-muted);
  font-size: 11px;
  line-height: 1.6;
}

@media (max-width: 1180px) {
  .guide-layout {
    grid-template-columns: minmax(0, 1fr) 250px;
  }
}

@media (max-width: 1050px) {
  .setup-guide {
    overflow: visible;
    padding-right: 0;
  }

  .guide-layout {
    grid-template-columns: 1fr;
  }

  .guide-side {
    display: grid;
    grid-template-columns: 1fr 1fr;
    align-items: start;
  }
}

@media (max-width: 700px) {
  .guide-heading {
    align-items: center;
  }

  .guide-step {
    gap: 10px;
    padding: 15px;
  }

  .step-title {
    display: block;
  }

  .step-title .el-button {
    margin: 6px 0 0;
  }

  .guide-side {
    grid-template-columns: 1fr;
  }

  .client-option {
    flex: 1 1 calc(50% - 6px);
  }

  .config-toolbar {
    align-items: flex-start;
  }

  .config-label {
    display: block;
  }

  .config-format {
    display: inline-block;
    margin: 4px 0 0;
  }
}
</style>
