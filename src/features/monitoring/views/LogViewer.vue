<template>
  <div class="log-viewer-view">
    <div class="page-header">
      <div>
        <h2 class="page-title">容器日志诊断</h2>
        <p class="page-subtitle">查看核心服务实时输出，按关键字过滤并复制日志内容</p>
      </div>
    </div>

    <div class="log-toolbar card-surface">
      <div class="controls-left">
        <el-select v-model="selectedContainer" placeholder="选择容器" style="width: 280px" @change="fetchLogs">
          <el-option
            v-for="c in containerOptions"
            :key="c.name"
            :label="`${c.name} (${c.label})`"
            :value="c.name"
          />
        </el-select>

        <el-select v-model="tailLines" style="width: 120px" @change="fetchLogs">
          <el-option label="最新 100 行" :value="100" />
          <el-option label="最新 300 行" :value="300" />
          <el-option label="最新 500 行" :value="500" />
          <el-option label="最新 1000 行" :value="1000" />
        </el-select>

        <el-input
          v-model="searchKeyword"
          placeholder="过滤日志关键字..."
          clearable
          :prefix-icon="Search"
          style="width: 220px"
        />
      </div>

      <div class="controls-right">
        <el-switch v-model="autoRefresh" active-text="自动刷新 (3s)" @change="handleAutoRefreshChange" />
        <el-button type="primary" :loading="loading" @click="fetchLogs">
          <el-icon><Refresh /></el-icon>
          <span>刷新</span>
        </el-button>
        <el-button @click="copyLogs">
          <el-icon><CopyDocument /></el-icon>
          <span>复制日志</span>
        </el-button>
      </div>
    </div>

    <div class="terminal-container">
      <div class="terminal-bar">
        <span class="dot red"></span>
        <span class="dot yellow"></span>
        <span class="dot green"></span>
        <span class="bar-title">docker logs --tail {{ tailLines }} {{ selectedContainer }}</span>
      </div>
      <div class="terminal-body" ref="termBody">
        <pre v-if="filteredLogs">{{ filteredLogs }}</pre>
        <div v-else class="empty-tip">暂无日志或该容器尚未产生输出</div>
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, computed, onMounted, onUnmounted, nextTick } from 'vue';
import { ElMessage } from 'element-plus';
import { CopyDocument, Refresh, Search } from '@element-plus/icons-vue';
import { getServiceLogs, getRemoteServiceLogs } from '../../../api/tauri.js';
import { getDefaultContainer, serviceOptions } from '../../../config/cluster.js';

const props = defineProps({
  connection: {
    type: Object,
    required: true
  },
  initialContainer: {
    type: String,
    default: ''
  }
});

const isRemote = computed(() => props.connection.mode === 'remote');

const containerOptions = serviceOptions();

const selectedContainer = ref(props.initialContainer || getDefaultContainer());
const tailLines = ref(300);
const searchKeyword = ref('');
const rawLogs = ref('');
const loading = ref(false);
const autoRefresh = ref(false);
const termBody = ref(null);
let timer = null;

const filteredLogs = computed(() => {
  if (!searchKeyword.value.trim()) return rawLogs.value;
  const kw = searchKeyword.value.toLowerCase();
  return rawLogs.value
    .split('\n')
    .filter((line) => line.toLowerCase().includes(kw))
    .join('\n');
});

const fetchLogs = async () => {
  loading.value = true;
  try {
    rawLogs.value = isRemote.value
      ? await getRemoteServiceLogs(
          props.connection.remoteHost,
          props.connection.remotePort,
          props.connection.remoteUser,
          props.connection.remoteKey || null,
          selectedContainer.value,
          tailLines.value
        )
      : await getServiceLogs(selectedContainer.value, tailLines.value);
    await nextTick();
    if (termBody.value) {
      termBody.value.scrollTop = termBody.value.scrollHeight;
    }
  } catch (e) {
    ElMessage.error(`读取日志失败: ${e}`);
    rawLogs.value = `读取日志失败: ${e}`;
  } finally {
    loading.value = false;
  }
};

const handleAutoRefreshChange = (val) => {
  if (val) {
    timer = setInterval(fetchLogs, 3000);
  } else if (timer) {
    clearInterval(timer);
    timer = null;
  }
};

const copyLogs = async () => {
  try {
    await navigator.clipboard.writeText(rawLogs.value);
    ElMessage.success('日志已复制到剪贴板！');
  } catch (e) {
    ElMessage.error('复制失败');
  }
};

onMounted(() => {
  fetchLogs();
});

onUnmounted(() => {
  if (timer) clearInterval(timer);
});
</script>

<style scoped>
.log-viewer-view {
  padding: 24px 28px 28px;
  height: 100%;
  display: flex;
  flex-direction: column;
  box-sizing: border-box;
}

.log-toolbar {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-bottom: 16px;
  gap: 16px;
  padding: 12px 14px;
  flex-wrap: wrap;
}

.controls-left, .controls-right {
  display: flex;
  align-items: center;
  gap: 12px;
}

.terminal-container {
  flex: 1;
  min-height: 0;
  background-color: #0b0f17;
  border: 1px solid var(--border-color);
  border-radius: var(--radius-md);
  display: flex;
  flex-direction: column;
  overflow: hidden;
  box-shadow: 0 10px 30px rgba(0, 0, 0, 0.3);
}

.terminal-bar {
  height: 34px;
  background-color: #121824;
  border-bottom: 1px solid var(--border-color);
  display: flex;
  align-items: center;
  padding: 0 14px;
  gap: 8px;
}

.dot {
  width: 10px;
  height: 10px;
  border-radius: 50%;
}

.dot.red { background-color: #ef4444; }
.dot.yellow { background-color: #f59e0b; }
.dot.green { background-color: #10b981; }

.bar-title {
  font-size: 12px;
  color: var(--text-muted);
  font-family: monospace;
  margin-left: 8px;
}

.terminal-body {
  flex: 1;
  padding: 16px;
  overflow-y: auto;
  font-family: "JetBrains Mono", Menlo, Consolas, monospace;
  font-size: 12px;
  line-height: 1.5;
  color: #cbd5e1;
}

pre {
  margin: 0;
  white-space: pre-wrap;
  word-break: break-all;
}

.empty-tip {
  color: #64748b;
  text-align: center;
  margin-top: 40px;
}
</style>
