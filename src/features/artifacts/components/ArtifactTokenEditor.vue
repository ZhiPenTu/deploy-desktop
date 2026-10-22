<template>
  <el-dialog
    v-model="visible"
    :title="editing ? '编辑访问令牌' : '新建访问令牌'"
    width="560px"
    :close-on-click-modal="!saving"
  >
    <el-form label-width="112px" label-position="left">
      <el-form-item label="令牌名称" required>
        <el-input v-model="form.name" placeholder="例如：云效个人令牌" />
      </el-form-item>
      <el-form-item label="访问令牌" :required="!editing">
        <el-input
          v-model="tokenInput"
          type="password"
          show-password
          autocomplete="new-password"
          :placeholder="editing && form.hasToken ? '已保存，留空表示不修改' : '云效个人访问令牌'"
        />
      </el-form-item>
    </el-form>
    <div class="dialog-tip">
      令牌仅保存在本机，供当前用户读取；保存后可在多个云效制品源之间复用，不必重复填写。
    </div>
    <template #footer>
      <el-button @click="visible = false">取消</el-button>
      <el-button type="primary" :loading="saving" @click="save">保存</el-button>
    </template>
  </el-dialog>
</template>

<script setup>
import { computed, reactive, ref, watch } from 'vue';
import { ElMessage } from 'element-plus';
import { saveArtifactToken } from '../../../api/tauri.js';
import { errorText } from '../../../shared/errors.js';

const props = defineProps({
  modelValue: {
    type: Boolean,
    default: false
  },
  token: {
    type: Object,
    default: null
  }
});

const emit = defineEmits(['update:modelValue', 'saved']);

const saving = ref(false);
const tokenInput = ref('');
const form = reactive({
  id: '',
  name: '',
  hasToken: false
});

const visible = computed({
  get: () => props.modelValue,
  set: (value) => emit('update:modelValue', value)
});

const editing = computed(() => Boolean(props.token?.id));

watch(
  () => [props.modelValue, props.token],
  ([open]) => {
    if (open) {
      resetForm();
    }
  },
  { immediate: true }
);

function resetForm() {
  const token = props.token || {};
  Object.assign(form, {
    id: token.id || '',
    name: token.name || '',
    hasToken: Boolean(token.hasToken)
  });
  tokenInput.value = '';
}

async function save() {
  if (!form.name.trim()) {
    ElMessage.warning('请填写令牌名称');
    return;
  }
  if (!editing.value && !tokenInput.value.trim()) {
    ElMessage.warning('请填写云效个人访问令牌');
    return;
  }

  saving.value = true;
  try {
    const saved = await saveArtifactToken({
      id: form.id,
      name: form.name.trim(),
      token: tokenInput.value ? tokenInput.value : null
    });
    visible.value = false;
    ElMessage.success('访问令牌已保存');
    emit('saved', saved);
  } catch (error) {
    ElMessage.error(errorText(error));
  } finally {
    saving.value = false;
  }
}
</script>

<style scoped>
.dialog-tip {
  margin: 8px 0 0;
  padding: 10px 14px;
  font-size: 12px;
  line-height: 1.6;
  color: var(--text-muted);
  background: var(--panel-subtle);
  border: 1px solid var(--border-color);
  border-radius: var(--radius-sm);
}
</style>
