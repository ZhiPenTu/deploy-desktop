<template>
  <el-dialog
    :model-value="modelValue"
    title="部署配置对比"
    width="94vw"
    top="4vh"
    append-to-body
    destroy-on-close
    class="config-diff-dialog"
    @update:model-value="emit('update:modelValue', $event)"
    @opened="focusInitialChange"
  >
    <div class="diff-toolbar">
      <div class="diff-summary" role="status" aria-live="polite">
        <el-tag :type="diff.changes.length ? 'warning' : 'info'" size="small">
          {{ diff.changes.length ? `${diff.changes.length} 处差异` : '配置内容一致' }}
        </el-tag>
        <span v-if="serviceName">当前服务：{{ serviceName }}</span>
        <span v-if="diff.redacted">敏感值已隐藏</span>
      </div>
      <div class="diff-navigation">
        <span v-if="diff.changes.length">{{ activeChange + 1 }} / {{ diff.changes.length }}</span>
        <el-button size="small" :disabled="!diff.changes.length || activeChange <= 0" @click="goToChange(activeChange - 1)">
          上一处
        </el-button>
        <el-button size="small" :disabled="!diff.changes.length || activeChange >= diff.changes.length - 1" @click="goToChange(activeChange + 1)">
          下一处
        </el-button>
      </div>
    </div>
    <div class="diff-panes">
      <section v-for="side in sides" :key="side.key" class="diff-pane" :aria-label="side.title">
        <header class="pane-header" :class="side.key">
          <div><strong>{{ side.title }}</strong><span>{{ side.subtitle }}</span></div>
          <code>{{ side.file || '尚无配置文件' }}</code>
        </header>
        <div
          :ref="(element) => paneElements[side.key] = element"
          class="pane-code"
          :aria-label="`${side.title}配置内容`"
          tabindex="0"
          @scroll="syncScroll"
        >
          <div class="code-content" :style="{ minWidth: `max(100%, ${side.width + 10}ch)` }">
            <div :style="{ height: `${startRow * ROW_HEIGHT}px` }" />
            <div
              v-for="(row, index) in visibleRows"
              :key="startRow + index"
              class="diff-line"
              :class="lineClass(row, side.key)"
              :data-row="startRow + index"
            >
              <span class="line-number">{{ row[`${side.key}Line`] }}</span>
              <span class="line-marker" aria-hidden="true">{{ row.changed && row[`${side.key}Line`] ? side.key === 'old' ? '-' : '+' : '' }}</span>
              <code>{{ row[`${side.key}Text`] || ' ' }}</code>
            </div>
            <div :style="{ height: `${Math.max(0, diff.rows.length - startRow - visibleRows.length) * ROW_HEIGHT}px` }" />
          </div>
          <div v-if="!diff.rows.length" class="empty-content">配置文件为空</div>
        </div>
        <footer class="pane-footer">
          <span>{{ side.key === 'old' ? oldLineCount : newLineCount }} 行</span>
          <span v-if="side.key === 'old' ? diff.oldNoNewline : diff.newNoNewline">文件末尾无换行</span>
        </footer>
      </section>
    </div>
    <template #footer>
      <span class="diff-legend"><span class="removed-key">红色：旧版删除或修改</span><span class="added-key">绿色：新版新增或修改</span></span>
      <el-button @click="emit('update:modelValue', false)">关闭</el-button>
    </template>
  </el-dialog>
</template>

<script setup>
import { computed, nextTick, reactive, ref } from 'vue';
import { buildConfigDiff, serviceDiffRow } from '../services/configDiff.js';

const props = defineProps({
  modelValue: { type: Boolean, default: false },
  review: { type: Object, required: true },
  serviceName: { type: String, default: '' },
  version: { type: String, default: '' }
});
const emit = defineEmits(['update:modelValue']);
const ROW_HEIGHT = 24;
const paneElements = reactive({ old: null, new: null });
const scrollTop = ref(0);
const viewportHeight = ref(700);
const activeChange = ref(0);
const diff = computed(() => buildConfigDiff(props.review.currentCompose, props.review.desiredCompose));
const startRow = computed(() => Math.max(0, Math.floor(scrollTop.value / ROW_HEIGHT) - 12));
const visibleRows = computed(() => diff.value.rows.slice(startRow.value, startRow.value + Math.ceil(viewportHeight.value / ROW_HEIGHT) + 24));
const oldLineCount = computed(() => diff.value.rows.filter((row) => row.oldLine !== null).length);
const newLineCount = computed(() => diff.value.rows.filter((row) => row.newLine !== null).length);
const sides = computed(() => [
  { key: 'old', title: '旧版', subtitle: '当前部署', file: props.review.targetComposeFile, width: diff.value.rows.reduce((width, row) => Math.max(width, row.oldText.length), 0) },
  { key: 'new', title: '新版', subtitle: props.version ? `所选制品 ${props.version}` : '所选制品', file: props.review.composeFile, width: diff.value.rows.reduce((width, row) => Math.max(width, row.newText.length), 0) }
]);

function lineClass(row, side) {
  return {
    'line-removed': row.changed && side === 'old' && row.oldLine !== null,
    'line-added': row.changed && side === 'new' && row.newLine !== null,
    'line-placeholder': row[`${side}Line`] === null
  };
}

function syncScroll(event) {
  const source = event.target;
  scrollTop.value = source.scrollTop;
  viewportHeight.value = source.clientHeight;
  for (const pane of Object.values(paneElements)) {
    if (pane && pane !== source && pane.scrollTop !== source.scrollTop) pane.scrollTop = source.scrollTop;
  }
}

function scrollToRow(row) {
  const position = Math.max(0, (row - 3) * ROW_HEIGHT);
  for (const pane of Object.values(paneElements)) if (pane) pane.scrollTop = position;
  scrollTop.value = paneElements.old?.scrollTop || 0;
}

function goToChange(index) {
  if (index < 0 || index >= diff.value.changes.length) return;
  scrollToRow(diff.value.changes[index]);
  activeChange.value = index;
}

async function focusInitialChange() {
  await nextTick();
  viewportHeight.value = paneElements.old?.clientHeight || 700;
  const row = Math.max(0, serviceDiffRow(diff.value.rows, props.serviceName));
  activeChange.value = Math.max(0, diff.value.changes.findLastIndex((change) => change <= row));
  scrollToRow(row);
}

</script>

<style scoped>
.diff-toolbar, .diff-summary, .diff-navigation, .diff-legend { display: flex; align-items: center; gap: 12px; }
.diff-toolbar { flex: none; justify-content: space-between; flex-wrap: wrap; margin-bottom: 16px; }
.diff-summary, .diff-navigation, .diff-legend { color: var(--text-muted); font-size: 12px; flex-wrap: wrap; }
.diff-navigation .el-button + .el-button { margin-left: 0; }
.diff-panes { flex: 1; min-height: 0; display: grid; grid-template-columns: minmax(0, 1fr) minmax(0, 1fr); border: 1px solid var(--border-color); border-radius: var(--radius-sm); overflow: hidden; }
.diff-pane { min-width: 0; min-height: 0; display: flex; flex-direction: column; }
.diff-pane + .diff-pane { border-left: 1px solid var(--border-color); }
.pane-header { flex: none; padding: 14px 16px; height: 87px; box-sizing: border-box; border-bottom: 1px solid var(--border-color); background: var(--panel-subtle); }
.pane-header > div { display: flex; align-items: baseline; gap: 10px; margin-bottom: 8px; }
.pane-header strong { font-size: 15px; color: var(--text-main); }
.pane-header span, .pane-header > code { font-size: 12px; color: var(--text-muted); }
.pane-header > code { display: block; white-space: nowrap; overflow: auto; }
.pane-code { flex: 1; min-height: 0; position: relative; overflow: auto; background: #fff; }
.pane-code:focus-visible { outline: 2px solid var(--primary-color); outline-offset: -2px; }
.code-content { width: max-content; min-width: 100%; padding-bottom: 0; font-family: 'SFMono-Regular', Menlo, Consolas, monospace; font-size: 12px; line-height: 24px; }
.diff-line { display: flex; height: 24px; box-sizing: border-box; padding-right: 20px; color: var(--text-main); }
.line-number { flex: none; width: 50px; padding-right: 12px; text-align: right; color: var(--text-faint); background: #f6f8fa; user-select: none; }
.line-marker { flex: none; width: 22px; text-align: center; user-select: none; }
.diff-line code { white-space: pre; font: inherit; }
.line-removed { background: #ffebe9; color: #82071e; }
.line-removed .line-number { background: #ffd7d5; color: #a40e26; }
.line-added { background: #dafbe1; color: #116329; }
.line-added .line-number { background: #b7efc5; color: #116329; }
.line-placeholder { background: repeating-linear-gradient(135deg, #f6f8fa, #f6f8fa 4px, #eef1f5 4px, #eef1f5 5px); }
.line-placeholder .line-number { background: transparent; }
.pane-footer { flex: none; display: flex; justify-content: space-between; gap: 8px; padding: 9px 14px; border-top: 1px solid var(--border-color); font-size: 11px; color: var(--text-muted); background: var(--panel-subtle); }
.empty-content { padding: 24px; color: var(--text-muted); }
.diff-legend { display: inline-flex; margin-right: 20px; vertical-align: middle; }
.removed-key { color: #a40e26; }
.added-key { color: #116329; }
@media (max-width: 680px) {
  .pane-header { padding: 12px 8px; }
  .pane-header > div { gap: 5px; flex-wrap: wrap; }
  .line-number { width: 34px; padding-right: 6px; }
  .line-marker { width: 16px; }
  .diff-legend { margin: 0 0 12px; width: 100%; }
}
</style>

<style>
.config-diff-dialog {
  height: min(92vh, 1000px);
}
.config-diff-dialog .el-dialog__body {
  flex: 1;
  display: flex;
  flex-direction: column;
  overflow: hidden;
}
</style>
