<template>
  <div class="page-view mcp-view">
    <div class="page-header">
      <div>
        <h2 class="page-title">MCP 服务</h2>
        <p class="page-subtitle">通过支持 MCP 的 AI 客户端使用主机、镜像与部署能力</p>
      </div>
      <el-tag type="info" effect="plain">能力概览</el-tag>
    </div>

    <section class="card-surface service-overview" aria-label="MCP 服务概览">
      <div class="service-intro">
        <span class="icon-tile"><el-icon><Connection /></el-icon></span>
        <div>
          <h3>让 AI 客户端协助部署与运维</h3>
          <p>服务由 MCP 客户端启动并调用，连接状态请在客户端中查看。</p>
        </div>
      </div>
      <dl class="overview-stats">
        <div><dt>工具能力</dt><dd>{{ allTools.length }}<span>项</span></dd></div>
        <div><dt>能力分类</dt><dd>{{ mcpCategories.length }}<span>类</span></dd></div>
        <div><dt>只读查询</dt><dd>{{ readOnlyCount }}<span>项</span></dd></div>
      </dl>
      <p class="config-note">MCP 的主机、镜像仓库与更新源单独保存，使用前请在 MCP 客户端中核对目标与配置。</p>
    </section>

    <nav class="category-nav" aria-label="MCP 能力分类">
      <button
        v-for="category in categoryOptions"
        :key="category.id"
        type="button"
        class="category-button"
        :class="{ active: activeCategory === category.id }"
        :aria-pressed="activeCategory === category.id"
        @click="activeCategory = category.id"
      >
        <span>{{ category.title }}</span>
        <span class="category-count">{{ category.count }}</span>
      </button>
    </nav>

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
      <span class="result-count" role="status">{{ visibleToolCount }} 项能力</span>
      <el-button v-if="hasFilters" link type="primary" @click="resetFilters">重置筛选</el-button>
    </div>

    <div v-if="visibleCategories.length" class="capability-grid">
      <section
        v-for="category in visibleCategories"
        :key="category.id"
        class="card-surface capability-category"
        :aria-labelledby="`mcp-category-${category.id}`"
      >
        <div class="category-heading">
          <span class="icon-tile neutral"><el-icon><component :is="categoryIcons[category.id]" /></el-icon></span>
          <div class="category-copy">
            <h3 :id="`mcp-category-${category.id}`">{{ category.title }}</h3>
            <p>{{ category.description }}</p>
          </div>
          <el-tag type="info" effect="plain" size="small">{{ category.tools.length }} 项</el-tag>
        </div>
        <ul class="capability-list">
          <li v-for="tool in category.tools" :key="tool.id" class="capability-item">
            <div class="tool-heading">
              <h4>{{ tool.title }}</h4>
              <el-tag :type="tool.confirmation ? 'warning' : 'info'" effect="plain" size="small">
                {{ tool.confirmation || (tool.access === 'read' ? '只读' : '更改操作') }}
              </el-tag>
            </div>
            <p>{{ tool.description }}</p>
            <p v-if="tool.requirement" class="requirement-note">{{ tool.requirement }}</p>
          </li>
        </ul>
      </section>
    </div>
    <div v-else class="card-surface empty-results">
      <el-empty description="没有匹配的 MCP 能力，请调整关键词或筛选条件。">
        <el-button type="primary" plain @click="resetFilters">重置筛选</el-button>
      </el-empty>
    </div>
  </div>
</template>

<script setup>
import { computed, ref } from 'vue';
import { Box, Connection, Document, Download, Files, Monitor, Operation, Search, Setting } from '@element-plus/icons-vue';
import { mcpCategories } from '../catalog.js';

const categoryIcons = {
  profile: Setting, hosts: Monitor, registries: Box, manifests: Document,
  images: Files, bundles: Download, operations: Operation, updates: Connection
};
const allTools = mcpCategories.flatMap((category) => category.tools);
const readOnlyCount = allTools.filter((tool) => tool.access === 'read').length;
const categoryOptions = [
  { id: 'all', title: '全部能力', count: allTools.length },
  ...mcpCategories.map((category) => ({ id: category.id, title: category.title, count: category.tools.length }))
];
const activeCategory = ref('all');
const query = ref('');
const accessFilter = ref('all');
const hasFilters = computed(() => activeCategory.value !== 'all' || Boolean(query.value) || accessFilter.value !== 'all');
const visibleCategories = computed(() => {
  const keyword = query.value.trim().toLocaleLowerCase();
  return mcpCategories
    .filter((category) => activeCategory.value === 'all' || category.id === activeCategory.value)
    .map((category) => ({
      ...category,
      tools: category.tools.filter((tool) => {
        const matchesAccess = accessFilter.value === 'all' || tool.access === accessFilter.value ||
          (accessFilter.value === 'confirm' && Boolean(tool.confirmation));
        const content = `${category.title} ${category.description} ${tool.title} ${tool.description} ${tool.requirement || ''}`;
        return matchesAccess && content.toLocaleLowerCase().includes(keyword);
      })
    }))
    .filter((category) => category.tools.length > 0);
});
const visibleToolCount = computed(() => visibleCategories.value.reduce((total, category) => total + category.tools.length, 0));

function resetFilters() {
  activeCategory.value = 'all';
  query.value = '';
  accessFilter.value = 'all';
}
</script>

<style scoped>
.service-overview {
  display: grid;
  grid-template-columns: minmax(0, 1fr) auto;
  gap: 18px 28px;
  padding: 22px 24px;
}

.service-intro,
.category-heading,
.tool-heading {
  display: flex;
  align-items: center;
  gap: 12px;
}

.service-intro h3 {
  font-size: 15px;
}

.service-intro p,
.config-note,
.category-copy p,
.capability-item p {
  margin-top: 6px;
  font-size: 12px;
  line-height: 1.65;
  color: var(--text-muted);
}

.overview-stats {
  display: flex;
  gap: 30px;
}

.overview-stats dt {
  font-size: 11px;
  color: var(--text-muted);
}

.overview-stats dd {
  margin-top: 4px;
  font-size: 24px;
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
  padding-top: 14px;
  border-top: 1px solid var(--border-color);
}

.category-nav {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
  margin: 22px 0 18px;
}

.category-button {
  display: inline-flex;
  align-items: center;
  gap: 8px;
  padding: 8px 12px;
  border: 1px solid var(--border-color);
  border-radius: var(--radius-sm);
  background: var(--panel-bg);
  color: var(--text-secondary);
  font: inherit;
  font-size: 12px;
  cursor: pointer;
}

.category-button:hover,
.category-button.active {
  border-color: var(--primary-color);
  color: var(--primary-color);
  background: var(--primary-soft);
}

.category-button:focus-visible {
  outline: 2px solid var(--primary-color);
  outline-offset: 2px;
}

.category-count {
  font-size: 11px;
  font-variant-numeric: tabular-nums;
  opacity: 0.8;
}

.capability-toolbar {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 12px;
  margin-bottom: 18px;
}

.capability-search {
  width: 300px;
  max-width: 100%;
}

.access-filter {
  width: 156px;
}

.result-count {
  color: var(--text-muted);
  font-size: 12px;
}

.capability-grid {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  align-items: start;
  gap: 18px;
}

.category-heading {
  padding: 18px 20px;
  border-bottom: 1px solid var(--border-color);
}

.category-copy {
  flex: 1;
  min-width: 0;
}

.category-copy h3 {
  font-size: 14px;
}

.category-copy p {
  margin-top: 3px;
}

.capability-list {
  padding: 0 20px;
  list-style: none;
}

.capability-item {
  padding: 15px 0;
}

.capability-item + .capability-item {
  border-top: 1px solid var(--border-color);
}

.tool-heading {
  justify-content: space-between;
  flex-wrap: wrap;
  gap: 6px;
}

.tool-heading h4 {
  font-size: 13px;
  font-weight: 500;
}

.capability-item .requirement-note {
  color: var(--warning-color);
}

@media (max-width: 1180px) {
  .service-overview {
    grid-template-columns: 1fr;
  }

  .overview-stats {
    gap: 42px;
  }

  .capability-grid {
    grid-template-columns: 1fr;
  }
}

@media (max-width: 700px) {
  .mcp-view {
    padding: 22px 16px 30px;
  }

  .mcp-view .page-header {
    align-items: flex-start;
    flex-wrap: wrap;
    gap: 10px;
  }

  .service-overview {
    padding: 18px;
  }

  .service-intro {
    align-items: flex-start;
  }

  .overview-stats {
    flex-wrap: wrap;
    gap: 18px;
  }

  .category-heading {
    flex-wrap: wrap;
    padding: 16px;
  }

  .capability-list {
    padding: 0 16px;
  }
}
</style>
