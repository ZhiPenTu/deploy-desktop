<template>
  <el-dialog
    v-model="visible"
    :title="editing ? '编辑 SSH 凭证' : '新建 SSH 凭证'"
    width="600px"
    :close-on-click-modal="!saving"
  >
    <el-form label-width="112px" label-position="left">
      <el-form-item label="凭证名称" required>
        <el-input v-model="form.name" placeholder="例如：生产服务器运维账号" />
      </el-form-item>
      <el-form-item label="登录用户名" required>
        <el-input v-model="form.username" placeholder="例如：root" autocomplete="off" />
      </el-form-item>
      <el-form-item label="私钥路径">
        <el-input v-model="form.privateKeyPath" placeholder="例如：~/.ssh/id_ed25519" />
      </el-form-item>
      <el-form-item label="初始密码">
        <el-input
          v-model="passwordInput"
          type="password"
          show-password
          :placeholder="editing && form.hasPassword ? '已保存，留空表示不修改' : '选填，仅用于首次配置免密登录'"
          autocomplete="new-password"
        />
      </el-form-item>
      <el-form-item v-if="editing && form.hasPassword" label=" ">
        <el-checkbox v-model="clearPassword">清除已保存的初始密码</el-checkbox>
      </el-form-item>
    </el-form>
    <p class="field-hint">
      私钥保留在原位置，应用只记录路径；留空时尝试使用默认 SSH 密钥。初始密码仅用于首次下发免密密钥，不会显示在列表中。
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
import { saveSshCredential } from '../../../api/tauri.js';
import { errorText } from '../../../shared/errors.js';

const props = defineProps({
  modelValue: {
    type: Boolean,
    default: false
  },
  credential: {
    type: Object,
    default: null
  }
});

const emit = defineEmits(['update:modelValue', 'saved']);

const saving = ref(false);
const passwordInput = ref('');
const clearPassword = ref(false);
const form = reactive({
  id: '',
  name: '',
  username: '',
  privateKeyPath: '',
  hasPassword: false
});

const visible = computed({
  get: () => props.modelValue,
  set: (value) => emit('update:modelValue', value)
});

const editing = computed(() => Boolean(props.credential?.id));

watch(
  () => [props.modelValue, props.credential],
  ([open]) => {
    if (open) {
      resetForm();
    }
  },
  { immediate: true }
);

function resetForm() {
  const credential = props.credential || {};
  Object.assign(form, {
    id: credential.id || '',
    name: credential.name || '',
    username: credential.username || 'root',
    privateKeyPath: credential.privateKeyPath || '',
    hasPassword: Boolean(credential.hasPassword)
  });
  passwordInput.value = '';
  clearPassword.value = false;
}

async function save() {
  if (!form.name.trim()) {
    ElMessage.warning('请填写凭证名称');
    return;
  }
  if (!form.username.trim()) {
    ElMessage.warning('请填写登录用户名');
    return;
  }

  saving.value = true;
  try {
    const saved = await saveSshCredential({
      id: form.id,
      name: form.name.trim(),
      username: form.username.trim(),
      privateKeyPath: form.privateKeyPath.trim(),
      password: passwordInput.value ? passwordInput.value : null,
      clearPassword: clearPassword.value
    });
    visible.value = false;
    ElMessage.success('SSH 凭证已保存');
    emit('saved', saved);
  } catch (error) {
    ElMessage.error(errorText(error));
  } finally {
    saving.value = false;
  }
}
</script>

<style scoped>
.field-hint {
  margin: 0;
  padding-left: 112px;
  font-size: 11px;
  line-height: 1.6;
  color: var(--text-muted);
}
</style>
