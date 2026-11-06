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
    <el-alert v-if="imageResult" :title="imageResult" type="success" :closable="false" show-icon />
    <el-collapse v-if="imageDiagnostics" class="image-diagnostics">
      <el-collapse-item title="查看镜像处理详情" name="images">
        <pre>{{ imageDiagnostics }}</pre>
      </el-collapse-item>
    </el-collapse>
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
        <el-button :disabled="busy" @click="openConfigDiff()">查看配置对比</el-button>
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
        <el-radio-group
          :model-value="imageSource"
          :disabled="busy"
          aria-label="镜像来源"
          @update:model-value="(value) => emit('update:image-source', value)"
        >
          <el-radio-button value="pull">镜像仓库</el-radio-button>
          <el-radio-button v-if="isRemote" value="docker">本机 Docker</el-radio-button>
          <el-radio-button value="offline">离线镜像包</el-radio-button>
        </el-radio-group>
        <p v-if="imageSource === 'pull'" class="field-hint">
          按清单中的镜像地址拉取 {{ review.missingImages.length }} 个缺失镜像到{{ isRemote ? '目标主机' : '本机' }} Docker，拉取完成后自动重新比对。
        </p>
        <p v-else-if="isRemote && imageSource === 'docker'" class="field-hint">
          从本机 Docker 导出 {{ review.missingImages.length }} 个缺失镜像，传输到 {{ targetLabel }} 后解压并导入。请先在本机准备与目标主机架构一致的镜像。
        </p>
        <p v-else class="field-hint">
          选择本机离线镜像包目录{{ isRemote ? '，仅传输缺失镜像对应的镜像包，随后在目标主机解压并导入。' : '（含 images/），将其中的镜像包导入本机。' }}导入后会重新比对。
        </p>
        <div v-if="imageSource === 'pull'" class="registry-pull-fields">
          <el-select
            :model-value="imageRegistrySourceId"
            :disabled="busy"
            clearable
            :placeholder="loginStateLabel"
            aria-label="登录镜像源"
            @update:model-value="(value) => emit('update:image-registry-source-id', value || '')"
          >
            <el-option :label="loginStateLabel" value="" />
            <el-option
              v-for="source in imageRegistryOptions"
              :key="source.id"
              :label="`${source.name}（${source.address}）`"
              :value="source.id"
            />
          </el-select>
          <el-input
            v-if="imageRegistryNeedsPassword"
            :model-value="imageRegistryPassword"
            :disabled="busy"
            type="password"
            show-password
            autocomplete="new-password"
            placeholder="本次使用的密码或令牌"
            aria-label="镜像源密码"
            @update:model-value="(value) => emit('update:image-registry-password', value)"
          />
          <p v-if="imageRegistryLoadError" class="field-hint">
            {{ imageRegistryLoadError }}；仍可按清单地址尝试公开拉取。
          </p>
          <p v-else class="field-hint">
            {{ registryHint }}
          </p>
        </div>
        <div class="input-row wrap">
          <template v-if="imageSource === 'offline'">
            <el-input
              :model-value="imageMediaDir"
              :disabled="busy"
              :placeholder="bundleDir || '本机离线介质目录'"
              aria-label="镜像介质目录"
              @update:model-value="(value) => emit('update:image-media-dir', value)"
            />
            <el-button :disabled="busy" @click="emit('pick-image-directory')"> 选择目录 </el-button>
          </template>
          <el-button
            type="primary"
            :loading="importingImages"
            :disabled="busy"
            @click="emit('prepare-images')"
          >
            {{ prepareButtonText }}
          </el-button>
        </div>
      </div>
      <div v-if="importingImages && imagePhase" class="image-progress" role="status">
        <p>{{ imagePhase }}</p>
        <el-progress v-if="imageProgress" :percentage="imageProgress.percent" />
        <p v-if="imageProgress?.message" class="field-hint">{{ imageProgress.message }}</p>
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
import { computed, ref, watch } from 'vue';
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
  imageSource: { type: String, default: 'pull' },
  imageRegistrySourceId: { type: String, default: '' },
  imageRegistryPassword: { type: String, default: '' },
  imageRegistryOptions: { type: Array, default: () => [] },
  imageRegistryNeedsPassword: { type: Boolean, default: false },
  imageRegistryMixed: { type: Boolean, default: false },
  imageRegistryLoadError: { type: String, default: '' },
  imagePhase: { type: String, default: '' },
  imageProgress: { type: Object, default: null },
  imageDiagnostics: { type: String, default: '' },
  imageResult: { type: String, default: '' },
  importingImages: { type: Boolean, default: false },
  reviewTab: { type: String, default: 'services' }
});

const emit = defineEmits([
  'recheck',
  'update:review-tab',
  'update:image-media-dir',
  'update:image-source',
  'update:image-registry-source-id',
  'update:image-registry-password',
  'pick-image-directory',
  'prepare-images',
  'previous',
  'next'
]);

const configDiffOpen = ref(false);
const configDiffService = ref('');
const loginStateLabel = computed(() =>
  props.isRemote ? '使用目标机已有登录状态' : '使用本机已有登录状态'
);
const registryHint = computed(() => {
  if (props.imageRegistryMixed) {
    return `缺失镜像来自多个镜像仓库，当前只能${loginStateLabel.value}或公开访问；私有镜像请先在各仓库完成登录，或改用离线镜像包。`;
  }
  if (props.imageRegistryOptions.length) {
    return '仅列出与缺失镜像仓库地址一致的镜像源；公开镜像可留空。凭据只用于本次拉取。';
  }
  return '未找到与缺失镜像仓库地址一致的镜像源。公开镜像可直接拉取；私有镜像请先在“设置 > 镜像设置”中配置对应镜像源。';
});
const prepareButtonText = computed(() => {
  if (props.imageSource === 'pull') return '拉取缺失镜像';
  return props.isRemote ? '传输并导入缺失镜像' : '导入镜像并比对';
});

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
.image-preparation .el-radio-group {
  margin-top: 14px;
}
.registry-pull-fields {
  display: flex;
  flex-wrap: wrap;
  gap: 12px;
  margin-top: 14px;
}
.registry-pull-fields .el-select,
.registry-pull-fields .el-input {
  flex: 1;
  min-width: 220px;
}
.registry-pull-fields .field-hint {
  width: 100%;
  margin-top: 0;
}
.image-progress {
  margin-top: 16px;
  font-size: 13px;
}
.image-diagnostics pre {
  white-space: pre-wrap;
  overflow-wrap: anywhere;
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
