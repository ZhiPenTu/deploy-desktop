<template>
  <el-dialog
    :model-value="visible"
    title="拉取镜像"
    width="600px"
    style="max-width: calc(100vw - 32px)"
    :close-on-click-modal="!pulling"
    :close-on-press-escape="!pulling"
    :show-close="!pulling"
    @update:model-value="onVisibleChange"
    @closed="form.password = ''"
  >
    <el-form label-width="100px" :disabled="pulling" @submit.prevent>
      <el-form-item label="镜像源">
        <el-select
          v-model="form.registrySourceId"
          :loading="loadingSources"
          placeholder="直接填写完整镜像地址"
          clearable
          style="width: 100%"
          @change="resetCredentials"
        >
          <el-option label="直接填写完整镜像地址" value="" />
          <el-option
            v-for="source in sources"
            :key="source.id"
            :label="`${source.name}（${source.address}）`"
            :value="source.id"
          />
        </el-select>
        <el-button link type="primary" @click="openSourceSettings">管理镜像源</el-button>
      </el-form-item>
      <el-form-item label="镜像地址" required>
        <el-input
          v-model="form.reference"
          :placeholder="
            selectedSource
              ? '仓库内路径，例如 app@sha256:…'
              : '例如 registry.example.com/demo/app@sha256:…'
          "
          clearable
          @keyup.enter="submit"
        />
      </el-form-item>
      <el-form-item v-if="!selectedSource" label="仓库用户名">
        <el-input v-model="form.username" placeholder="公开镜像可留空" autocomplete="off" clearable />
      </el-form-item>
      <el-form-item
        v-if="!selectedSource || (selectedSource.username && !selectedSource.hasPassword)"
        label="密码或令牌"
      >
        <el-input
          v-model="form.password"
          type="password"
          show-password
          placeholder="仅用于本次拉取，不会保存"
          autocomplete="new-password"
        />
      </el-form-item>
      <el-form-item v-if="selectedSource" label="访问方式">
        <span>{{ accessText }}</span>
      </el-form-item>
    </el-form>
    <div class="dialog-hint">
      使用 sha256 摘要固定镜像版本。镜像拉取到本机 Docker 后，可导出为离线包。
      <div v-if="form.reference.trim()">拉取地址：{{ resolvedReference }}</div>
    </div>
    <template #footer>
      <el-button :disabled="pulling" @click="close">取消</el-button>
      <el-button type="primary" :loading="pulling" :disabled="loadingSources" @click="submit">
        开始拉取
      </el-button>
    </template>
  </el-dialog>
</template>

<script setup>
import { computed, reactive, watch } from 'vue';
import { ElMessage } from 'element-plus';

const props = defineProps({
  visible: { type: Boolean, default: false },
  sources: { type: Array, default: () => [] },
  loadingSources: { type: Boolean, default: false },
  pulling: { type: Boolean, default: false }
});

const emit = defineEmits(['update:visible', 'submit', 'go-settings']);

const form = reactive({ reference: '', registrySourceId: '', username: '', password: '' });

const selectedSource = computed(
  () => props.sources.find((source) => source.id === form.registrySourceId) || null
);

const resolvedReference = computed(() => {
  const reference = form.reference.trim();
  const source = selectedSource.value;
  if (!source) return reference;
  const first = reference.split('/')[0];
  if (reference.includes('/') && (first.includes('.') || first.includes(':') || first === 'localhost')) {
    return reference;
  }
  return [source.address, source.namespace, reference].filter(Boolean).join('/');
});

const accessText = computed(() => {
  const source = selectedSource.value;
  if (!source) return '';
  if (!source.username) return '无需登录';
  return `${source.username}（${source.hasPassword ? '使用已保存的密码' : '请填写本次密码'}）`;
});

function resetCredentials() {
  form.username = '';
  form.password = '';
}

function resetForm() {
  form.reference = '';
  form.registrySourceId = '';
  form.username = '';
  form.password = '';
}

function onVisibleChange(value) {
  if (value) return;
  close();
}

function close() {
  if (props.pulling) return;
  emit('update:visible', false);
  form.password = '';
}

function openSourceSettings() {
  if (props.pulling) return;
  close();
  emit('go-settings');
}

function submit() {
  if (props.pulling || props.loadingSources) return;
  const reference = form.reference.trim();
  if (!reference) {
    ElMessage.warning('请填写要拉取的镜像地址');
    return;
  }
  if (!/@sha256:[a-f0-9]{64}$/.test(reference)) {
    ElMessage.warning('请填写包含 64 位小写 sha256 摘要的镜像地址');
    return;
  }
  const source = selectedSource.value;
  if (
    (source?.username && !source.hasPassword && !form.password) ||
    (!source && form.username.trim() && !form.password)
  ) {
    ElMessage.warning('请填写仓库密码或访问令牌');
    return;
  }
  if (!source && form.password && !form.username.trim()) {
    ElMessage.warning('已填写仓库密码，请同时填写仓库用户名');
    return;
  }
  emit('submit', {
    reference,
    resolved: resolvedReference.value,
    username: source ? null : form.username.trim() || null,
    password: form.password || null,
    registrySourceId: form.registrySourceId || null
  });
}

watch(
  () => props.visible,
  (visible) => {
    if (visible) resetForm();
  }
);

watch(
  () => props.pulling,
  (pulling) => {
    if (!pulling) form.password = '';
  }
);

watch(
  () => props.sources,
  (sources) => {
    if (!sources.some((source) => source.id === form.registrySourceId)) {
      form.registrySourceId = '';
    }
  }
);
</script>

<style scoped>
.dialog-hint {
  padding: 10px 14px;
  border-radius: var(--radius-sm);
  background: var(--panel-subtle);
  border: 1px solid var(--border-color);
  color: var(--text-muted);
  font-size: 12px;
  line-height: 1.6;
}
</style>
