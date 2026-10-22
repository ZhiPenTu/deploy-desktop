<template>
  <div class="step-card" v-loading="checkingReview">
    <div class="section-heading">
      <div>
        <h3>核对服务清单与镜像</h3>
        <p class="desc">对比所选制品与 {{ targetLabel }} 上的现有部署，确认所需镜像是否已具备。</p>
      </div>
      <el-button :loading="checkingReview" :disabled="busy" @click="emit('recheck')"> 重新比对 </el-button>
    </div>
    <el-alert v-if="reviewError" :title="reviewError" type="error" :closable="false" show-icon />
    <template v-if="review">
      <div class="status-row">
        <el-tag :type="review.hasDeployment ? 'primary' : 'info'">
          {{ review.hasDeployment ? '检测到现有部署' : '未检测到现有部署' }}
        </el-tag>
        <span>制品版本 {{ version || '未标记' }}</span>
        <span>{{ review.services?.filter((s) => s.change !== 'removed').length }} 个服务</span>
      </div>
      <div class="section-heading compose-review-heading">
        <p class="field-hint">部署清单：{{ review.composeFile }}</p>
        <el-button :disabled="checkingReview" @click="openConfigDiff()">查看配置对比</el-button>
      </div>
      <el-alert
        v-if="review.hasDeployment && review.composeChanged"
        title="配置有变化，可继续选择更新内容；仅勾选“部署配置”时才会替换配置文件。"
        type="info"
        :closable="false"
        show-icon
      />
      <el-tabs :model-value="reviewTab" @update:model-value="(value) => emit('update:review-tab', value)">
        <el-tab-pane label="服务清单比对" name="services">
          <el-table :data="review.services || []" size="small" empty-text="未读取到服务清单">
            <el-table-column prop="name" label="服务" min-width="140" />
            <el-table-column label="制品镜像" min-width="240">
              <template #default="{ row }">
                <code>{{ row.image || '未声明' }}</code>
              </template>
            </el-table-column>
            <el-table-column label="当前镜像" min-width="220">
              <template #default="{ row }">
                <code>{{ row.currentImage || '未部署' }}</code>
              </template>
            </el-table-column>
            <el-table-column label="变化" width="110">
              <template #default="{ row }">
                <el-button
                  v-if="row.change !== 'unchanged'"
                  link
                  type="warning"
                  :aria-label="`查看 ${row.name} 的配置差异`"
                  @click="openConfigDiff(row.name)"
                >
                  {{ changeLabel(row.change) }}
                </el-button>
                <el-tag v-else size="small" type="info">
                  {{ changeLabel(row.change) }}
                </el-tag>
              </template>
            </el-table-column>
            <el-table-column label="容器状态" width="110">
              <template #default="{ row }">{{ stateLabel(row.containerState) }}</template>
            </el-table-column>
          </el-table>
        </el-tab-pane>
        <el-tab-pane label="镜像核对" name="images">
          <el-table :data="review.images || []" size="small" empty-text="清单中没有镜像">
            <el-table-column label="所需镜像" min-width="280">
              <template #default="{ row }">
                <code>{{ row.imageRef }}</code>
              </template>
            </el-table-column>
            <el-table-column label="服务" min-width="150">
              <template #default="{ row }">{{ row.services?.join('、') }}</template>
            </el-table-column>
            <el-table-column label="目标主机" width="115">
              <template #default="{ row }">
                <el-tag size="small" :type="row.status === 'ready' ? 'success' : 'warning'">
                  {{ row.status === 'ready' ? '已具备' : row.status === 'unconfirmed' ? '未确认' : '缺失' }}
                </el-tag>
              </template>
            </el-table-column>
          </el-table>
        </el-tab-pane>
      </el-tabs>
      <ConfigDiffDialog
        v-if="configDiffOpen"
        v-model="configDiffOpen"
        :review="review"
        :service-name="configDiffService"
        :version="version"
      />
      <ul v-if="review.issues?.length" class="issue-list">
        <li v-for="issue in review.issues" :key="issue">
          {{ safeText(issue) }}
        </li>
      </ul>
      <div v-if="review.missingImages?.length" class="image-preparation">
        <h4>准备缺失镜像</h4>
        <p class="field-hint">
          选择本机的离线介质目录（含 images/），将其中的镜像包导入{{
            isRemote ? '目标主机' : '本机'
          }}。导入后会重新比对。
        </p>
        <div class="input-row wrap">
          <el-input
            :model-value="imageMediaDir"
            :disabled="busy"
            :placeholder="bundleDir || '本机离线介质目录'"
            aria-label="镜像介质目录"
            @update:model-value="(value) => emit('update:image-media-dir', value)"
          />
          <el-button :disabled="busy" @click="emit('pick-image-directory')"> 选择目录 </el-button>
          <el-button
            type="primary"
            :loading="importingImages"
            :disabled="busy"
            @click="emit('prepare-images')"
          >
            导入镜像并比对
          </el-button>
        </div>
      </div>
    </template>
    <div class="step-footer">
      <el-button :disabled="busy" @click="emit('previous')"> 上一步 </el-button>
      <el-button type="primary" :disabled="busy || !review?.ready" @click="emit('next')">
        下一步：选择更新
        <el-icon class="el-icon--right"><ArrowRight /></el-icon>
      </el-button>
    </div>
  </div>
</template>

<script setup>
import { ref, watch } from 'vue';
import { ArrowRight } from '@element-plus/icons-vue';
import ConfigDiffDialog from './ConfigDiffDialog.vue';
import { changeLabel, stateLabel } from '../domain/review.js';
import { safeText } from '../domain/errors.js';

const props = defineProps({
  targetLabel: { type: String, default: '' },
  checkingReview: { type: Boolean, default: false },
  busy: { type: Boolean, default: false },
  reviewError: { type: String, default: '' },
  review: { type: Object, default: null },
  version: { type: String, default: '' },
  bundleDir: { type: String, default: '' },
  isRemote: { type: Boolean, default: false },
  imageMediaDir: { type: String, default: '' },
  importingImages: { type: Boolean, default: false },
  reviewTab: { type: String, default: 'services' }
});

const emit = defineEmits([
  'recheck',
  'update:review-tab',
  'update:image-media-dir',
  'pick-image-directory',
  'prepare-images',
  'previous',
  'next'
]);

const configDiffOpen = ref(false);
const configDiffService = ref('');

function openConfigDiff(serviceName = '') {
  configDiffService.value = serviceName;
  configDiffOpen.value = true;
}

watch(
  () => props.review,
  () => {
    configDiffOpen.value = false;
    configDiffService.value = '';
  }
);
</script>

<style scoped>
.step-card {
  padding: 26px 28px;
  background: var(--panel-bg);
  border: 1px solid var(--border-color);
  border-radius: var(--radius-md);
  box-shadow: var(--shadow-card);
}
h3 {
  font-size: 16px;
  font-weight: 650;
  letter-spacing: -0.01em;
  margin-bottom: 6px;
  color: var(--text-main);
}
.desc {
  color: var(--text-muted);
  font-size: 12.5px;
  line-height: 1.6;
  margin-bottom: 22px;
}
.section-heading {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 12px;
}
.compose-review-heading {
  align-items: center;
  margin-bottom: 12px;
}
.compose-review-heading .field-hint {
  flex: 1;
  margin-top: 0;
}
.status-row {
  display: flex;
  align-items: center;
  gap: 12px;
  flex-wrap: wrap;
  margin: 16px 0;
  color: var(--text-secondary);
  font-size: 12.5px;
}
.issue-list {
  padding: 14px 18px 14px 34px;
  margin: 16px 0;
  color: var(--warning-color);
  background: var(--warning-soft);
  border: 1px solid rgba(245, 158, 11, 0.2);
  border-radius: var(--radius-sm);
  font-size: 12px;
  line-height: 1.8;
}
.input-row {
  display: flex;
  align-items: center;
  gap: 12px;
  width: 100%;
}
.input-row .el-input {
  flex: 1;
  min-width: 150px;
}
.wrap {
  flex-wrap: wrap;
}
.field-hint {
  width: 100%;
  font-size: 12px;
  color: var(--text-muted);
  line-height: 1.65;
  margin-top: 6px;
  overflow-wrap: anywhere;
}
.image-preparation {
  padding: 18px 20px;
  background: var(--panel-subtle);
  border: 1px solid var(--border-color);
  border-radius: var(--radius-sm);
  margin-top: 20px;
}
.image-preparation .input-row {
  margin-top: 14px;
}
code {
  font-size: 12px;
  overflow-wrap: anywhere;
}
.step-footer {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
  padding-top: 22px;
  margin-top: 26px;
  border-top: 1px solid var(--border-color);
}
@media (max-width: 760px) {
  .step-card {
    padding: 20px;
  }
  .step-footer,
  .section-heading {
    flex-wrap: wrap;
  }
}
</style>
