<template>
  <div class="page-view mcp-view">
    <section class="card-surface service-overview" aria-label="MCP 服务概览">
      <div class="service-intro">
        <span class="icon-tile"><el-icon><Connection /></el-icon></span>
        <div>
          <h2>让 AI 客户端协助部署与运维</h2>
          <p>服务由 MCP 客户端启动并调用，连接状态请在客户端中查看。</p>
        </div>
      </div>
      <dl class="overview-stats">
        <div><dt>工具能力</dt><dd>{{ mcpTools.length }}<span>项</span></dd></div>
        <div><dt>能力分类</dt><dd>{{ mcpCategories.length }}<span>类</span></dd></div>
        <div><dt>只读查询</dt><dd>{{ readOnlyCount }}<span>项</span></dd></div>
      </dl>
      <p class="config-note">主机、镜像仓库与更新源由 MCP 单独保存；项目与制品源使用桌面端配置，使用前请核对目标与配置。</p>
    </section>

    <div class="capability-workspace">
      <nav class="card-surface category-nav" aria-label="MCP 能力分类">
        <h3 class="nav-heading">能力分类</h3>
        <button
          v-for="category in categoryOptions"
          :key="category.id"
          type="button"
          class="category-button"
          :class="{ active: activeCategory === category.id }"
          :aria-pressed="activeCategory === category.id"
          @click="activeCategory = category.id"
        >
          <el-icon><component :is="category.id === 'all' ? Grid : categoryIcons[category.id]" /></el-icon>
          <span>{{ category.title }}</span>
          <span class="category-count">{{ category.count }}</span>
        </button>
      </nav>

      <div class="card-surface capability-panel">
        <div class="capability-toolbar">
          <el-input
            v-model="query"
            class="capability-search"
            placeholder="搜索能力名称或用途"
            aria-label="搜索 MCP 能力"
            :prefix-icon="Search"
            clearable
          />
          <el-select v-model="accessFilter" class="access-filter" aria-label="筛选操作类型">
            <el-option label="全部操作" value="all" />
            <el-option label="只读查询" value="read" />
            <el-option label="更改操作" value="write" />
            <el-option label="含确认步骤" value="confirm" />
          </el-select>
          <span class="result-count" role="status" aria-live="polite">{{ visibleToolCount }} 项能力</span>
          <el-button v-if="hasFilters" link type="primary" @click="resetFilters">重置筛选</el-button>
        </div>

        <div ref="resultsPanel" class="capability-results" role="region" aria-label="MCP 能力列表" tabindex="0">
          <div v-if="visibleCategories.length" class="capability-groups">
            <section
              v-for="category in visibleCategories"
              :key="category.id"
              class="capability-category"
              :aria-labelledby="`mcp-category-${category.id}`"
            >
              <div class="category-heading">
                <span class="icon-tile neutral"><el-icon><component :is="categoryIcons[category.id]" /></el-icon></span>
                <div class="category-copy">
                  <h3 :id="`mcp-category-${category.id}`">{{ category.title }}</h3>
                  <p>{{ category.description }}</p>
                </div>
                <el-tag type="info" effect="plain" size="small" disable-transitions>{{ category.tools.length }} 项</el-tag>
              </div>
              <ul class="capability-list">
                <li v-for="tool in category.tools" :key="tool.id" class="capability-item">
                  <div class="tool-copy">
                    <h4>{{ tool.title }}</h4>
                    <div class="tool-description">
                      <p>{{ tool.description }}</p>
                      <p v-if="tool.requirement" class="requirement-note">{{ tool.requirement }}</p>
                    </div>
                  </div>
                  <el-tag :type="tool.confirmation ? 'warning' : 'info'" effect="plain" size="small" disable-transitions>
                    {{ tool.confirmation || (tool.access === 'read' ? '只读' : '更改操作') }}
                  </el-tag>
                </li>
              </ul>
            </section>
          </div>
          <div v-else class="empty-results">
            <el-empty description="没有匹配的 MCP 能力，请调整关键词或筛选条件。">
              <el-button type="primary" plain @click="resetFilters">重置筛选</el-button>
            </el-empty>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>

<script setup>
import { computed, ref, watch } from 'vue';
import { Box, Connection, Document, Download, Files, FolderOpened, Grid, Monitor, Operation, Search, Setting, Tickets } from '@element-plus/icons-vue';
import { filterMcpCategories, mcpCategories, mcpTools } from '../catalog.js';

const categoryIcons = {
  profile: Setting, projects: FolderOpened, hosts: Monitor, registries: Box,
  manifests: Document, images: Files, bundles: Download, artifacts: Tickets,
  operations: Operation, updates: Connection
};
const readOnlyCount = mcpTools.filter((tool) => tool.access === 'read').length;
const categoryOptions = [
  { id: 'all', title: '全部能力', count: mcpTools.length },
  ...mcpCategories.map((category) => ({ id: category.id, title: category.title, count: category.tools.length }))
];
const activeCategory = ref('all');
const query = ref('');
const accessFilter = ref('all');
const resultsPanel = ref(null);
const hasFilters = computed(() => activeCategory.value !== 'all' || Boolean(query.value) || accessFilter.value !== 'all');
const visibleCategories = computed(() =>
  filterMcpCategories({
    category: activeCategory.value,
    keyword: query.value,
    access: accessFilter.value
  })
);
const visibleToolCount = computed(() => visibleCategories.value.reduce((total, category) => total + category.tools.length, 0));

watch([activeCategory, query, accessFilter], () => {
  resultsPanel.value?.scrollTo({ top: 0 });
}, { flush: 'post' });

function resetFilters() {
  activeCategory.value = 'all';
  query.value = '';
  accessFilter.value = 'all';
}
</script>

<style scoped>
.mcp-view {
  display: flex;
  flex-direction: column;
  gap: 20px;
  overflow: hidden;
  padding-top: 24px;
  padding-bottom: 24px;
}

.service-overview {
  display: grid;
  grid-template-columns: minmax(0, 1fr) auto;
  align-items: center;
  flex: none;
  gap: 14px 24px;
  padding: 20px;
}

.service-intro,
.category-heading {
  display: flex;
  align-items: center;
  gap: 12px;
}

.service-intro h2 {
  font-size: 15px;
  line-height: 1.5;
}

.service-intro p,
.config-note,
.category-copy p,
.capability-item p {
  font-size: 12px;
  line-height: 1.65;
  color: var(--text-muted);
}

.service-intro p {
  margin-top: 4px;
}

.overview-stats {
  display: flex;
  gap: 24px;
  padding-left: 24px;
  border-left: 1px solid var(--border-color);
}

.overview-stats dt {
  font-size: 11px;
  color: var(--text-muted);
}

.overview-stats dd {
  margin-top: 3px;
  font-size: 22px;
  font-weight: 600;
  font-variant-numeric: tabular-nums;
}

.overview-stats dd span {
  margin-left: 5px;
  font-size: 11px;
  font-weight: 400;
  color: var(--text-muted);
}

.config-note {
  grid-column: 1 / -1;
  margin: 0;
  padding-top: 12px;
  border-top: 1px solid var(--border-color);
}

.capability-workspace {
  display: grid;
  grid-template-columns: 176px minmax(0, 1fr);
  align-items: start;
  flex: 1;
  min-height: 0;
  gap: 20px;
}

.category-nav {
  display: flex;
  flex-direction: column;
  gap: 4px;
  max-height: 100%;
  padding: 12px;
  overflow-y: auto;
}

.nav-heading {
  margin: 4px 10px 8px;
  font-size: 11px;
  font-weight: 500;
  color: var(--text-muted);
}

.category-button {
  display: flex;
  align-items: center;
  flex: none;
  gap: 10px;
  min-height: 38px;
  padding: 8px 10px;
  border: 1px solid transparent;
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--text-secondary);
  font: inherit;
  font-size: 12px;
  text-align: left;
  cursor: pointer;
}

.category-button:hover {
  background: var(--panel-subtle);
}

.category-button.active {
  color: var(--primary-color);
  background: var(--primary-soft);
}

.category-button:focus-visible,
.capability-results:focus-visible {
  outline: 2px solid var(--primary-color);
  outline-offset: -2px;
}

.category-button .el-icon {
  font-size: 15px;
}

.category-count {
  margin-left: auto;
  font-size: 11px;
  font-variant-numeric: tabular-nums;
  opacity: 0.8;
}

.capability-panel {
  display: flex;
  flex-direction: column;
  height: 100%;
  min-height: 0;
  overflow: hidden;
}

.capability-toolbar {
  display: flex;
  align-items: center;
  flex: none;
  flex-wrap: wrap;
  gap: 10px;
  padding: 16px 20px;
  border-bottom: 1px solid var(--border-color);
}

.capability-search {
  flex: 1;
  min-width: 180px;
  max-width: 360px;
}

.access-filter {
  width: 140px;
}

.result-count {
  margin-left: auto;
  color: var(--text-muted);
  font-size: 12px;
  white-space: nowrap;
}

.capability-toolbar .el-button {
  margin-left: 0;
}

.capability-results {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  overscroll-behavior: contain;
  scrollbar-gutter: stable;
}

.capability-category + .capability-category {
  border-top: 1px solid var(--border-color);
}

.category-heading {
  padding: 14px 20px;
  background: var(--panel-subtle);
  border-bottom: 1px solid var(--border-color);
}

.category-heading .icon-tile {
  width: 32px;
  height: 32px;
  border-radius: 8px;
  font-size: 16px;
  background: var(--panel-bg);
}

.category-copy {
  flex: 1;
  min-width: 0;
}

.category-copy h3 {
  font-size: 14px;
}

.category-copy p {
  margin-top: 2px;
}

.capability-list {
  padding: 0 20px;
  list-style: none;
}

.capability-item {
  display: grid;
  grid-template-columns: minmax(0, 1fr) auto;
  align-items: start;
  gap: 16px;
  padding: 16px 0;
}

.capability-item + .capability-item {
  border-top: 1px solid var(--border-color);
}

.tool-copy {
  display: grid;
  grid-template-columns: 150px minmax(0, 1fr);
  align-items: baseline;
  gap: 16px;
  min-width: 0;
}

.tool-copy h4 {
  font-size: 13px;
  font-weight: 500;
  line-height: 1.65;
}

.capability-item > .el-tag {
  margin-top: 1px;
}

.capability-item .requirement-note {
  margin-top: 6px;
  color: var(--warning-color);
}

.empty-results {
  display: grid;
  place-items: center;
  min-height: 100%;
  padding: 20px;
}

@media (max-width: 1280px) {
  .tool-copy {
    grid-template-columns: 1fr;
    gap: 4px;
  }
}

@media (max-width: 1000px) {
  .service-overview {
    grid-template-columns: 1fr;
  }

  .overview-stats {
    grid-row: 2;
    gap: 36px;
    padding-left: 0;
    border-left: 0;
  }
  .mcp-view {
    overflow-y: auto;
  }

  .capability-workspace {
    grid-template-columns: 1fr;
    flex: none;
    gap: 14px;
  }

  .category-nav {
    flex-direction: row;
    flex-wrap: wrap;
    gap: 6px;
    max-height: none;
    overflow: visible;
    padding: 8px;
  }

  .nav-heading {
    display: none;
  }

  .category-button {
    white-space: nowrap;
  }

  .capability-panel {
    height: auto;
  }

  .capability-results {
    overflow: visible;
    scrollbar-gutter: auto;
  }

  .capability-toolbar {
    padding: 14px 16px;
  }
}

@media (max-width: 700px) {
  .mcp-view {
    padding: 22px 16px 30px;
  }

  .service-overview {
    padding: 18px;
  }

  .service-intro {
    align-items: flex-start;
  }

  .overview-stats {
    justify-content: space-between;
    gap: 12px;
  }

  .capability-search {
    flex-basis: 100%;
    min-width: 0;
    max-width: none;
  }

  .access-filter {
    width: 132px;
  }

  .capability-toolbar > .el-button {
    flex-basis: 100%;
    justify-content: flex-start;
  }

  .category-button .el-icon {
    display: none;
  }

  .category-heading {
    flex-wrap: wrap;
    padding: 16px;
  }

  .capability-list {
    padding: 0 16px;
  }

  .capability-item {
    gap: 8px;
  }
}
</style>
