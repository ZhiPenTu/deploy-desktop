<template>
  <div class="usb-write">
    <div class="selection-row card-surface">
      <div class="selection-copy">
        <strong>选择要写入的内容</strong>
        <span class="selection-hint">
          已选择 {{ bundleSelection.length }} 个交付包、{{ packageSelection.length }} 个镜像包
        </span>
      </div>
      <div class="selection-actions">
        <el-button :loading="bundlesLoading || packagesLoading" :disabled="busy" @click="refreshSources">
          <el-icon><Refresh /></el-icon>
          <span>刷新本机内容</span>
        </el-button>
        <el-button
          type="primary"
          :disabled="!canWrite"
          :loading="exporting"
          @click="writeSelected"
        >
          写入 USB
        </el-button>
      </div>
    </div>

    <el-alert
      v-if="scan && !scan.writable"
      title="所选目录不可写入"
      description="请确认 U 盘或移动硬盘已连接，且没有设置为只读。"
      type="warning"
      :closable="false"
      show-icon
    />

    <div v-if="progress" class="write-progress card-surface">
      <el-progress :percentage="progress.percent" :stroke-width="8" :show-text="false" />
      <span class="progress-text">
        {{ progress.message }}<template v-if="progress.label">：{{ progress.label }}</template>
        <template v-if="progress.total">（{{ progress.current }} / {{ progress.total }}）</template>
      </span>
    </div>

    <el-alert v-if="error" :title="error" type="error" :closable="false" show-icon />

    <el-alert
      v-if="failedItems.length"
      type="warning"
      :closable="false"
      show-icon
      title="部分内容未能写入"
    >
      <ul class="failure-list">
        <li v-for="(item, index) in failedItems" :key="`${item.kind}-${item.label}-${index}`">
          {{ item.label }}：{{ item.message }}
        </li>
      </ul>
    </el-alert>

    <div class="card-surface usb-card">
      <div class="card-title-bar">
        <h3>本机交付包</h3>
        <span class="hint">已下载或已解压的交付包，共 {{ localBundles.length }} 个</span>
      </div>

      <el-alert v-if="bundlesError" :title="bundlesError" type="warning" :closable="false" show-icon />

      <el-table
        v-if="localBundles.length"
        ref="bundleTableRef"
        :data="localBundles"
        row-key="dir"
        size="small"
        class="usb-table"
        @selection-change="(rows) => (bundleSelection = rows)"
      >
        <el-table-column type="selection" width="44" />
        <el-table-column label="名称" min-width="220">
          <template #default="{ row }">
            <div class="name-cell">
              <span>{{ row.label }}</span>
              <span v-if="row.product && row.product !== row.label" class="muted-text">
                {{ row.product }}
              </span>
            </div>
          </template>
        </el-table-column>
        <el-table-column prop="version" label="版本" min-width="150" />
        <el-table-column label="提交" width="110">
          <template #default="{ row }">
            <span v-if="row.commit" class="mono-text">{{ row.commit }}</span>
            <span v-else class="muted-text">未记录</span>
          </template>
        </el-table-column>
        <el-table-column label="来源" min-width="140">
          <template #default="{ row }">
            <span :class="{ 'muted-text': !row.sourceName }">
              {{ row.sourceName || '本机选择' }}
            </span>
          </template>
        </el-table-column>
        <el-table-column label="大小" width="110">
          <template #default="{ row }">{{ formatSize(row.sizeBytes) }}</template>
        </el-table-column>
      </el-table>

      <div v-else-if="!bundlesLoading && !bundlesError" class="empty-state">
        <span class="empty-title">暂无可写入的交付包</span>
        <span class="empty-hint">在「制品中心」下载版本，或在部署向导中选择本机交付包后，这里会显示可写入的内容。</span>
      </div>
    </div>

    <div class="card-surface usb-card">
      <div class="card-title-bar">
        <h3>本机离线镜像包</h3>
        <span class="hint">从镜像列表导出的离线包，共 {{ packages.length }} 个</span>
      </div>

      <el-alert v-if="packagesError" :title="packagesError" type="warning" :closable="false" show-icon />

      <el-table
        v-if="packages.length"
        ref="packageTableRef"
        :data="packages"
        row-key="name"
        size="small"
        class="usb-table"
        @selection-change="(rows) => (packageSelection = rows)"
      >
        <el-table-column type="selection" width="44" />
        <el-table-column prop="name" label="离线包文件" min-width="260" />
        <el-table-column label="对应镜像" min-width="280">
          <template #default="{ row }">
            <template v-if="row.imageRefs && row.imageRefs.length">
              <code v-for="image in row.imageRefs" :key="image" class="ref-line">{{ image }}</code>
            </template>
            <span v-else class="muted-text">未记录对应镜像</span>
          </template>
        </el-table-column>
        <el-table-column label="大小" width="110">
          <template #default="{ row }">{{ formatSize(row.sizeBytes) }}</template>
        </el-table-column>
      </el-table>

      <div v-else-if="!packagesLoading && !packagesError" class="empty-state">
        <span class="empty-title">暂无可写入的离线镜像包</span>
        <span class="empty-hint">先在「镜像与离线包」中选择镜像并导出离线包，再回到这里写入 USB。</span>
        <el-button size="small" @click="emit('go-images')">前往镜像与离线包</el-button>
      </div>
    </div>
  </div>
</template>

<script setup>
import { computed, ref, watch } from 'vue';
import { ElMessage, ElMessageBox } from 'element-plus';
import { Refresh } from '@element-plus/icons-vue';
import { exportUsbMedia } from '../../../api/tauri.js';
import { errorText } from '../../../shared/errors.js';
import { formatSize } from '../../../shared/format.js';

const props = defineProps({
  mediaDir: { type: String, default: '' },
  scan: { type: Object, default: null },
  localBundles: { type: Array, default: () => [] },
  bundlesLoading: { type: Boolean, default: false },
  bundlesError: { type: String, default: '' },
  offlineScan: { type: Object, default: null },
  packagesLoading: { type: Boolean, default: false },
  packagesError: { type: String, default: '' },
  desktopAvailable: { type: Boolean, default: false }
});

const emit = defineEmits(['refresh-bundles', 'refresh-packages', 'exported', 'go-images', 'busy-change']);

const bundleTableRef = ref(null);
const packageTableRef = ref(null);
const bundleSelection = ref([]);
const packageSelection = ref([]);
const exporting = ref(false);
const error = ref('');
const progress = ref(null);
const results = ref([]);

const packages = computed(() => props.offlineScan?.files || []);
const failedItems = computed(() => results.value.filter((item) => !item.success));
const busy = computed(() => exporting.value);
const canWrite = computed(
  () =>
    props.desktopAvailable &&
    !busy.value &&
    Boolean(props.scan?.writable) &&
    (bundleSelection.value.length > 0 || packageSelection.value.length > 0)
);

watch(busy, (value) => emit('busy-change', value), { flush: 'sync' });

function refreshSources() {
  emit('refresh-bundles');
  emit('refresh-packages');
}

function joinPath(directory, name) {
  const base = String(directory || '').replace(/[\\/]+$/, '');
  return base ? `${base}/${name}` : name;
}

async function writeSelected() {
  if (!canWrite.value) return;
  if (props.scan?.entries?.length) {
    try {
      await ElMessageBox.confirm(
        `将把选中的内容写入 ${props.scan.rootDir}。介质中的同名文件会被替换，其他内容保持不变。`,
        '写入 USB',
        {
          type: 'warning',
          confirmButtonText: '写入',
          cancelButtonText: '取消'
        }
      );
    } catch {
      return;
    }
  }
  exporting.value = true;
  error.value = '';
  results.value = [];
  progress.value = { percent: 0, message: '正在准备写入', label: '', current: 0, total: 0 };
  try {
    const imageFiles = packageSelection.value.map((row) => ({
      path: joinPath(props.offlineScan?.offlineDir, row.name),
      images: row.imageRefs || []
    }));
    const result = await exportUsbMedia(
      props.mediaDir,
      bundleSelection.value.map((row) => row.dir),
      imageFiles,
      (payload) => {
        progress.value = {
          percent: Number(payload?.percent) || 0,
          message: payload?.message || '正在写入',
          label: payload?.label || '',
          current: Number(payload?.current) || 0,
          total: Number(payload?.total) || 0
        };
      }
    );
    results.value = result.items || [];
    emit('exported', result);
    bundleTableRef.value?.clearSelection();
    packageTableRef.value?.clearSelection();
    bundleSelection.value = [];
    packageSelection.value = [];
    if (result.failed) {
      ElMessage.warning(`已写入 ${result.written} 项，${result.failed} 项未能完成`);
    } else {
      ElMessage.success(`已写入 ${result.written} 项到 USB 介质`);
    }
  } catch (writeError) {
    error.value = errorText(writeError);
    ElMessage.error(error.value);
  } finally {
    exporting.value = false;
    progress.value = null;
  }
}
</script>

<style scoped>
.usb-write {
  display: flex;
  flex-direction: column;
  gap: 16px;
}

.selection-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
  padding: 14px 18px;
  flex-wrap: wrap;
}

.selection-copy {
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.selection-copy strong {
  font-size: 14px;
  color: var(--text-main);
}

.selection-hint {
  font-size: 12px;
  color: var(--text-muted);
}

.selection-actions {
  display: flex;
  align-items: center;
  gap: 8px;
}

.selection-actions :deep(.el-button + .el-button) {
  margin-left: 0;
}

.usb-card {
  padding: 18px 20px;
}

.usb-table {
  margin-top: 4px;
}

.name-cell {
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.name-cell .muted-text {
  font-size: 11.5px;
}

.ref-line {
  display: block;
  margin-bottom: 2px;
}

.ref-line:last-child {
  margin-bottom: 0;
}

.write-progress {
  display: flex;
  flex-direction: column;
  gap: 8px;
  padding: 14px 18px;
}

.progress-text {
  font-size: 12px;
  color: var(--text-muted);
}

.failure-list {
  margin: 6px 0 0;
  padding-left: 18px;
  font-size: 12px;
  line-height: 1.7;
}

.empty-state {
  margin-top: 8px;
}

@media (max-width: 760px) {
  .usb-card {
    padding: 16px;
  }

  .selection-row {
    align-items: flex-start;
  }
}
</style>
