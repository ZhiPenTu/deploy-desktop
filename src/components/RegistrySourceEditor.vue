<template>
  <el-dialog
    v-model="visible"
    :title="editing ? '编辑镜像源' : '新增镜像源'"
    width="600px"
    style="max-width: calc(100vw - 32px)"
    :close-on-click-modal="!saving"
    :close-on-press-escape="!saving"
    :show-close="!saving"
    @closed="passwordInput = ''"
  >
    <el-form label-width="100px" :disabled="saving" @submit.prevent>
      <el-form-item label="名称" required>
        <el-input v-model="form.name" placeholder="例如：团队私有仓库" maxlength="80" />
      </el-form-item>
      <el-form-item label="仓库地址" required>
        <el-input v-model="form.address" placeholder="例如：registry.example.com:5000" autocomplete="off" />
        <div class="field-hint">填写仓库主机名和端口，命名空间在下方单独填写。</div>
      </el-form-item>
      <el-form-item label="命名空间">
        <el-input v-model="form.namespace" placeholder="例如：team/project（可选）" />
      </el-form-item>
      <el-form-item label="仓库用户名">
        <el-input v-model="form.username" placeholder="公开仓库可留空" autocomplete="off" />
      </el-form-item>
      <el-form-item label="密码或令牌">
        <el-input
          v-model="passwordInput"
          type="password"
          show-password
          :disabled="clearPassword"
          :placeholder="source?.hasPassword ? '已保存，留空表示不修改' : '留空则在每次拉取时输入'"
          autocomplete="new-password"
        />
      </el-form-item>
      <el-form-item v-if="source?.hasPassword" label=" ">
        <el-checkbox v-model="clearPassword">清除已保存的密码</el-checkbox>
      </el-form-item>
    </el-form>
    <p class="field-hint">密码仅保存在本机，保存后不再显示原文。更换仓库地址或用户名后，需要重新填写密码。</p>
    <template #footer>
      <el-button :disabled="saving" @click="visible = false">取消</el-button>
      <el-button type="primary" :loading="saving" @click="save">保存</el-button>
    </template>
  </el-dialog>
</template>

<script setup>
import { computed, reactive, ref, watch } from 'vue';
import { ElMessage } from 'element-plus';
import { saveRegistrySource } from '../api/tauri.js';

const props = defineProps({ modelValue: Boolean, source: { type: Object, default: null } });
const emit = defineEmits(['update:modelValue', 'saved']);
const saving = ref(false);
const passwordInput = ref('');
const clearPassword = ref(false);
const form = reactive({ id: '', name: '', address: '', namespace: '', username: '' });
const editing = computed(() => Boolean(props.source?.id));
const visible = computed({ get: () => props.modelValue, set: (value) => emit('update:modelValue', value) });

watch(() => props.modelValue, (opened) => {
  if (!opened) return;
  for (const field of Object.keys(form)) form[field] = props.source?.[field] || '';
  passwordInput.value = '';
  clearPassword.value = false;
});

async function save() {
  if (saving.value) return;
  if (!form.name.trim() || !form.address.trim()) {
    ElMessage.warning('请填写镜像源名称和仓库地址');
    return;
  }
  if (passwordInput.value && !clearPassword.value && !form.username.trim()) {
    ElMessage.warning('已填写仓库密码，请同时填写仓库用户名');
    return;
  }
  saving.value = true;
  try {
    const saved = await saveRegistrySource({
      ...Object.fromEntries(Object.entries(form).map(([field, value]) => [field, value.trim()])),
      password: clearPassword.value ? null : passwordInput.value || null,
      clearPassword: clearPassword.value
    });
    passwordInput.value = '';
    visible.value = false;
    ElMessage.success('镜像源已保存');
    emit('saved', saved);
  } catch (error) {
    ElMessage.error(error instanceof Error ? error.message : String(error));
  } finally {
    saving.value = false;
  }
}
</script>

<style scoped>
.field-hint {
  width: 100%;
  margin: 6px 0 0;
  font-size: 12px;
  line-height: 1.6;
  color: var(--text-muted);
}
</style>
