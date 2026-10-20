<template>
  <el-dialog
    v-model="visible"
    :title="editing ? '编辑主机' : '新增主机'"
    width="620px"
    :close-on-click-modal="!saving"
  >
    <el-form label-width="112px" label-position="left">
      <el-form-item label="主机名称" required>
        <el-input v-model="form.name" placeholder="例如：生产应用服务器" />
      </el-form-item>
      <el-row :gutter="18">
        <el-col :span="16">
          <el-form-item label="主机地址" required>
            <el-input v-model="form.address" placeholder="例如：192.0.2.10" />
          </el-form-item>
        </el-col>
        <el-col :span="8">
          <el-form-item label="SSH 端口" label-width="84px">
            <el-input-number v-model="form.port" :min="1" :max="65535" style="width: 100%" />
          </el-form-item>
        </el-col>
      </el-row>
      <el-form-item label="SSH 凭证" required>
        <el-select v-model="form.credentialId" style="width: 100%" placeholder="选择登录凭证">
          <el-option
            v-for="credential in credentials"
            :key="credential.id"
            :label="`${credential.name} · ${credential.username}`"
            :value="credential.id"
          />
        </el-select>
      </el-form-item>
      <div v-if="selectedCredential" class="credential-summary">
        <span>登录用户 {{ selectedCredential.username }}</span>
        <span>{{ selectedCredential.privateKeyPath || '自动选择默认密钥' }}</span>
      </div>
      <el-form-item label="部署目录">
        <el-input v-model="form.deployDir" placeholder="例如：/opt/deploy" />
      </el-form-item>
      <el-form-item label="备注">
        <el-input
          v-model="form.notes"
          type="textarea"
          :rows="3"
          maxlength="200"
          show-word-limit
          placeholder="记录用途、网络区域或维护窗口等信息"
        />
      </el-form-item>
    </el-form>
    <p class="field-hint">
      选择主机后会一并带入登录身份与部署目录；在部署向导中临时修改的地址不会覆盖这里保存的内容。
    </p>
    <template #footer>
      <el-button @click="visible = false">取消</el-button>
      <el-button type="primary" :loading="saving" :disabled="!credentials.length" @click="save">保存</el-button>
    </template>
  </el-dialog>
</template>

<script setup>
import { computed, reactive, ref, watch } from 'vue';
import { ElMessage } from 'element-plus';
import { saveManagedHost } from '../api/tauri.js';

const props = defineProps({
  modelValue: {
    type: Boolean,
    default: false
  },
  host: {
    type: Object,
    default: null
  },
  credentials: {
    type: Array,
    default: () => []
  },
  defaultDeployDir: {
    type: String,
    default: ''
  }
});

const emit = defineEmits(['update:modelValue', 'saved']);

const saving = ref(false);
const form = reactive({
  id: '',
  name: '',
  address: '',
  port: 22,
  credentialId: '',
  deployDir: '',
  notes: ''
});

const visible = computed({
  get: () => props.modelValue,
  set: (value) => emit('update:modelValue', value)
});

const editing = computed(() => Boolean(props.host?.id));
const selectedCredential = computed(
  () => props.credentials.find((credential) => credential.id === form.credentialId) || null
);

watch(
  () => [props.modelValue, props.host, props.credentials, props.defaultDeployDir],
  ([open]) => {
    if (open) {
      resetForm();
    }
  },
  { immediate: true }
);

function resetForm() {
  const host = props.host || {};
  Object.assign(form, {
    id: host.id || '',
    name: host.name || '',
    address: host.address || '',
    port: host.port || 22,
    credentialId: host.credentialId || props.credentials[0]?.id || '',
    deployDir: host.deployDir || props.defaultDeployDir || '',
    notes: host.notes || ''
  });
}

async function save() {
  if (!form.name.trim()) {
    ElMessage.warning('请填写主机名称');
    return;
  }
  if (!form.address.trim()) {
    ElMessage.warning('请填写主机地址');
    return;
  }
  if (!form.credentialId) {
    ElMessage.warning('请选择 SSH 凭证');
    return;
  }

  saving.value = true;
  try {
    const saved = await saveManagedHost({
      id: form.id,
      name: form.name.trim(),
      address: form.address.trim(),
      port: form.port,
      credentialId: form.credentialId,
      deployDir: form.deployDir.trim(),
      notes: form.notes.trim()
    });
    visible.value = false;
    ElMessage.success('主机已保存');
    emit('saved', saved);
  } catch (error) {
    ElMessage.error(String(error));
  } finally {
    saving.value = false;
  }
}
</script>

<style scoped>
.credential-summary {
  display: flex;
  gap: 8px 18px;
  flex-wrap: wrap;
  margin: -4px 0 16px 112px;
  padding: 9px 12px;
  border-radius: var(--radius-sm);
  background: var(--panel-subtle);
  color: var(--text-muted);
  font-size: 11px;
  line-height: 1.5;
}

.field-hint {
  margin: 0;
  padding-left: 112px;
  font-size: 11px;
  line-height: 1.6;
  color: var(--text-muted);
}
</style>
