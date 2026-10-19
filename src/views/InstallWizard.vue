<template>
  <div class="page-view wizard-container">
    <div class="page-header">
      <div>
        <h2 class="page-title">离线部署向导</h2>
        <p class="page-subtitle">按步骤完成目标环境配置、镜像导入、容器启动与数据库迁移</p>
      </div>
    </div>

    <div class="steps-header card-surface">
      <div class="stepper">
        <template v-for="(step, index) in steps" :key="step">
          <div class="step" :class="{ active: activeStep === index, done: activeStep > index }">
            <span class="step-node">
              <el-icon v-if="activeStep > index"><Check /></el-icon>
              <span v-else>{{ index + 1 }}</span>
            </span>
            <span class="step-label">{{ step }}</span>
          </div>
          <span
            v-if="index < steps.length - 1"
            class="step-connect"
            :class="{ done: activeStep > index }"
          ></span>
        </template>
      </div>
    </div>

    <div class="step-content">
      <!-- 步骤 1: 模式配置 -->
      <div v-if="activeStep === 0" class="step-card">
        <h3>部署架构与目标选择</h3>
        <p class="desc">请选择是在目标服务器本机直接运行离线安装，还是通过工程师电脑远程推送到内网主机。</p>

        <div class="mode-selector">
          <div
            class="mode-box"
            :class="{ active: config.mode === 'local' }"
            @click="config.mode = 'local'"
          >
            <span class="icon-tile mode-icon">
              <el-icon><Monitor /></el-icon>
            </span>
            <div class="name">现场服务器本地直接安装</div>
            <div class="detail">现场将一体包解压在目标服务器直接双击安装，无需网络与开发环境</div>
          </div>

          <div
            class="mode-box"
            :class="{ active: config.mode === 'remote' }"
            @click="config.mode = 'remote'"
          >
            <span class="icon-tile mode-icon">
              <el-icon><Connection /></el-icon>
            </span>
            <div class="name">局域网远程推送部署 (SSH)</div>
            <div class="detail">在维护工作站上通过局域网 (如 192.0.2.10) 远程自动化推送并拉起集群</div>
          </div>
        </div>

        <div v-if="config.mode === 'local'" class="form-section">
          <el-form label-width="120px">
            <el-form-item label="离线安装包路径">
              <el-input v-model="config.bundleDir" placeholder="如: /opt/deploy/bundle 或当前目录" />
            </el-form-item>
            <el-form-item label="服务部署目录">
              <el-input v-model="config.deployDir" placeholder="默认: /opt/deploy" />
            </el-form-item>
          </el-form>
        </div>

        <div v-else class="form-section">
          <el-form label-width="120px">
            <el-row :gutter="20">
              <el-col :span="16">
                <el-form-item label="目标主机 IP">
                  <el-input v-model="config.remoteHost" placeholder="例如: 192.0.2.10" />
                </el-form-item>
              </el-col>
              <el-col :span="8">
                <el-form-item label="SSH 端口" label-width="88px">
                  <el-input-number v-model="config.remotePort" :min="1" :max="65535" style="width: 100%" />
                </el-form-item>
              </el-col>
            </el-row>
            <el-row :gutter="20">
              <el-col :span="12">
                <el-form-item label="登录用户名">
                  <el-input v-model="config.remoteUser" placeholder="默认: root" />
                </el-form-item>
              </el-col>
              <el-col :span="12">
                <el-form-item label="初始密码 (选填)">
                  <el-input v-model="config.remotePassword" type="password" show-password placeholder="仅用于首次下发免密密钥" />
                </el-form-item>
              </el-col>
            </el-row>
            <el-form-item label="本地私钥路径">
              <el-input v-model="config.remoteKey" placeholder="留空自动使用 ~/.ssh/id_ed25519_<host>" />
            </el-form-item>
            <el-form-item>
              <div class="ssh-actions">
                <el-button type="primary" :loading="testingSsh" @click="handleTestSsh">
                  测试 SSH 连通性
                </el-button>
                <el-button type="success" :loading="settingKey" @click="handleSetupKey">
                  一键配置并下发免密 SSH 密钥
                </el-button>
                <span v-if="sshMsg" :class="sshOk ? 'text-success' : 'text-danger'">
                  {{ sshMsg }}
                </span>
              </div>
            </el-form-item>
          </el-form>
        </div>

        <div class="step-footer">
          <div></div>
          <el-button type="primary" size="large" @click="nextStep">
            <span>下一步：环境自检</span>
            <el-icon class="el-icon--right"><ArrowRight /></el-icon>
          </el-button>
        </div>
      </div>

      <!-- 步骤 2: 环境与 Docker 自检 -->
      <div v-if="activeStep === 1" class="step-card">
        <h3>系统资源与运行环境检测</h3>
        <p class="desc">确保宿主机满足 OceanBase 及全栈服务的运行底线要求，并确认 Docker 状态。</p>

        <el-alert
          v-if="envError"
          :title="envError"
          type="error"
          :closable="false"
          show-icon
          style="margin-bottom: 16px;"
        />
        <el-alert
          v-else-if="envData && envData.issues && envData.issues.length"
          :title="envData.issues.join('；')"
          type="warning"
          :closable="false"
          show-icon
          style="margin-bottom: 16px;"
        />

        <div class="env-metrics" v-if="envData">
          <div class="metric-card">
            <span class="label">操作系统/架构</span>
            <span class="value">{{ envData.os_release }} ({{ envData.arch }})</span>
          </div>
          <div class="metric-card">
            <span class="label">CPU 核心数</span>
            <span class="value">{{ envData.cpu_cores }} 核</span>
          </div>
          <div class="metric-card">
            <span class="label">物理内存</span>
            <span class="value">{{ envData.total_memory_mb }} MB</span>
          </div>
          <div class="metric-card">
            <span class="label">磁盘剩余空间</span>
            <span class="value">{{ envData.disk_available_gb }} GB</span>
          </div>
        </div>

        <div class="docker-status-box" v-if="envData">
          <div class="header-line">
            <span class="title">Docker 引擎状态：</span>
            <el-tag v-if="envData.docker_running" type="success">正常运行中 ({{ envData.docker_version }})</el-tag>
            <el-tag v-else-if="envData.docker_installed" type="warning">已安装但未启动</el-tag>
            <el-tag v-else type="danger">未安装 Docker 引擎</el-tag>

            <el-button
              v-if="!envData.docker_running"
              type="primary"
              size="small"
              :loading="installingDocker"
              @click="handleInstallDocker"
              style="margin-left: auto;"
            >
              一键离线静默安装 Docker 引擎
            </el-button>
          </div>

          <div v-if="dockerSteps.length > 0" class="docker-steps-log">
            <div v-for="(s, i) in dockerSteps" :key="i" class="step-item" :class="s.status">
              <el-icon
                class="step-status"
                :class="[s.status, { 'is-loading': s.status !== 'success' && s.status !== 'failed' }]"
              >
                <CircleCheck v-if="s.status === 'success'" />
                <CircleClose v-else-if="s.status === 'failed'" />
                <Loading v-else />
              </el-icon>
              <span class="name">{{ s.step }}:</span>
              <span class="msg">{{ s.message }}</span>
            </div>
          </div>
        </div>

        <div class="ports-list" v-if="envData">
          <h4>核心端口占用排查 (8000/8001/18080/12881/6390/9092/20000)</h4>
          <el-table :data="envData.ports" size="small" border>
            <el-table-column prop="port" label="端口" width="90" />
            <el-table-column prop="protocol" label="协议" width="80" />
            <el-table-column prop="service" label="对应服务组件" />
            <el-table-column label="状态" width="120">
              <template #default="{ row }">
                <el-tag :type="row.available ? 'success' : 'warning'" size="small">
                  {{ row.available ? '空闲' : '占用中' }}
                </el-tag>
              </template>
            </el-table-column>
            <el-table-column prop="occupying_process" label="占用详情" />
          </el-table>
        </div>

        <div class="step-footer">
          <el-button @click="prevStep">上一步</el-button>
          <el-button type="primary" size="large" @click="nextStep">
            <span>下一步：加载镜像</span>
            <el-icon class="el-icon--right"><ArrowRight /></el-icon>
          </el-button>
        </div>
      </div>

      <!-- 步骤 3: 镜像加载与产物分发 -->
      <div v-if="activeStep === 2" class="step-card">
        <h3>加载离线 Docker 镜像与分发应用产物</h3>
        <p class="desc">批量加载 {{ profile.images.length }} 个离线镜像包，导入顺序由镜像清单决定。</p>
        <p class="desc">
          镜像包目录：
          <code>{{ bundleImagesPath }}</code>
          <el-button link type="primary" @click="emit('go-images')">打开镜像包管理</el-button>
        </p>

        <div class="action-box">
          <el-button type="primary" :loading="loadingImages" @click="handleLoadImages">
            开始批量导入离线镜像包
          </el-button>
          <span class="tip" v-if="imageMsg">{{ imageMsg }}</span>
        </div>

        <div class="progress-box" v-if="imageProgress">
          <el-progress
            :percentage="Math.round((imageProgress.current / imageProgress.total) * 100)"
            :status="imageProgress.finished ? 'success' : ''"
          />
          <div class="prog-text">{{ imageProgress.message }}</div>
        </div>

        <div class="images-list">
          <h4>预计载入镜像列表</h4>
          <ul>
            <li v-for="image in profile.images" :key="image.file">
              <code>{{ image.imageRef }}</code>
            </li>
          </ul>
        </div>

        <div class="step-footer">
          <el-button @click="prevStep">上一步</el-button>
          <el-button type="primary" size="large" @click="nextStep">
            <span>下一步：启动容器全栈</span>
            <el-icon class="el-icon--right"><ArrowRight /></el-icon>
          </el-button>
        </div>
      </div>

      <!-- 步骤 4: 拉起容器集群 -->
      <div v-if="activeStep === 3" class="step-card">
        <h3>启动 {{ profile.containers.length }} 个服务容器</h3>
        <p class="desc">根据 <code>docker-compose.lan.yml</code> 启动容器全栈。</p>

        <div class="action-box">
          <el-button type="primary" size="large" :loading="startingContainers" @click="handleStartStack">
            一键拉起全栈容器 (docker compose up -d)
          </el-button>
        </div>

        <div v-if="composeLog" class="terminal-log">
          <pre>{{ composeLog }}</pre>
        </div>

        <div class="step-footer">
          <el-button @click="prevStep">上一步</el-button>
          <el-button type="primary" size="large" @click="nextStep">
            <span>下一步：数据库迁移</span>
            <el-icon class="el-icon--right"><ArrowRight /></el-icon>
          </el-button>
        </div>
      </div>

      <!-- 步骤 5: 数据库初始化与迁移 -->
      <div v-if="activeStep === 4" class="step-card">
        <h3>OceanBase 数据库初始化与全量数据迁移</h3>
        <p class="desc">自动探测 OceanBase 健康状态，并导入业务初始化表结构与全量 dump 数据。</p>

        <div class="ob-probe-box">
          <div class="status-row">
            <span>OceanBase 就绪状态：</span>
            <el-tag :type="obReady ? 'success' : 'warning'">
              {{ obMsg || '探测中...' }}
            </el-tag>
            <el-button size="small" plain @click="handleCheckOb">重新探测</el-button>
          </div>
        </div>

        <div class="action-box">
          <el-form label-width="120px">
            <el-form-item label="SQL 脚本路径">
              <el-input v-model="sqlPath" placeholder="如: /opt/deploy/bundle/database/01-database-dump.sql" />
            </el-form-item>
            <el-form-item>
              <el-button type="primary" :loading="migratingDb" @click="handleMigrateDb">
                执行数据库全量迁移导入
              </el-button>
            </el-form-item>
          </el-form>
        </div>

        <div v-if="dbResult" class="migration-summary" :class="dbResult.success ? 'success' : 'fail'">
          <div class="sum-title">{{ dbResult.message }}</div>
          <div v-if="dbResult.success" class="sum-details">
            <div class="sum-line">
              <el-icon><CircleCheck /></el-icon>
              <span>已验证表数量 <strong>{{ dbResult.table_count }}</strong> 张</span>
            </div>
            <div class="sum-line">
              <el-icon><CircleCheck /></el-icon>
              <span>已验证字典记录 <strong>{{ dbResult.dictionary_count }}</strong> 条</span>
            </div>
            <div class="sum-line">
              <el-icon><CircleCheck /></el-icon>
              <span>已验证系统用户 <strong>{{ dbResult.user_count }}</strong> 个</span>
            </div>
          </div>
          <div v-if="dbResult.error" class="sum-error">
            错误详情: {{ dbResult.error }}
          </div>
        </div>

        <div class="step-footer">
          <el-button @click="prevStep">上一步</el-button>
          <el-button type="primary" size="large" @click="nextStep">
            <span>下一步：完成验收</span>
            <el-icon class="el-icon--right"><ArrowRight /></el-icon>
          </el-button>
        </div>
      </div>

      <!-- 步骤 6: 完成验收 -->
      <div v-if="activeStep === 5" class="step-card text-center">
        <div class="done-mark">
          <el-icon :size="34"><CircleCheckFilled /></el-icon>
        </div>
        <h2>离线部署完成</h2>
        <p class="desc">所有核心容器服务已就绪，数据库数据迁移一致性校验已通过。</p>

        <div class="access-urls">
          <div class="url-card">
            <div class="title">Web 前端访问入口</div>
            <div class="url">http://{{ config.mode === 'remote' ? config.remoteHost : '127.0.0.1' }}:18080</div>
            <div class="note">Nginx 反向代理 + Vue 3 控制台</div>
          </div>

          <div class="url-card">
            <div class="title">后端 Spring Boot API</div>
            <div class="url">http://{{ config.mode === 'remote' ? config.remoteHost : '127.0.0.1' }}:8000 (主) / :8001 (从)</div>
            <div class="note">Java 21 Fat Jar 业务层</div>
          </div>

          <div class="url-card">
            <div class="title">设备通信端口</div>
            <div class="url">{{ config.mode === 'remote' ? config.remoteHost : '127.0.0.1' }}:20000 / UDP</div>
            <div class="note">设备网关 UDP 负载均衡接入</div>
          </div>
        </div>

        <div class="step-footer center">
          <el-button type="primary" size="large" @click="$emit('go-dashboard')">
            <span>进入集群监控看板</span>
            <el-icon class="el-icon--right"><DataBoard /></el-icon>
          </el-button>
        </div>
      </div>
    </div>
  </div>
</template>

<script setup>
import { computed, ref, onMounted } from 'vue';
import {
  ArrowRight,
  Check,
  CircleCheck,
  CircleCheckFilled,
  CircleClose,
  Connection,
  DataBoard,
  Loading,
  Monitor
} from '@element-plus/icons-vue';
import {
  checkLocalEnv,
  checkRemoteEnv,
  installOfflineDockerEngine,
  installRemoteDockerEngine,
  loadOfflineImages,
  loadRemoteImages,
  dockerComposeAction,
  remoteDockerComposeAction,
  checkOceanbaseStatus,
  checkRemoteOceanbaseStatus,
  importOceanbaseSql,
  importRemoteOceanbaseSql,
  testSshConnection,
  setupRemoteSshKey,
  onImageLoadProgress
} from '../api/tauri.js';
import profile from '../../profile.json';

const props = defineProps({
  connection: {
    type: Object,
    required: true
  }
});

const emit = defineEmits(['go-dashboard', 'go-images']);

const activeStep = ref(0);

const steps = ['模式配置', '环境检测', '镜像导入', '启动集群', '数据迁移', '完成验收'];

const config = props.connection;

const bundleImagesPath = computed(() => `${String(config.bundleDir || '').replace(/\/+$/, '')}/images`);

// SSH 测试
const testingSsh = ref(false);
const settingKey = ref(false);
const sshMsg = ref('');
const sshOk = ref(false);

// 环境与 Docker
const envData = ref(null);
const envError = ref('');
const installingDocker = ref(false);
const dockerSteps = ref([]);

// 镜像
const loadingImages = ref(false);
const imageMsg = ref('');
const imageProgress = ref(null);

// 启动
const startingContainers = ref(false);
const composeLog = ref('');

// 数据库
const obReady = ref(false);
const obMsg = ref('');
const sqlPath = ref('/tmp/database-dump.sql');
const migratingDb = ref(false);
const dbResult = ref(null);

const nextStep = async () => {
  if (activeStep.value === 0) {
    await loadEnvData();
  } else if (activeStep.value === 3) {
    await handleCheckOb();
  }
  activeStep.value++;
};

const prevStep = () => {
  if (activeStep.value > 0) activeStep.value--;
};

const handleTestSsh = async () => {
  testingSsh.value = true;
  sshMsg.value = '';
  try {
    const res = await testSshConnection(config.remoteHost, config.remotePort, config.remoteUser, config.remoteKey || null);
    sshOk.value = res.success;
    sshMsg.value = res.message;
  } catch (e) {
    sshOk.value = false;
    sshMsg.value = `测试失败: ${e}`;
  } finally {
    testingSsh.value = false;
  }
};

const handleSetupKey = async () => {
  settingKey.value = true;
  sshMsg.value = '';
  try {
    const res = await setupRemoteSshKey(
      config.remoteHost,
      config.remotePort,
      config.remoteUser,
      config.remotePassword || null,
      config.remoteKey || null
    );
    sshOk.value = res.success;
    sshMsg.value = res.message;
  } catch (e) {
    sshOk.value = false;
    sshMsg.value = `配置失败: ${e}`;
  } finally {
    settingKey.value = false;
  }
};

const loadEnvData = async () => {
  envError.value = '';
  envData.value = null;
  try {
    if (config.mode === 'remote') {
      envData.value = await checkRemoteEnv(
        config.remoteHost,
        config.remotePort,
        config.remoteUser,
        config.remoteKey || null
      );
    } else {
      envData.value = await checkLocalEnv(config.deployDir);
    }
  } catch (e) {
    console.error(e);
    envError.value = String(e);
  }
};

const handleInstallDocker = async () => {
  installingDocker.value = true;
  try {
    const res = config.mode === 'remote'
      ? await installRemoteDockerEngine(
          config.remoteHost,
          config.remotePort,
          config.remoteUser,
          config.remoteKey || null,
          config.bundleDir
        )
      : await installOfflineDockerEngine(config.bundleDir);
    dockerSteps.value = res.steps;
    await loadEnvData();
    if (!res.success && res.error_message) {
      envError.value = res.error_message;
    }
  } catch (e) {
    dockerSteps.value = [{ step: '错误', status: 'failed', message: String(e) }];
  } finally {
    installingDocker.value = false;
  }
};

const handleLoadImages = async () => {
  loadingImages.value = true;
  imageMsg.value = '正在加载离线镜像...';
  try {
    const res = config.mode === 'remote'
      ? await loadRemoteImages(
          config.remoteHost,
          config.remotePort,
          config.remoteUser,
          config.remoteKey || null,
          config.bundleDir
        )
      : await loadOfflineImages(config.bundleDir);
    imageMsg.value = `成功载入 ${res.length} 个镜像文件！`;
  } catch (e) {
    imageMsg.value = `导入错误: ${e}。可先打开“镜像包管理”扫描本地镜像包并上传到目标机，再回到本步重试。`;
  } finally {
    loadingImages.value = false;
  }
};

const handleStartStack = async () => {
  startingContainers.value = true;
  composeLog.value = '执行 docker compose -f docker-compose.lan.yml up -d ...';
  try {
    const res = config.mode === 'remote'
      ? await remoteDockerComposeAction(
          config.remoteHost,
          config.remotePort,
          config.remoteUser,
          config.remoteKey || null,
          config.deployDir,
          'up'
        )
      : await dockerComposeAction(config.deployDir, 'up');
    composeLog.value = res.output || (res.success ? '容器栈启动完成' : '容器栈启动失败');
    if (!res.success && res.error) {
      composeLog.value += `\n${res.error}`;
    }
  } catch (e) {
    composeLog.value = `启动出错: ${e}`;
  } finally {
    startingContainers.value = false;
  }
};

const handleCheckOb = async () => {
  try {
    const res = config.mode === 'remote'
      ? await checkRemoteOceanbaseStatus(
          config.remoteHost,
          config.remotePort,
          config.remoteUser,
          config.remoteKey || null
        )
      : await checkOceanbaseStatus();
    obReady.value = res.is_ready;
    obMsg.value = res.message;
  } catch (e) {
    obReady.value = false;
    obMsg.value = String(e);
  }
};

const handleMigrateDb = async () => {
  migratingDb.value = true;
  try {
    dbResult.value = config.mode === 'remote'
      ? await importRemoteOceanbaseSql(
          config.remoteHost,
          config.remotePort,
          config.remoteUser,
          config.remoteKey || null,
          sqlPath.value
        )
      : await importOceanbaseSql(sqlPath.value);
  } catch (e) {
    dbResult.value = {
      success: false,
      table_count: 0,
      dictionary_count: 0,
      user_count: 0,
      tables: [],
      message: '导入失败',
      error: String(e)
    };
  } finally {
    migratingDb.value = false;
  }
};

onMounted(() => {
  onImageLoadProgress((prog) => {
    imageProgress.value = prog;
  });
});
</script>

<style scoped>
.wizard-container {
  display: block;
}

.steps-header {
  margin-bottom: 24px;
  padding: 14px 16px;
}

.stepper {
  display: flex;
  align-items: center;
  width: 100%;
  overflow-x: auto;
  scrollbar-width: none;
}

.stepper::-webkit-scrollbar {
  display: none;
}

.step {
  display: flex;
  align-items: center;
  gap: 8px;
  flex: none;
  color: var(--text-muted);
}

.step-node {
  width: 26px;
  height: 26px;
  border-radius: 50%;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  flex: none;
  border: 1px solid var(--border-strong);
  background: rgba(255, 255, 255, 0.025);
  font-size: 11px;
  font-weight: 600;
  transition: all 0.2s ease;
}

.step-label {
  font-size: 12px;
  white-space: nowrap;
}

.step.active {
  color: var(--text-main);
}

.step.active .step-node {
  color: #fff;
  border-color: var(--primary-color);
  background: var(--primary-color);
  box-shadow: 0 0 0 4px rgba(79, 140, 255, 0.12);
}

.step.active .step-label {
  font-weight: 600;
}

.step.done .step-node {
  color: var(--success-color);
  border-color: rgba(53, 196, 143, 0.36);
  background: rgba(53, 196, 143, 0.12);
}

.step-connect {
  height: 1px;
  flex: 1;
  min-width: 16px;
  margin: 0 10px;
  background: var(--border-color);
}

.step-connect.done {
  background: rgba(53, 196, 143, 0.4);
}

.step-card {
  background-color: var(--panel-bg);
  border: 1px solid var(--border-color);
  border-radius: var(--radius-md);
  box-shadow: var(--shadow-card);
  padding: 28px;
}

.step-card h3 {
  font-size: 18px;
  color: #fff;
  margin-bottom: 6px;
}

.step-card .desc {
  font-size: 13px;
  color: var(--text-muted);
  margin-bottom: 20px;
}

.mode-selector {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 20px;
  margin-bottom: 24px;
}

.mode-box {
  background-color: var(--card-bg);
  border: 2px solid var(--border-color);
  border-radius: 10px;
  padding: 20px;
  cursor: pointer;
  transition: all 0.2s ease;
}

.mode-box:hover {
  border-color: #3b82f6;
  background-color: rgba(59, 130, 246, 0.05);
}

.mode-box.active {
  border-color: #3b82f6;
  background-color: rgba(59, 130, 246, 0.12);
}

.mode-box .mode-icon {
  margin-bottom: 14px;
}

.mode-box .name {
  font-size: 15px;
  font-weight: 600;
  color: #fff;
  margin-bottom: 6px;
}

.mode-box .detail {
  font-size: 12px;
  color: var(--text-muted);
  line-height: 1.4;
}

.form-section {
  background-color: rgba(0, 0, 0, 0.2);
  padding: 20px;
  border-radius: 8px;
  border: 1px solid rgba(255, 255, 255, 0.05);
  margin-bottom: 24px;
}

.ssh-actions {
  display: flex;
  align-items: center;
  gap: 12px;
}

.text-success {
  color: #34d399;
  font-size: 13px;
}

.text-danger {
  color: #f87171;
  font-size: 13px;
}

.step-footer {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-top: 24px;
  border-top: 1px solid var(--border-color);
  padding-top: 20px;
}

.step-footer.center {
  justify-content: center;
}

.env-metrics {
  display: grid;
  grid-template-columns: repeat(4, 1fr);
  gap: 16px;
  margin-bottom: 20px;
}

.metric-card {
  background-color: var(--card-bg);
  border: 1px solid var(--border-color);
  padding: 14px;
  border-radius: 8px;
  display: flex;
  flex-direction: column;
}

.metric-card .label {
  font-size: 11px;
  color: var(--text-muted);
}

.metric-card .value {
  font-size: 14px;
  font-weight: 600;
  color: #fff;
  margin-top: 4px;
}

.docker-status-box {
  background-color: var(--card-bg);
  border: 1px solid var(--border-color);
  padding: 16px;
  border-radius: 8px;
  margin-bottom: 20px;
}

.docker-status-box .header-line {
  display: flex;
  align-items: center;
  gap: 12px;
}

.docker-steps-log {
  margin-top: 14px;
  padding: 12px 14px;
  background-color: #0b0f17;
  border-radius: var(--radius-sm);
  font-family: monospace;
  font-size: 12px;
}

.step-item {
  display: flex;
  align-items: flex-start;
  gap: 8px;
  margin-bottom: 6px;
}

.step-status {
  margin-top: 2px;
  flex: none;
}

.step-status.success { color: var(--success-color); }
.step-status.failed { color: var(--danger-color); }
.step-status.running { color: var(--primary-color); }

.ports-list h4, .images-list h4 {
  font-size: 13px;
  color: #cbd5e1;
  margin-bottom: 10px;
}

.action-box {
  margin-bottom: 20px;
  display: flex;
  align-items: center;
  gap: 16px;
}

.progress-box {
  background-color: #0b0f17;
  padding: 16px;
  border-radius: 8px;
  margin-bottom: 20px;
}

.prog-text {
  font-size: 12px;
  color: #94a3b8;
  margin-top: 8px;
  font-family: monospace;
}

.images-list ul {
  list-style: none;
  background-color: #0b0f17;
  padding: 14px 18px;
  border-radius: 8px;
}

.images-list li {
  font-size: 13px;
  color: #94a3b8;
  margin-bottom: 8px;
}

.images-list code {
  color: #60a5fa;
}

.terminal-log {
  background-color: #090d14;
  border: 1px solid #1e293b;
  border-radius: 8px;
  padding: 14px;
  max-height: 240px;
  overflow-y: auto;
  font-family: monospace;
  font-size: 12px;
  color: #e2e8f0;
}

.ob-probe-box {
  background-color: var(--card-bg);
  border: 1px solid var(--border-color);
  padding: 14px;
  border-radius: 8px;
  margin-bottom: 20px;
}

.status-row {
  display: flex;
  align-items: center;
  gap: 12px;
}

.migration-summary {
  padding: 16px;
  border-radius: 8px;
  margin-top: 16px;
}

.migration-summary.success {
  background-color: rgba(16, 185, 129, 0.1);
  border: 1px solid rgba(16, 185, 129, 0.3);
}

.migration-summary.fail {
  background-color: rgba(239, 68, 68, 0.1);
  border: 1px solid rgba(239, 68, 68, 0.3);
}

.sum-title {
  font-size: 14px;
  font-weight: 600;
  margin-bottom: 8px;
}

.sum-details {
  font-size: 13px;
  color: #a7f3d0;
  display: flex;
  flex-direction: column;
  gap: 7px;
}

.sum-line {
  display: flex;
  align-items: center;
  gap: 7px;
}

.sum-line .el-icon {
  flex: none;
}

.done-mark {
  width: 72px;
  height: 72px;
  margin: 0 auto 18px;
  border-radius: 50%;
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--success-color);
  background: rgba(53, 196, 143, 0.12);
  border: 1px solid rgba(53, 196, 143, 0.24);
}

.text-center {
  text-align: center;
}

.access-urls {
  display: grid;
  grid-template-columns: repeat(3, 1fr);
  gap: 16px;
  margin: 32px 0;
  text-align: left;
}

.url-card {
  background-color: var(--card-bg);
  border: 1px solid var(--border-color);
  border-radius: 8px;
  padding: 16px;
}

.url-card .title {
  font-size: 12px;
  color: var(--text-muted);
}

.url-card .url {
  font-size: 14px;
  font-weight: 600;
  color: #60a5fa;
  margin: 8px 0 4px;
  font-family: monospace;
}

.url-card .note {
  font-size: 11px;
  color: #64748b;
}
</style>
