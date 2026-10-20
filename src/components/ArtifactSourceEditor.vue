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
        <el-form-item label="组织 ID">
          <el-input v-model="form.orgId" placeholder="云效组织 ID" />
        </el-form-item>
        <el-form-item label="流水线 ID">
          <el-input v-model="form.pipelineId" placeholder="数字流水线 ID" />
        </el-form-item>
        <el-form-item label="制品名">
          <el-input v-model="form.artifactName" placeholder="deploy-bundle" />
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
        <el-form-item label="下载地址">
          <el-input v-model="form.downloadUrl" placeholder="https://example.com/deploy-bundle.tgz" />
        </el-form-item>
      </template>
    </el-form>
    <p class="field-hint">
      令牌只在本机使用，保存在应用数据目录的 artifact-tokens.json（仅当前用户可读），配置文件中只记录组织、流水线与制品名。
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
import { saveArtifactSource } from '../api/artifact.js';

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
    downloadUrl: source.downloadUrl || '',
    hasToken: Boolean(source.hasToken)
  });
  tokenInput.value = '';
  clearToken.value = false;
}

async function save() {
  saving.value = true;
  try {
    await saveArtifactSource({
      id: form.id,
      name: form.name,
      type: form.type,
      orgId: form.orgId,
      pipelineId: form.pipelineId,
      artifactName: form.artifactName,
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
