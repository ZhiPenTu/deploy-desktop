<template>
  <el-dialog
    v-model="visible"
    :title="editing ? '编辑主机' : '新增主机'"
    width="620px"
    :close-on-click-modal="!saving"
  >
    <el-form label-width="112px" label-position="left">
      <el-form-item label="主机类型">
        <el-radio-group v-model="form.kind" :disabled="editing">
          <el-radio-button value="local" :disabled="!localAvailable">
            本机（当前电脑）
          </el-radio-button>
          <el-radio-button value="ssh">远程主机（SSH）</el-radio-button>
        </el-radio-group>
        <p v-if="!localAvailable && !editing" class="kind-hint">
          本机已添加，可在列表中直接编辑。
        </p>
      </el-form-item>
      <el-form-item label="主机名称" required>
        <el-input
          v-model="form.name"
          :placeholder="isLocal ? '例如：本机' : '例如：生产应用服务器'"
        />
      </el-form-item>
      <template v-if="!isLocal">
        <el-row :gutter="18">
          <el-col :span="14">
            <el-form-item label="主机地址" required>
              <el-input v-model="form.address" placeholder="例如：192.0.2.10" />
            </el-form-item>
          </el-col>
          <el-col :span="10">
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
          <p v-if="!credentials.length" class="kind-hint">
            还没有 SSH 凭证，保存远程主机前请先在凭证管理中创建。
          </p>
        </el-form-item>
        <div v-if="selectedCredential" class="credential-summary">
          <span>登录用户 {{ selectedCredential.username }}</span>
          <span>{{ selectedCredential.privateKeyPath || '自动选择默认密钥' }}</span>
        </div>
      </template>
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
      <template v-if="isLocal">
        本机主机不需要 SSH 地址与登录凭证；切换为当前主机后，部署和运维操作直接在当前电脑执行。
      </template>
      <template v-else>
        选择主机后会一并带入登录身份与部署目录；在部署向导中临时修改的地址不会覆盖这里保存的内容。
      </template>
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
import { saveManagedHost } from '../../../api/tauri.js';
import { errorText } from '../../../shared/errors.js';

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
  hosts: {
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
  kind: 'ssh',
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
const isLocal = computed(() => form.kind === 'local');
const localAvailable = computed(
  () => !props.hosts.some((host) => host.kind === 'local' && host.id !== props.host?.id)
);
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
  const kind = host.kind || (localAvailable.value ? 'local' : 'ssh');
  Object.assign(form, {
    id: host.id || '',
    kind,
    name: host.name || (kind === 'local' ? '本机' : ''),
    address: host.address || '',
    port: host.port || 22,
    credentialId: host.credentialId || (kind === 'ssh' ? props.credentials[0]?.id || '' : ''),
    deployDir: host.deployDir || props.defaultDeployDir || '',
    notes: host.notes || ''
  });
}

watch(
  () => form.kind,
  (kind, previous) => {
    if (editing.value) return;
    if (kind === 'local' && !form.name.trim()) {
      form.name = '本机';
    }
    if (kind === 'ssh' && previous === 'local' && form.name.trim() === '本机') {
      form.name = '';
    }
    if (kind === 'ssh' && !form.credentialId) {
      form.credentialId = props.credentials[0]?.id || '';
    }
  }
);

async function save() {
  if (!form.name.trim()) {
    ElMessage.warning('请填写主机名称');
    return;
  }
  if (!isLocal.value && !form.address.trim()) {
    ElMessage.warning('请填写主机地址');
    return;
  }
  if (!isLocal.value && !form.credentialId) {
    ElMessage.warning('请选择 SSH 凭证');
    return;
  }

  saving.value = true;
  try {
    const saved = await saveManagedHost({
      id: form.id,
      kind: form.kind,
      name: form.name.trim(),
      address: isLocal.value ? '' : form.address.trim(),
      port: isLocal.value ? 22 : form.port,
      credentialId: isLocal.value ? '' : form.credentialId,
      deployDir: form.deployDir.trim(),
      notes: form.notes.trim()
    });
    visible.value = false;
    ElMessage.success('主机已保存');
    emit('saved', saved);
  } catch (error) {
    ElMessage.error(errorText(error));
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

.kind-hint {
  width: 100%;
  margin: 5px 0 0;
  font-size: 11px;
  line-height: 1.55;
  color: var(--text-muted);
}
</style>
