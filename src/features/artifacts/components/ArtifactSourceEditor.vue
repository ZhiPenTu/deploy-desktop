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
          <div class="token-field">
            <el-select v-model="form.tokenId" placeholder="选择已保存的令牌" style="width: 100%">
              <el-option
                v-for="item in tokens"
                :key="item.id"
                :label="item.name"
                :value="item.id"
              />
              <el-option v-if="legacyToken" label="沿用当前保存的令牌" value="__legacy__" />
              <el-option label="新建访问令牌" value="__new__" />
            </el-select>
            <span v-if="selectedTokenMissing" class="token-warning">
              该令牌的内容已缺失，请在设置中重新填写。
            </span>
          </div>
        </el-form-item>
        <template v-if="form.tokenId === '__new__'">
          <el-form-item label="令牌名称">
            <el-input v-model="newToken.name" placeholder="例如：云效个人令牌" />
          </el-form-item>
          <el-form-item label="令牌内容">
            <el-input
              v-model="newToken.value"
              type="password"
              show-password
              autocomplete="new-password"
              placeholder="云效个人访问令牌"
            />
          </el-form-item>
        </template>
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
    <p v-if="form.type === 'yunxiaoFlow'" class="field-hint">
      令牌仅保存在本机，供当前用户读取，并可在多个云效制品源之间复用。
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
import { listArtifactTokens, saveArtifactSource, saveArtifactToken } from '../../../api/tauri.js';
import { errorText } from '../../../shared/errors.js';

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
const tokens = ref([]);
const newToken = reactive({ name: '', value: '' });
const form = reactive({
  id: '',
  name: '',
  type: 'yunxiaoFlow',
  orgId: '',
  pipelineId: '',
  artifactName: 'deploy-bundle',
  version: '',
  downloadUrl: '',
  tokenId: '',
  hasToken: false
});

const visible = computed({
  get: () => props.modelValue,
  set: (value) => emit('update:modelValue', value)
});

const editing = computed(() => Boolean(props.source && props.source.id));
const legacyToken = computed(
  () => editing.value && form.hasToken && !(props.source && props.source.tokenId)
);
const selectedTokenMissing = computed(() =>
  tokens.value.some((item) => item.id === form.tokenId && !item.hasToken)
);

watch(
  () => [props.modelValue, props.source],
  ([open]) => {
    if (open) {
      resetForm();
    }
  },
  { immediate: true }
);

async function resetForm() {
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
    tokenId: source.tokenId || '',
    hasToken: Boolean(source.hasToken)
  });
  newToken.name = '';
  newToken.value = '';
  await loadTokens();
  if (!form.tokenId) {
    form.tokenId = defaultTokenId();
  }
}

async function loadTokens() {
  try {
    tokens.value = await listArtifactTokens();
  } catch (error) {
    tokens.value = [];
    ElMessage.error(errorText(error));
  }
}

function defaultTokenId() {
  if (legacyToken.value) return '__legacy__';
  if (tokens.value.length === 1) return tokens.value[0].id;
  if (!tokens.value.length) return '__new__';
  return '';
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
  if (form.type === 'yunxiaoFlow') {
    if (form.tokenId === '__new__' && (!newToken.name.trim() || !newToken.value.trim())) {
      ElMessage.warning('请填写令牌名称和令牌内容');
      return;
    }
    if (form.tokenId !== '__new__' && !form.tokenId) {
      ElMessage.warning('请选择访问令牌，或新建一个令牌');
      return;
    }
  }
  saving.value = true;
  try {
    let tokenId = '';
    if (form.type === 'yunxiaoFlow') {
      if (form.tokenId === '__new__') {
        const created = await saveArtifactToken({
          name: newToken.name.trim(),
          token: newToken.value
        });
        tokenId = created.id;
        tokens.value = [...tokens.value, created];
        form.tokenId = created.id;
        newToken.name = '';
        newToken.value = '';
      } else if (form.tokenId !== '__legacy__') {
        tokenId = form.tokenId;
      }
    }
    await saveArtifactSource({
      id: form.id,
      name: form.name,
      type: form.type,
      orgId: form.orgId,
      pipelineId: form.pipelineId,
      artifactName: form.artifactName,
      version: form.version,
      downloadUrl: form.downloadUrl,
      tokenId
    });
    visible.value = false;
    ElMessage.success('制品源已保存');
    emit('saved');
  } catch (error) {
    ElMessage.error(errorText(error));
  } finally {
    saving.value = false;
  }
}
</script>

<style scoped>
.token-field {
  width: 100%;
}

.token-warning {
  display: block;
  margin-top: 6px;
  font-size: 11px;
  color: var(--el-color-warning);
}

.field-hint {
  margin: 0;
  font-size: 11px;
  line-height: 1.55;
  color: var(--text-muted);
}
</style>
