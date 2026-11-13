<template>
  <div class="step-card" v-loading="checkingReview">
    <div class="section-heading">
      <div>
        <h3>核对目标环境</h3>
        <p class="desc">
          检查目标机器是否已有部署、所需镜像和制品文件；缺什么就在这里补齐，再进入部署。
        </p>
      </div>
      <el-button :loading="checkingReview" :disabled="busy" @click="emit('recheck')">
        重新检查
      </el-button>
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
        <span v-if="targetLabel">目标 {{ targetLabel }}</span>
        <el-tag :type="review.hasDeployment ? 'primary' : 'info'">
          {{ review.hasDeployment ? '检测到现有部署' : '未检测到现有部署' }}
        </el-tag>
        <span>制品版本 {{ version || '未标记' }}</span>
        <span>{{ review.services?.filter((service) => service.change !== 'removed').length }} 个服务</span>
        <span v-if="review.targetPlatform">目标架构 {{ review.targetPlatform }}</span>
        <span v-if="totalImageCount">镜像 {{ readyImageCount }} / {{ totalImageCount }} 已就绪</span>
        <span v-if="artifactProblemCount">制品 {{ artifactProblemCount }} 项待处理</span>
      </div>
      <div class="section-heading compose-review-heading">
        <p class="field-hint">部署清单：{{ review.composeFile }}</p>
        <el-button :disabled="busy" @click="openConfigDiff()">查看配置对比</el-button>
      </div>
      <el-alert
        v-if="review.hasDeployment && review.composeChanged"
        title="配置有变化。完整版本部署会替换全部部署配置，并在失败时恢复上一版本。"
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
        <el-tab-pane label="制品文件" name="artifacts">
          <p class="field-hint tab-hint">{{ artifactHint }}</p>
          <el-alert
            v-for="issue in review.artifactIssues || []"
            :key="issue"
            :title="issue"
            type="warning"
            :closable="false"
            show-icon
            class="artifact-alert"
          />
          <el-table :data="review.artifacts || []" size="small" empty-text="交付包未声明制品文件">
            <el-table-column label="制品" min-width="150">
              <template #default="{ row }">
                {{ row.label || row.key }}
                <span class="muted">（{{ row.kind === 'dir' ? '目录' : '文件' }}）</span>
              </template>
            </el-table-column>
            <el-table-column label="目标位置" min-width="280">
              <template #default="{ row }">
                <code>{{ row.target }}</code>
              </template>
            </el-table-column>
            <el-table-column label="状态" width="100">
              <template #default="{ row }">
                <el-tag size="small" :type="artifactStatusTone(row.status, review.hasDeployment)">
                  {{ artifactStatusLabel(row.status, review.hasDeployment) }}
                </el-tag>
              </template>
            </el-table-column>
            <el-table-column label="大小或文件数" width="170">
              <template #default="{ row }">
                <template v-if="row.kind === 'dir'">
                  {{ row.actualFiles || 0 }} / {{ row.expectedFiles || 0 }} 个文件
                </template>
                <template v-else>
                  {{ formatSize(row.actualBytes) }} / {{ formatSize(row.expectedBytes) }}
                </template>
              </template>
            </el-table-column>
          </el-table>
          <div v-if="artifactFixableCount || artifactBlockedCount" class="input-row wrap artifact-actions">
            <el-button
              v-if="artifactFixableCount"
              type="primary"
              :disabled="busy || !review.ready"
              @click="emit('next')"
            >
              下一步：部署完整版本
            </el-button>
            <el-button v-if="artifactBlockedCount" :disabled="busy" @click="emit('recheck')">
              重新检查
            </el-button>
            <p class="field-hint">
              {{
                artifactBlockedCount
                  ? '先处理无法核对的路径并重新检查，才能继续。'
                  : '下一步会写入交付包声明的全部受管文件。'
              }}
            </p>
          </div>
        </el-tab-pane>
        <el-tab-pane label="镜像与修复" name="images">
          <div class="image-summary">
            <span>待处理 {{ imagePendingCount }} 个</span>
            <span v-if="imageCounts.missing">缺失 {{ imageCounts.missing }} 个</span>
            <span v-if="imageCounts.mismatch">架构不匹配 {{ imageCounts.mismatch }} 个</span>
            <span v-if="imageCounts.failed">检查失败 {{ imageCounts.failed }} 个</span>
            <span v-if="imageCounts.invalid">地址无效 {{ imageCounts.invalid }} 个</span>
          </div>
          <el-alert
            v-if="!review.dockerRunning"
            title="目标 Docker 未运行，当前无法检查或拉取镜像。请返回环境检测启动 Docker 后重新检查。"
            type="warning"
            :closable="false"
            show-icon
          />
          <el-table :data="review.images || []" size="small" empty-text="清单中没有镜像">
            <el-table-column label="所需镜像" min-width="260">
              <template #default="{ row }">
                <code>{{ row.imageRef }}</code>
              </template>
            </el-table-column>
            <el-table-column label="服务" min-width="130">
              <template #default="{ row }">{{ row.services?.join('、') }}</template>
            </el-table-column>
            <el-table-column label="状态" width="115">
              <template #default="{ row }">
                <el-tag size="small" :type="imageStatusTone(row.status)">
                  {{ imageStatusLabel(row.status) }}
                </el-tag>
              </template>
            </el-table-column>
            <el-table-column label="原因与处理" min-width="280">
              <template #default="{ row }">{{ imageStatusDetail(row) }}</template>
            </el-table-column>
          </el-table>
          <el-alert
            v-if="imageBlockedCount"
            :title="`有 ${imageBlockedCount} 个镜像不能直接批量拉取，请按原因重新检查、更换交付包或使用离线镜像包。`"
            type="warning"
            :closable="false"
            show-icon
          />
          <div v-if="imagePendingCount" class="image-preparation">
            <h4>准备待处理镜像</h4>
            <p class="field-hint">{{ imageSummary }}</p>
            <el-radio-group
              :model-value="imageSource"
              :disabled="busy"
              aria-label="镜像来源"
              @update:model-value="(value) => emit('update:image-source', value)"
            >
              <el-radio-button value="pull">从镜像仓库拉取</el-radio-button>
              <el-radio-button v-if="isRemote" value="relay">本机拉取后传入</el-radio-button>
              <el-radio-button value="offline">离线镜像包</el-radio-button>
            </el-radio-group>
            <p v-if="imageSource === 'pull'" class="field-hint">
              在{{ isRemote ? '目标主机' : '本机' }}拉取镜像。公开镜像无需登录；私有镜像请选择对应镜像源。
            </p>
            <p v-else-if="imageSource === 'relay'" class="field-hint">
              先在本机按目标架构拉取，再通过 SSH 传入目标机导入。适用于目标机无法访问镜像仓库的局域网环境。
            </p>
            <p v-else class="field-hint">
              选择本机离线镜像包目录{{ isRemote ? '，仅传输待处理镜像对应的镜像包，随后在目标主机解压并导入。' : '（含 images/），将其中的镜像包导入本机。' }}
            </p>
            <el-alert
              v-if="digestTransferNotice"
              :title="digestTransferNotice"
              type="warning"
              :closable="false"
              show-icon
              class="digest-alert"
            />
            <div v-if="imageSource !== 'offline' && imagePullGroups.length" class="registry-groups">
              <div v-for="group in imagePullGroups" :key="group.registry" class="registry-group">
                <div class="registry-group-title">
                  <span>{{ group.label }}</span>
                  <span class="field-hint">{{ group.count }} 个镜像</span>
                </div>
                <el-select
                  :model-value="group.sourceId"
                  :disabled="busy"
                  placeholder="使用已有登录状态"
                  aria-label="登录镜像源"
                  @update:model-value="(value) => emit('update:image-pull-source', {
                    registry: group.registry,
                    sourceId: value || ''
                  })"
                >
                  <el-option label="使用已有登录状态" value="" />
                  <el-option
                    v-for="source in group.sources || []"
                    :key="source.id"
                    :label="`${source.name}（${source.address}）`"
                    :value="source.id"
                  />
                </el-select>
                <el-input
                  v-if="group.hasUsername"
                  :model-value="group.credential"
                  :disabled="busy"
                  type="password"
                  show-password
                  autocomplete="new-password"
                  :placeholder="group.needsPassword ? '本次使用的密码或令牌' : '留空使用已保存的密码'"
                  aria-label="镜像源密码"
                  @update:model-value="(value) => emit('update:image-pull-credential', {
                    registry: group.registry,
                    credential: value
                  })"
                />
                <p v-if="!(group.sources || []).length" class="field-hint">
                  未找到匹配的镜像源。公开镜像可直接拉取；私有镜像请先在“设置 &gt; 镜像设置”中配置对应镜像源。
                </p>
                <p v-else-if="group.needsPassword" class="field-hint">
                  请填写本次使用的密码或令牌，凭据只用于本次拉取。
                </p>
                <p v-else-if="group.hasUsername" class="field-hint">
                  留空时使用该镜像源已保存的密码；填写后只用于本次拉取。
                </p>
                <p v-else class="field-hint">
                  留空时使用当前 Docker 登录状态。
                </p>
              </div>
            </div>
            <p v-else-if="imageSource !== 'offline' && !pullableCount" class="field-hint">
              当前没有可通过拉取修复的镜像，请改用离线镜像包或检查交付包。
            </p>
            <p v-if="imageSource !== 'offline' && imageRegistryLoadError" class="field-hint">
              {{ imageRegistryLoadError }}；仍可按清单地址尝试公开拉取。
            </p>
            <div v-if="imageSource === 'offline'" class="input-row wrap offline-row">
              <el-input
                :model-value="imageMediaDir"
                :disabled="busy"
                :placeholder="bundleDir || '本机离线介质目录'"
                aria-label="镜像介质目录"
                @update:model-value="(value) => emit('update:image-media-dir', value)"
              />
              <el-button :disabled="busy" @click="emit('pick-image-directory')">选择目录</el-button>
            </div>
            <div class="input-row wrap">
              <el-button
                v-if="canPrepare"
                type="primary"
                :loading="importingImages"
                :disabled="busy || !canPrepare"
                @click="emit('prepare-images')"
              >
                {{ prepareButtonText }}
              </el-button>
              <el-button :disabled="busy" @click="emit('recheck')">重新检查</el-button>
            </div>
            <div v-if="imageSource === 'pull' && manualPullCommands.length" class="manual-pull">
              <div class="manual-pull-head">
                <h4>手动拉取</h4>
                <el-button size="small" :disabled="busy" @click="copyManualPullCommands">
                  复制指令
                </el-button>
              </div>
              <p class="field-hint">
                自动拉取仍失败时，请打开{{ isRemote ? '目标主机' : '本机' }}终端执行以下指令，完成后点击“重新检查”。
              </p>
              <pre>{{ manualPullCommands.join('\n') }}</pre>
            </div>
          </div>
          <div v-else class="image-preparation image-ready">
            <h4>镜像已就绪</h4>
            <p class="field-hint">目标 Docker 已具备清单中的全部镜像，可以直接继续。</p>
            <el-button :disabled="busy" @click="emit('recheck')">重新检查</el-button>
          </div>
          <div v-if="importingImages && imagePhase" class="image-progress" role="status">
            <p>{{ imagePhase }}</p>
            <el-progress v-if="imageProgress" :percentage="imageProgress.percent" />
            <p v-if="imageProgress?.message" class="field-hint">{{ imageProgress.message }}</p>
          </div>
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
    </template>
    <div class="step-footer">
      <el-button :disabled="busy" @click="emit('previous')">上一步</el-button>
      <div class="footer-actions">
        <p v-if="imageNextBlockedReason" class="next-blocked" role="status">
          {{ imageNextBlockedReason }}
        </p>
        <el-button type="primary" :disabled="busy || !review?.ready" @click="emit('next')">
          下一步：部署版本
          <el-icon class="el-icon--right"><ArrowRight /></el-icon>
        </el-button>
      </div>
    </div>
  </div>
</template>

<script setup>
import { computed, ref, watch } from 'vue';
import { ElMessage } from 'element-plus';
import { ArrowRight } from '@element-plus/icons-vue';
import ConfigDiffDialog from './ConfigDiffDialog.vue';
import {
  artifactStatusLabel,
  artifactStatusTone,
  changeLabel,
  stateLabel
} from '../domain/review.js';
import {
  imageStatusDetail,
  imageStatusLabel,
  imageStatusTone,
  pullableImages
} from '../domain/images.js';
import { formatSize } from '../../../shared/format.js';
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
  imagePullGroups: { type: Array, default: () => [] },
  imageCounts: { type: Object, default: () => ({}) },
  imagePendingCount: { type: Number, default: 0 },
  imageDigestRefs: { type: Array, default: () => [] },
  imageBlockedCount: { type: Number, default: 0 },
  imageNextBlockedReason: { type: String, default: '' },
  imageRegistryLoadError: { type: String, default: '' },
  imagePhase: { type: String, default: '' },
  imageProgress: { type: Object, default: null },
  imageDiagnostics: { type: String, default: '' },
  imageResult: { type: String, default: '' },
  importingImages: { type: Boolean, default: false },
  reviewTab: { type: String, default: 'images' }
});

const emit = defineEmits([
  'recheck',
  'update:review-tab',
  'update:image-media-dir',
  'update:image-source',
  'update:image-pull-source',
  'update:image-pull-credential',
  'pick-image-directory',
  'prepare-images',
  'previous',
  'next'
]);

const configDiffOpen = ref(false);
const configDiffService = ref('');

const totalImageCount = computed(() => props.review?.images?.length || 0);
const readyImageCount = computed(
  () => props.review?.images?.filter((image) => image.status === 'ready').length || 0
);
const pullableCount = computed(() =>
  Math.max(0, Number(props.imagePendingCount) - Number(props.imageBlockedCount))
);
const transferableCount = computed(() =>
  props.imageSource === 'offline'
    ? Math.max(0, Number(props.imagePendingCount) - props.imageDigestRefs.length)
    : pullableCount.value
);
const imageSummary = computed(() => {
  const counts = props.imageCounts || {};
  const parts = [];
  if (counts.missing) parts.push(`缺失 ${counts.missing} 个`);
  if (counts.mismatch) parts.push(`架构不匹配 ${counts.mismatch} 个`);
  if (counts.failed) parts.push(`检查失败 ${counts.failed} 个`);
  if (counts.invalid) parts.push(`地址无效 ${counts.invalid} 个`);
  return parts.length ? `${parts.join('，')}。` : '所需镜像均已具备。';
});
const digestTransferNotice = computed(() => {
  if (props.imageSource !== 'offline') return '';
  if (!props.imageDigestRefs.length) return '';
  return `离线镜像包无法核对仓库摘要，请选择“${props.isRemote ? '本机拉取后传入' : '从镜像仓库拉取'}”处理以下镜像：${props.imageDigestRefs.join('、')}`;
});
const artifactProblemCount = computed(() => {
  if (!props.review?.hasDeployment) return 0;
  return (props.review.artifacts || []).filter((artifact) => artifact.status !== 'present').length;
});
const artifactFixableCount = computed(
  () =>
    (props.review?.artifacts || []).filter((artifact) =>
      ['missing', 'incomplete'].includes(artifact.status)
    ).length
);
const artifactBlockedCount = computed(
  () =>
    (props.review?.artifacts || []).filter((artifact) =>
      ['unconfirmed', 'invalid'].includes(artifact.status)
    ).length
);
const artifactHint = computed(() => {
  if (!props.review?.artifacts?.length) {
    return '交付包没有声明需要写入目标目录的制品文件。';
  }
  return props.review.hasDeployment
    ? '这里显示目标机上实际存在的制品文件；下一步会部署完整版本，全部受管文件以交付包为准。'
    : '首次部署会在下一步把这些制品文件写入目标目录。';
});
const prepareButtonText = computed(() => {
  if (props.imageSource === 'pull') return `拉取 ${pullableCount.value} 个镜像`;
  if (props.imageSource === 'relay') return '本机拉取并传入';
  return props.isRemote ? '传输并导入镜像' : '导入镜像并比对';
});
const manualPullCommands = computed(() =>
  pullableImages(props.review?.images || []).map((image) => {
    const platform = image.targetPlatform || props.review?.targetPlatform;
    return image.status === 'platform_mismatch' && platform
      ? `docker pull --platform ${platform} ${image.imageRef}`
      : `docker pull ${image.imageRef}`;
  })
);
const canPrepare = computed(
  () =>
    props.imagePendingCount > 0 &&
    (props.imageSource === 'pull' ? pullableCount.value > 0 : transferableCount.value > 0) &&
    (props.imageSource !== 'relay' || props.isRemote)
);

async function copyManualPullCommands() {
  try {
    await navigator.clipboard.writeText(manualPullCommands.value.join('\n'));
    ElMessage.success('拉取指令已复制');
  } catch {
    ElMessage.error('复制失败，请手动选择指令');
  }
}

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
h4 {
  font-size: 14px;
  font-weight: 600;
  margin-bottom: 8px;
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
.tab-hint {
  margin: 0 0 12px;
}
.artifact-alert {
  margin-bottom: 12px;
}
.digest-alert {
  margin-top: 12px;
}
.artifact-actions {
  margin-top: 14px;
}
.image-summary {
  display: flex;
  gap: 14px;
  flex-wrap: wrap;
  margin-bottom: 12px;
  color: var(--text-secondary);
  font-size: 12.5px;
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
.image-ready {
  display: flex;
  align-items: center;
  gap: 16px;
  flex-wrap: wrap;
}
.image-ready .field-hint {
  flex: 1;
  min-width: 220px;
  margin: 0;
}
.registry-groups {
  display: grid;
  gap: 12px;
  margin-top: 14px;
}
.registry-group {
  padding: 14px 16px;
  background: var(--panel-bg);
  border: 1px solid var(--border-color);
  border-radius: var(--radius-sm);
}
.registry-group-title {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: 12px;
  margin-bottom: 8px;
  font-size: 13px;
  font-weight: 600;
}
.registry-group-title .field-hint {
  width: auto;
  margin: 0;
  font-weight: 400;
}
.registry-group .el-select,
.registry-group .el-input {
  width: 100%;
}
.registry-group .el-input {
  margin-top: 10px;
}
.offline-row {
  margin-top: 14px;
}
.image-progress {
  margin-top: 16px;
  font-size: 13px;
}
.image-diagnostics pre {
  white-space: pre-wrap;
  overflow-wrap: anywhere;
}
.manual-pull {
  margin-top: 18px;
  padding-top: 16px;
  border-top: 1px dashed var(--border-color);
}
.manual-pull-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
}
.manual-pull pre {
  margin: 10px 0 0;
  padding: 12px;
  background: var(--panel-bg);
  border: 1px solid var(--border-color);
  border-radius: var(--radius-sm);
  white-space: pre-wrap;
  overflow-wrap: anywhere;
  font-size: 12px;
}
code {
  font-size: 12px;
  overflow-wrap: anywhere;
}
:deep(.el-table .cell) {
  overflow-wrap: anywhere;
}
.muted {
  color: var(--text-muted);
  font-size: 12px;
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
.footer-actions {
  display: flex;
  align-items: center;
  justify-content: flex-end;
  gap: 14px;
  flex-wrap: wrap;
}
.next-blocked {
  margin: 0;
  color: var(--text-muted);
  font-size: 12px;
  line-height: 1.5;
}
@media (max-width: 760px) {
  .step-card {
    padding: 20px;
  }
  .step-footer,
  .section-heading {
    flex-wrap: wrap;
  }
  .footer-actions {
    justify-content: flex-start;
  }
}
</style>
