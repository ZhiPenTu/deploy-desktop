<template>
  <div class="runtime-logs">
    <section class="log-overview">
      <div class="overview-head">
        <div class="overview-title">
          <span class="icon-tile"><el-icon><Document /></el-icon></span>
          <div>
            <h3>运行日志</h3>
            <p>记录下载、部署、连接与数据库操作的完整过程，日志只保存在本机</p>
          </div>
        </div>
        <div class="overview-actions">
          <el-button :loading="loading" @click="load">
            <el-icon><Refresh /></el-icon>
            <span>刷新</span>
          </el-button>
          <el-button @click="openDir">
            <el-icon><FolderOpened /></el-icon>
            <span>打开目录</span>
          </el-button>
          <el-button type="danger" plain :disabled="!totalBytes" @click="clear">
            <el-icon><Delete /></el-icon>
            <span>清空</span>
          </el-button>
        </div>
      </div>

      <div class="overview-meter">
        <div class="meter-head">
          <span>日志占用</span>
          <strong>{{ formatSize(totalBytes) }}<em>/ {{ formatSize(maxBytes) }}</em></strong>
        </div>
        <el-progress
          :percentage="usagePercent"
          :stroke-width="6"
          :show-text="false"
          :status="usageStatus"
        />
        <p class="meter-hint">写满后自动轮转，最多保留两份，合计不超过 {{ formatSize(maxBytes) }}</p>
      </div>

      <div class="overview-path">
        <div class="path-head">
          <span>日志位置</span>
          <span v-if="settings.isCustom" class="path-tag">自定义</span>
        </div>
        <div class="path-body">
          <span class="path-value" :title="currentDir">{{ currentDir || '尚未确定' }}</span>
          <div class="path-actions">
            <el-button link :disabled="!currentDir" @click="copyDir">复制路径</el-button>
            <el-button link @click="changeDir">更改目录</el-button>
            <el-button v-if="settings.isCustom" link @click="resetDir">恢复默认</el-button>
          </div>
        </div>
      </div>
    </section>

    <section class="log-filters">
      <button
        v-for="item in filters"
        :key="item.value"
        type="button"
        class="filter-card"
        :class="[`is-${item.tone}`, { 'is-active': level === item.value }]"
        @click="selectLevel(item.value)"
      >
        <span class="filter-label">{{ item.label }}</span>
        <span class="filter-count">{{ item.count }}</span>
      </button>
    </section>

    <section class="log-panel">
      <div class="log-toolbar">
        <el-input
          v-model="keyword"
          class="log-search"
          placeholder="搜索内容或来源"
          clearable
          :prefix-icon="Search"
          @input="onKeywordInput"
        />
        <div class="toolbar-right">
          <span class="toolbar-meta">{{ summaryText }}</span>
          <label class="debug-toggle">
            <span class="debug-toggle-label">详细调试</span>
            <el-switch v-model="debugEnabled" :loading="switching" @change="toggleDebug" />
          </label>
          <el-button link :disabled="!rows.length" @click="copyVisible">
            <el-icon><CopyDocument /></el-icon>
            <span>复制当前视图</span>
          </el-button>
        </div>
      </div>

      <p v-if="debugEnabled" class="debug-hint">
        详细调试已开启，之后每一次操作都会记录耗时与结果；排查结束后建议关闭。
      </p>

      <div v-loading="loading" class="log-stream">
        <template v-for="group in grouped" :key="group.date">
          <div class="log-date">{{ group.label }}</div>
          <div
            v-for="row in group.rows"
            :key="row.key"
            class="log-item"
            :class="`is-${row.tone}`"
          >
            <span class="log-marker" />
            <span class="log-time">{{ row.clock }}</span>
            <span class="log-level">{{ row.levelText }}</span>
            <span class="log-target">{{ row.target }}</span>
            <span
              class="log-text"
              :class="{ 'is-open': expanded.has(row.key) }"
              @click="toggleRow(row.key)"
            >{{ row.message }}</span>
          </div>
        </template>

        <div v-if="!loading && !rows.length" class="log-empty">
          <span class="empty-icon"><el-icon><Document /></el-icon></span>
          <span class="empty-title">{{ emptyTitle }}</span>
          <span class="empty-hint">{{ emptyHint }}</span>
        </div>
      </div>
    </section>
  </div>
</template>

<script setup>
import { computed, onMounted, reactive, ref } from 'vue';
import { ElMessage, ElMessageBox } from 'element-plus';
import {
  CopyDocument,
  Delete,
  Document,
  FolderOpened,
  Refresh,
  Search
} from '@element-plus/icons-vue';
import {
  clearRuntimeLogs,
  getRuntimeLogSettings,
  openRuntimeLogDir,
  readRuntimeLogs,
  selectLocalPath,
  setCommandTrace,
  setRuntimeLogDir,
  setRuntimeLogLevel
} from '../../../api/tauri.js';
import { errorText } from '../../../shared/errors.js';

const level = ref('');
const keyword = ref('');
const loading = ref(false);
const switching = ref(false);
const debugEnabled = ref(false);
const expanded = reactive(new Set());
const settings = ref({ dir: '', defaultDir: '', isCustom: false });
const snapshot = ref({
  dir: '',
  entries: [],
  counts: { info: 0, warn: 0, error: 0, debug: 0 },
  matched: 0,
  truncated: false,
  totalBytes: 0,
  maxBytes: 10 * 1024 * 1024,
  level: 'info'
});

let keywordTimer = null;

const currentDir = computed(() => settings.value.dir || snapshot.value.dir || '');

const totalBytes = computed(() => Number(snapshot.value.totalBytes || 0));

const maxBytes = computed(() => Number(snapshot.value.maxBytes || 0));

const usagePercent = computed(() => {
  const max = Number(snapshot.value.maxBytes || 1);
  return Math.min(100, Math.round((totalBytes.value / max) * 100));
});

const usageStatus = computed(() => {
  if (usagePercent.value >= 90) return 'exception';
  if (usagePercent.value >= 70) return 'warning';
  return '';
});

const filters = computed(() => {
  const items = [
    { value: '', label: '全部', tone: 'all', count: totalCount.value },
    { value: 'info', label: '正常', tone: 'info', count: snapshot.value.counts.info },
    { value: 'warn', label: '警告', tone: 'warn', count: snapshot.value.counts.warn },
    { value: 'error', label: '错误', tone: 'error', count: snapshot.value.counts.error }
  ];
  if (snapshot.value.counts.debug > 0 || level.value === 'debug') {
    items.push({ value: 'debug', label: '调试', tone: 'debug', count: snapshot.value.counts.debug });
  }
  return items;
});

const totalCount = computed(() => {
  const counts = snapshot.value.counts;
  return counts.info + counts.warn + counts.error + counts.debug;
});

const rows = computed(() =>
  snapshot.value.entries.map((entry, index) => ({
    key: `${entry.time}-${index}`,
    time: entry.time,
    clock: entry.time.slice(11) || entry.time,
    date: entry.date || entry.time.slice(0, 10),
    levelText: levelText(entry.level),
    tone: toneOf(entry.level),
    target: entry.target,
    message: entry.message
  }))
);

const grouped = computed(() => {
  const groups = [];
  for (const row of rows.value) {
    const current = groups[groups.length - 1];
    if (current && current.date === row.date) {
      current.rows.push(row);
    } else {
      groups.push({ date: row.date, label: dateLabel(row.date), rows: [row] });
    }
  }
  return groups;
});

const summaryText = computed(() => {
  if (!rows.value.length) return '没有匹配的日志';
  if (snapshot.value.truncated) {
    return `显示最近 ${rows.value.length} 条，共匹配 ${snapshot.value.matched} 条`;
  }
  return `共 ${rows.value.length} 条`;
});

const emptyTitle = computed(() => {
  if (keyword.value.trim()) return '没有匹配的日志';
  if (level.value) return '当前分类还没有日志';
  return '还没有运行日志';
});

const emptyHint = computed(() =>
  keyword.value.trim()
    ? '换一个关键词，或切换分类后再试一次。'
    : '执行下载、部署或连接操作后，这里会记录过程和失败原因。'
);

function levelText(value) {
  if (value === 'ERROR') return '错误';
  if (value === 'WARN') return '警告';
  if (value === 'DEBUG' || value === 'TRACE') return '调试';
  return '正常';
}

function toneOf(value) {
  if (value === 'ERROR') return 'error';
  if (value === 'WARN') return 'warn';
  if (value === 'DEBUG' || value === 'TRACE') return 'debug';
  return 'info';
}

function dateLabel(date) {
  const today = new Date();
  const stamp = new Date(`${date}T00:00:00`);
  const diff = Math.round((new Date(today.toDateString()) - new Date(stamp.toDateString())) / 86400000);
  if (diff === 0) return `今天 · ${date}`;
  if (diff === 1) return `昨天 · ${date}`;
  return date;
}

function formatSize(bytes) {
  const value = Number(bytes || 0);
  if (!value) return '0 MB';
  const units = ['B', 'KB', 'MB', 'GB'];
  let size = value;
  let index = 0;
  while (size >= 1024 && index < units.length - 1) {
    size /= 1024;
    index += 1;
  }
  return `${size.toFixed(index <= 1 ? 0 : 1)} ${units[index]}`;
}

function selectLevel(value) {
  level.value = value;
  load();
}

function toggleRow(key) {
  if (expanded.has(key)) expanded.delete(key);
  else expanded.add(key);
}

function onKeywordInput() {
  if (keywordTimer) clearTimeout(keywordTimer);
  keywordTimer = setTimeout(load, 350);
}

async function load() {
  loading.value = true;
  try {
    snapshot.value = await readRuntimeLogs(level.value, 500, keyword.value.trim());
    debugEnabled.value = snapshot.value.level === 'debug';
    setCommandTrace(debugEnabled.value);
    expanded.clear();
  } catch (error) {
    ElMessage.error(errorText(error));
  } finally {
    loading.value = false;
  }
}

async function toggleDebug(value) {
  switching.value = true;
  try {
    const next = await setRuntimeLogLevel(value ? 'debug' : 'info');
    debugEnabled.value = next === 'debug';
    setCommandTrace(debugEnabled.value);
    await load();
  } catch (error) {
    debugEnabled.value = !value;
    ElMessage.error(errorText(error));
  } finally {
    switching.value = false;
  }
}

async function loadSettings() {
  try {
    settings.value = await getRuntimeLogSettings();
  } catch {
    settings.value = { dir: snapshot.value.dir, defaultDir: '', isCustom: false };
  }
}

async function changeDir() {
  let selected = '';
  try {
    selected = await selectLocalPath({
      directory: true,
      multiple: false,
      title: '选择日志保存目录'
    });
  } catch {
    ElMessage.error('无法选择目录，请重试');
    return;
  }
  if (!selected) return;
  try {
    await ElMessageBox.confirm(
      `之后产生的日志会写入 ${selected}，现有日志保留在原目录。是否继续？`,
      '更改日志位置',
      { type: 'info', confirmButtonText: '更改', cancelButtonText: '取消' }
    );
  } catch {
    return;
  }
  try {
    settings.value = await setRuntimeLogDir(selected);
    await load();
    ElMessage.success('日志位置已更新');
  } catch (error) {
    ElMessage.error(errorText(error));
  }
}

async function resetDir() {
  try {
    await ElMessageBox.confirm(
      '恢复后新日志写回应用默认目录，自定义目录中的日志不会被删除。是否继续？',
      '恢复默认日志位置',
      { type: 'info', confirmButtonText: '恢复默认', cancelButtonText: '取消' }
    );
  } catch {
    return;
  }
  try {
    settings.value = await setRuntimeLogDir('');
    await load();
    ElMessage.success('已恢复默认日志位置');
  } catch (error) {
    ElMessage.error(errorText(error));
  }
}

async function copyVisible() {
  const text = rows.value
    .map((row) => `${row.time} ${row.levelText} ${row.target} - ${row.message}`)
    .join('\n');
  try {
    await navigator.clipboard.writeText(text);
    ElMessage.success('已复制当前视图');
  } catch {
    ElMessage.error('复制失败，请在列表中手动选择内容');
  }
}

async function copyDir() {
  try {
    await navigator.clipboard.writeText(currentDir.value);
    ElMessage.success('已复制日志位置');
  } catch {
    ElMessage.error('复制失败，请手动选择路径');
  }
}

async function openDir() {
  try {
    await openRuntimeLogDir();
  } catch (error) {
    ElMessage.error(errorText(error));
  }
}

async function clear() {
  try {
    await ElMessageBox.confirm('清空后无法恢复，是否继续？', '清空运行日志', {
      type: 'warning',
      confirmButtonText: '清空',
      cancelButtonText: '取消'
    });
  } catch {
    return;
  }
  try {
    await clearRuntimeLogs();
    ElMessage.success('运行日志已清空');
    await load();
  } catch (error) {
    ElMessage.error(errorText(error));
  }
}

onMounted(async () => {
  await load();
  await loadSettings();
});
</script>

<style scoped>
.runtime-logs {
  display: flex;
  flex-direction: column;
  gap: 16px;
}

.log-overview {
  display: flex;
  flex-direction: column;
  gap: 18px;
  padding: 20px 22px;
  border: 1px solid var(--border-color);
  border-radius: var(--radius-md);
  background: linear-gradient(180deg, #fbfcff 0%, #ffffff 68%);
  box-shadow: var(--shadow-card);
}

.overview-head {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 20px;
  flex-wrap: wrap;
}

.overview-title {
  display: flex;
  gap: 12px;
  align-items: flex-start;
}

.overview-title h3 {
  font-size: 16px;
  font-weight: 650;
  letter-spacing: -0.01em;
  color: var(--text-main);
}

.overview-title p {
  margin-top: 4px;
  font-size: 12.5px;
  line-height: 1.55;
  color: var(--text-muted);
}

.overview-actions {
  display: flex;
  gap: 8px;
  flex-wrap: wrap;
}

.overview-meter {
  display: flex;
  flex-direction: column;
  gap: 8px;
  padding-top: 14px;
  border-top: 1px solid var(--border-color);
}

.meter-head {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  font-size: 12px;
  color: var(--text-muted);
}

.meter-head strong {
  font-size: 13px;
  font-weight: 600;
  color: var(--text-main);
  font-variant-numeric: tabular-nums;
}

.meter-head em {
  margin-left: 4px;
  font-style: normal;
  font-weight: 500;
  color: var(--text-faint);
}

.meter-hint {
  font-size: 12px;
  color: var(--text-faint);
}

.overview-path {
  display: flex;
  flex-direction: column;
  gap: 8px;
  padding-top: 14px;
  border-top: 1px solid var(--border-color);
}

.path-head {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 12px;
  color: var(--text-muted);
}

.path-tag {
  padding: 1px 8px;
  border-radius: 999px;
  font-size: 11px;
  color: var(--primary-color);
  background: var(--primary-soft);
}

.path-body {
  display: flex;
  align-items: center;
  gap: 12px;
}

.path-value {
  flex: 1;
  min-width: 0;
  font-family: var(--font-mono, ui-monospace, SFMono-Regular, Menlo, monospace);
  font-size: 12px;
  line-height: 1.6;
  color: var(--text-primary);
  user-select: text;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.path-actions {
  display: flex;
  align-items: center;
  gap: 4px;
  flex: none;
}

.log-filters {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(150px, 1fr));
  gap: 12px;
}

.filter-card {
  display: flex;
  flex-direction: column;
  gap: 6px;
  padding: 14px 16px;
  text-align: left;
  border: 1px solid var(--border-color);
  border-radius: var(--radius-md);
  background: var(--panel-bg);
  box-shadow: var(--shadow-card);
  cursor: pointer;
  transition: border-color 0.15s ease, box-shadow 0.15s ease, transform 0.15s ease;
}

.filter-card:hover {
  border-color: var(--border-strong);
}

.filter-card.is-active {
  border-color: var(--primary-color);
  box-shadow: 0 0 0 3px var(--primary-ring);
}

.filter-label {
  font-size: 12px;
  color: var(--text-muted);
}

.filter-count {
  font-size: 22px;
  font-weight: 650;
  letter-spacing: -0.02em;
  color: var(--text-main);
  font-variant-numeric: tabular-nums;
}

.filter-card.is-info .filter-count {
  color: var(--success-color);
}

.filter-card.is-warn .filter-count {
  color: var(--warning-color);
}

.filter-card.is-error .filter-count {
  color: var(--danger-color);
}

.filter-card.is-debug .filter-count {
  color: var(--text-secondary);
}

.log-panel {
  display: flex;
  flex-direction: column;
  border: 1px solid var(--border-color);
  border-radius: var(--radius-md);
  background: var(--panel-bg);
  box-shadow: var(--shadow-card);
  overflow: hidden;
}

.log-toolbar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
  flex-wrap: wrap;
  padding: 14px 18px;
  border-bottom: 1px solid var(--border-color);
  background: var(--panel-subtle);
}

.log-search {
  max-width: 280px;
}

.toolbar-right {
  display: flex;
  align-items: center;
  gap: 14px;
  flex-wrap: wrap;
}

.debug-toggle {
  display: inline-flex;
  align-items: center;
  gap: 8px;
  cursor: pointer;
}
.debug-toggle-label {
  font-size: 12px;
  color: var(--text-secondary);
  white-space: nowrap;
}
.toolbar-meta {
  font-size: 12px;
  color: var(--text-muted);
  font-variant-numeric: tabular-nums;
}

.debug-hint {
  padding: 10px 18px;
  font-size: 12px;
  color: var(--warning-color);
  background: var(--warning-soft);
  border-bottom: 1px solid var(--border-color);
}

.log-stream {
  display: flex;
  flex-direction: column;
  max-height: 560px;
  min-height: 220px;
  overflow-y: auto;
}

.log-date {
  position: sticky;
  top: 0;
  z-index: 1;
  padding: 8px 18px;
  font-size: 11.5px;
  font-weight: 600;
  letter-spacing: 0.02em;
  color: var(--text-faint);
  background: var(--panel-subtle);
  border-bottom: 1px solid var(--border-color);
}

.log-item {
  display: grid;
  grid-template-columns: 4px 96px 52px 120px minmax(0, 1fr);
  align-items: baseline;
  gap: 12px;
  padding: 9px 18px 9px 0;
  border-bottom: 1px solid rgba(230, 233, 239, 0.7);
  transition: background 0.12s ease;
}

.log-item:hover {
  background: var(--panel-subtle);
}

.log-marker {
  align-self: stretch;
  width: 4px;
  border-radius: 0 3px 3px 0;
  background: var(--border-strong);
}

.log-item.is-info .log-marker {
  background: var(--success-color);
}

.log-item.is-warn .log-marker {
  background: var(--warning-color);
}

.log-item.is-error .log-marker {
  background: var(--danger-color);
}

.log-item.is-debug .log-marker {
  background: var(--text-faint);
}

.log-time {
  font-family: var(--el-font-family-mono, ui-monospace, SFMono-Regular, Menlo, monospace);
  font-size: 12px;
  color: var(--text-muted);
  font-variant-numeric: tabular-nums;
}

.log-level {
  font-size: 12px;
  font-weight: 600;
}

.log-item.is-info .log-level {
  color: var(--success-color);
}

.log-item.is-warn .log-level {
  color: var(--warning-color);
}

.log-item.is-error .log-level {
  color: var(--danger-color);
}

.log-item.is-debug .log-level {
  color: var(--text-faint);
}

.log-target {
  font-size: 12px;
  color: var(--primary-color);
  background: var(--primary-soft);
  border-radius: 999px;
  padding: 1px 8px;
  justify-self: start;
  max-width: 120px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.log-text {
  font-size: 12.5px;
  line-height: 1.6;
  color: var(--text-secondary);
  word-break: break-word;
  display: -webkit-box;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
  overflow: hidden;
  cursor: pointer;
}

.log-text.is-open {
  display: block;
  -webkit-line-clamp: unset;
  color: var(--text-main);
}

.log-item.is-error .log-text {
  color: var(--danger-color);
}

.log-empty {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 8px;
  padding: 64px 20px;
  color: var(--text-muted);
}

.empty-icon {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 46px;
  height: 46px;
  border-radius: 50%;
  background: var(--panel-subtle);
  color: var(--text-faint);
  font-size: 22px;
}

.empty-title {
  font-size: 14px;
  font-weight: 600;
  color: var(--text-main);
}

.empty-hint {
  font-size: 12.5px;
  color: var(--text-muted);
  text-align: center;
}

@media (max-width: 960px) {
  .log-filters {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }

  .log-item {
    grid-template-columns: 4px 84px 52px minmax(0, 1fr);
  }

  .log-target {
    display: none;
  }
}
</style>
