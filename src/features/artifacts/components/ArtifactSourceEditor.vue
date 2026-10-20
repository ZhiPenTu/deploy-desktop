<template>
  <el-dialog
    v-model="visible"
    :title="editing ? '编辑制品源' : '新建制品源'"
    width="580px"
    :close-on-click-modal="!saving"
  >
    <el-form label-width="112px" label-position="left">
      <el-form-item label="名称">
        <el-input v-model="form.name" placeholder="例如：生产交付流水线" />
      </el-form-item>
      <el-form-item label="类型">
        <el-select v-model="form.type" style="width: 100%">
          <el-option label="云效流水线" value="yunxiaoFlow" />
          <el-option label="直接下载地址" value="url" />
        </el-select>
      </el-form-item>
      <template v-if="form.type === 'yunxiaoFlow'">
        <el-form-item label="制品名">
          <el-input v-model="form.artifactName" placeholder="deploy-bundle" />
        </el-form-item>
        <el-form-item label="组织 ID">
          <el-input v-model="form.orgId" placeholder="云效组织 ID" />
        </el-form-item>
        <el-form-item label="流水线 ID">
          <el-input v-model="form.pipelineId" placeholder="数字流水线 ID" />
        </el-form-item>
        <el-form-item label="访问令牌">
          <el-input
            v-model="tokenInput"
            type="password"
            show-password
            :placeholder="editing && form.hasToken ? '已保存，留空表示不修改' : '云效个人访问令牌'"
          />
        </el-form-item>
        <el-form-item v-if="editing && form.hasToken" label=" ">
          <el-checkbox v-model="clearToken">清除已保存的令牌</el-checkbox>
        </el-form-item>
      </template>
      <template v-else>
        <el-form-item label="制品名" required>
          <el-input v-model="form.artifactName" placeholder="deploy-bundle" />
        </el-form-item>
        <el-form-item label="版本号" required>
          <el-input v-model="form.version" placeholder="填写交付包的版本号" />
        </el-form-item>
        <el-form-item label="下载地址">
          <el-input v-model="form.downloadUrl" placeholder="https://example.com/deploy-bundle.tgz" />
        </el-form-item>
      </template>
    </el-form>
    <p class="field-hint">
      令牌仅在本机使用，供当前用户读取。
    </p>
    <template #footer>
      <el-button @click="visible = false">取消</el-button>
      <el-button type="primary" :loading="saving" @click="save">保存</el-button>
    </template>
  </el-dialog>
</template>

<script setup>
import { computed, reactive, ref, watch } from 'vue';
import { ElMessage } from 'element-plus';
import { saveArtifactSource } from '../../../api/tauri.js';

const props = defineProps({
  modelValue: {
    type: Boolean,
    default: false
  },
  source: {
    type: Object,
    default: null
  }
});

const emit = defineEmits(['update:modelValue', 'saved']);

const saving = ref(false);
const tokenInput = ref('');
const clearToken = ref(false);
const form = reactive({
  id: '',
  name: '',
  type: 'yunxiaoFlow',
  orgId: '',
  pipelineId: '',
  artifactName: 'deploy-bundle',
  version: '',
  downloadUrl: '',
  hasToken: false
});

const visible = computed({
  get: () => props.modelValue,
  set: (value) => emit('update:modelValue', value)
});

const editing = computed(() => Boolean(props.source && props.source.id));

watch(
  () => [props.modelValue, props.source],
  ([open]) => {
    if (open) {
      resetForm();
    }
  },
  { immediate: true }
);

function resetForm() {
  const source = props.source || {};
  Object.assign(form, {
    id: source.id || '',
    name: source.name || '',
    type: source.type || 'yunxiaoFlow',
    orgId: source.orgId || '',
    pipelineId: source.pipelineId || '',
    artifactName: source.artifactName || 'deploy-bundle',
    version: source.version || (source.lastVersion !== 'manual' ? source.lastVersion : '') || '',
    downloadUrl: source.downloadUrl || '',
    hasToken: Boolean(source.hasToken)
  });
  tokenInput.value = '';
  clearToken.value = false;
}

async function save() {
  if (!form.name.trim() || !form.artifactName.trim()) {
    ElMessage.warning('请填写名称和制品名');
    return;
  }
  if (form.type === 'url' && (form.version.trim() === 'manual' || !/^[a-zA-Z0-9][a-zA-Z0-9._-]{0,127}$/.test(form.version.trim()))) {
    ElMessage.warning('请填写版本号，以字母或数字开头，仅使用字母、数字、点、短横线和下划线，最多 128 个字符');
    return;
  }
  saving.value = true;
  try {
    await saveArtifactSource({
      id: form.id,
      name: form.name,
      type: form.type,
      orgId: form.orgId,
      pipelineId: form.pipelineId,
      artifactName: form.artifactName,
      version: form.version,
      downloadUrl: form.downloadUrl,
      token: tokenInput.value ? tokenInput.value : null,
      clearToken: clearToken.value
    });
    visible.value = false;
    ElMessage.success('制品源已保存');
    emit('saved');
  } catch (error) {
    ElMessage.error(String(error));
  } finally {
    saving.value = false;
  }
}
</script>

<style scoped>
.field-hint {
  margin: 0;
  font-size: 11px;
  line-height: 1.55;
  color: var(--text-muted);
}
</style>
