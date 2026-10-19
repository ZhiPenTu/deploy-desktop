import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import profile from "../../profile.json";

const MOCK_PORTS = profile.containers
  .filter((container) => container.port)
  .map((container) => ({
    port: container.port,
    protocol: container.protocol,
    service: `${container.serviceKey} (${container.label})`,
    available: true,
    occupying_process: null
  }));

const MOCK_CONTAINERS = profile.containers.map((container, index) => ({
  name: `${profile.containerPrefix}-${container.serviceKey}`,
  service_key: container.serviceKey,
  state: "running",
  health: container.serviceKey === "frontend" ? "none" : "healthy",
  status_text: container.serviceKey === "frontend" ? "Up 2 hours" : "Up 2 hours (healthy)",
  ports: container.port ? `0.0.0.0:${container.port}->${container.port}/${container.protocol.toLowerCase()}` : "",
  image: profile.images[index % profile.images.length].imageRef
}));

export async function checkLocalEnv(targetDir = null) {
  if (!isTauri()) {
    console.warn("Running in browser preview mode, returning mock data");
    return {
      os: "linux",
      arch: "x86_64",
      os_release: "Rocky Linux 9.4 (模拟演示)",
      cpu_cores: 8,
      total_memory_mb: 16384,
      available_memory_mb: 12288,
      disk_available_gb: 85.5,
      docker_installed: true,
      docker_version: "Docker version 27.1.1, build 6312585",
      docker_running: true,
      compose_installed: true,
      compose_version: "Docker Compose version v2.29.1",
      ports: MOCK_PORTS,
      is_ready: true,
      issues: []
    };
  }
  return await invoke("check_local_env", { targetDir });
}

export async function installOfflineDockerEngine(bundleDir) {
  if (!isTauri()) {
    return {
      success: true,
      steps: [
        { step: "1. 扫描 Docker 离线静态二进制包", status: "success", message: "找到离线安装包 (模拟)" },
        { step: "2. 安装 Docker 核心二进制文件", status: "success", message: "部署至 /usr/bin/ 完成" },
        { step: "3. 配置 systemd 服务与 Daemon 参数", status: "success", message: "systemd 单元配置就绪" },
        { step: "4. 启动 Docker 服务并验证运行状态", status: "success", message: "Docker 服务运行正常" }
      ],
      docker_version: "Docker version 27.1.1 (离线安装模拟)",
      error_message: null
    };
  }
  return await invoke("install_offline_docker_engine", { bundleDir });
}

export async function loadOfflineImages(bundleDir) {
  if (!isTauri()) {
    return profile.images.slice(0, 3).map((image) => `${image.file}: Loaded image`);
  }
  return await invoke("load_offline_images", { bundleDir });
}

const MOCK_IMAGE_SIZES = [1450, 620, 130, 320, 75, 95];

const MOCK_IMAGE_FILES = profile.images.map((image, index) => ({
  name: image.file,
  size_bytes: (MOCK_IMAGE_SIZES[index] || 100) * 1024 * 1024,
  modified_ms: 1760000000000,
  image_ref: image.imageRef
}));

const MOCK_EXPECTATIONS = MOCK_IMAGE_FILES.map((file) => ({
  file_name: file.name,
  image_ref: file.image_ref,
  file_present: true
}));

export async function scanLocalImageBundle(dir) {
  if (!isTauri()) {
    return {
      input_dir: dir,
      images_dir: `${dir}/images`,
      exists: true,
      files: MOCK_IMAGE_FILES,
      total_bytes: MOCK_IMAGE_FILES.reduce((sum, file) => sum + file.size_bytes, 0),
      expectations: MOCK_EXPECTATIONS,
      message: `共 ${MOCK_IMAGE_FILES.length} 个镜像包文件 (模拟)`
    };
  }
  return await invoke("scan_local_image_bundle", { dir });
}

export async function inspectRemoteImageBundle(host, port, user, keyPath, bundleDir) {
  if (!isTauri()) {
    return {
      bundle_dir: bundleDir,
      images_dir: `${bundleDir}/images`,
      exists: true,
      files: MOCK_IMAGE_FILES.slice(0, 4),
      total_bytes: MOCK_IMAGE_FILES.slice(0, 4).reduce((sum, file) => sum + file.size_bytes, 0),
      docker_available: true,
      docker_running: true,
      docker_images: MOCK_IMAGE_FILES.slice(0, 4).map((file) => file.image_ref),
      expectations: MOCK_EXPECTATIONS,
      missing_images: MOCK_IMAGE_FILES.slice(4).map((file) => file.image_ref),
      free_kb: 24 * 1024 * 1024,
      issues: ["2 个镜像尚未导入 Docker (模拟)"]
    };
  }
  return await invoke("inspect_remote_image_bundle", { host, port, user, keyPath, bundleDir });
}

export async function uploadRemoteImageBundle(host, port, user, keyPath, localDir, remoteBundleDir, files, overwrite = false) {
  if (!isTauri()) {
    return files.map((file) => `${file} 上传完成 (模拟)`);
  }
  return await invoke("upload_remote_image_bundle", {
    host,
    port,
    user,
    keyPath,
    localDir,
    remoteBundleDir,
    files,
    overwrite
  });
}

export async function importRemoteImageBundle(host, port, user, keyPath, remoteBundleDir, files) {
  if (!isTauri()) {
    return files.map((file) => `${file}: Loaded image (模拟)`);
  }
  return await invoke("import_remote_image_bundle", {
    host,
    port,
    user,
    keyPath,
    remoteBundleDir,
    files
  });
}

export async function removeRemoteImageFiles(host, port, user, keyPath, remoteBundleDir, files) {
  if (!isTauri()) {
    return `已删除目标机镜像包: ${files.join("、")} (模拟)`;
  }
  return await invoke("remove_remote_image_files", {
    host,
    port,
    user,
    keyPath,
    remoteBundleDir,
    files
  });
}

export async function getContainersStatus() {
  if (!isTauri()) {
    return MOCK_CONTAINERS;
  }
  return await invoke("get_containers_status");
}

export async function dockerComposeAction(deployDir, action, service = null) {
  if (!isTauri()) {
    return { success: true, output: `模拟执行 docker compose ${action} ${service || ""}`, error: null };
  }
  return await invoke("docker_compose_action", { deployDir, action, service });
}

export async function getServiceLogs(containerName, tail = 200) {
  if (!isTauri()) {
    return `[2026-10-09 15:00:00] [INFO] ${containerName} 运行正常 (模拟日志)\n[2026-10-09 15:00:05] [INFO] Heartbeat check passed\n[2026-10-09 15:00:10] [INFO] Ready for connections.`;
  }
  return await invoke("get_service_logs", { containerName, tail });
}

export async function checkOceanbaseStatus() {
  if (!isTauri()) {
    return { is_running: true, is_ready: true, container_health: "healthy", message: "OceanBase 连接就绪 (模拟)" };
  }
  return await invoke("check_oceanbase_status");
}

export async function importOceanbaseSql(sqlFilePath) {
  if (!isTauri()) {
    return {
      success: true,
      table_count: 43,
      dictionary_count: 2264,
      user_count: 4,
      tables: ["alarm_records", "devices", "parameter_dict", "users"],
      message: "数据库迁移完成！成功同步 43 张数据表、2264 条字典数据与 4 个系统用户 (模拟)",
      error: null
    };
  }
  return await invoke("import_oceanbase_sql", { sqlFilePath });
}

export async function testSshConnection(host, port, user, keyPath = null) {
  if (!isTauri()) {
    return { success: true, host, user, message: "SSH 连接测试成功 (模拟)", latency_ms: 2 };
  }
  return await invoke("test_ssh_connection", { host, port, user, keyPath });
}

export async function setupRemoteSshKey(host, port, user, password = null, keyPath = null) {
  if (!isTauri()) {
    return { success: true, host, user, message: "SSH 密钥自动配置成功 (模拟)", latency_ms: null };
  }
  return await invoke("setup_remote_ssh_key", { host, port, user, password, keyPath });
}

export async function checkRemoteEnv(host, port, user, keyPath = null) {
  if (!isTauri()) {
    return {
      success: true,
      os: "Linux",
      arch: "x86_64",
      os_release: "Rocky Linux 9.4 (模拟演示)",
      cpu_cores: 8,
      total_memory_mb: 32768,
      available_memory_mb: 24576,
      disk_available_gb: 120.0,
      docker_installed: true,
      docker_version: "Docker version 27.1.1",
      docker_running: true,
      compose_installed: true,
      compose_version: "Docker Compose version v2.29.1",
      ports: MOCK_PORTS,
      is_ready: true,
      raw_output: "",
      issues: []
    };
  }
  return await invoke("check_remote_env", { host, port, user, keyPath });
}

export async function installRemoteDockerEngine(host, port, user, keyPath, bundleDir) {
  if (!isTauri()) {
    return {
      success: true,
      steps: [
        { step: "1. 扫描 Docker 离线静态二进制包", status: "success", message: "找到离线资源目录 (模拟)" },
        { step: "2. 安装 Docker 核心二进制文件", status: "success", message: "远程二进制部署完成 (模拟)" },
        { step: "3. 配置 systemd 服务与 Daemon 参数", status: "success", message: "远程 systemd 配置完成 (模拟)" },
        { step: "4. 启动 Docker 服务并验证运行状态", status: "success", message: "远程 Docker 服务运行正常 (模拟)" }
      ],
      docker_version: "Docker version 27.1.1 (模拟)",
      error_message: null
    };
  }
  return await invoke("install_remote_docker_engine", { host, port, user, keyPath, bundleDir });
}

export async function loadRemoteImages(host, port, user, keyPath, bundleDir) {
  if (!isTauri()) {
    return ["oceanbase.tar: Loaded image", "kafka.tar: Loaded image", "redis.tar: Loaded image"];
  }
  return await invoke("load_remote_images", { host, port, user, keyPath, bundleDir });
}

export async function remoteDockerComposeAction(host, port, user, keyPath, deployDir, action = "up", service = null) {
  if (!isTauri()) {
    return { success: true, output: `远程 docker compose ${action} 完成 (模拟)`, error: null };
  }
  return await invoke("remote_docker_compose_action", { host, port, user, keyPath, deployDir, action, service });
}

export async function checkRemoteOceanbaseStatus(host, port, user, keyPath) {
  if (!isTauri()) {
    return { is_running: true, is_ready: true, container_health: "healthy", message: "OceanBase 连接就绪 (模拟)" };
  }
  return await invoke("check_remote_oceanbase_status", { host, port, user, keyPath });
}

export async function importRemoteOceanbaseSql(host, port, user, keyPath, sqlFilePath) {
  if (!isTauri()) {
    return {
      success: true,
      table_count: 43,
      dictionary_count: 2264,
      user_count: 4,
      tables: ["alarm_records", "devices", "parameter_dict", "users"],
      message: "数据库迁移完成！成功同步 43 张数据表、2264 条字典数据与 4 个系统用户 (模拟)",
      error: null
    };
  }
  return await invoke("import_remote_oceanbase_sql", { host, port, user, keyPath, sqlFilePath });
}

export async function runRemoteDeployAction(host, port, user, keyPath = null, mode = "status", targetService = null, deployDir = null) {
  if (!isTauri()) {
    return { success: true, output: `远程操作 ${mode} 完成 (模拟)`, error: null };
  }
  return await invoke("run_remote_deploy_action", { host, port, user, keyPath, mode, targetService, deployDir });
}

export async function inspectRemoteDeployment(host, port, user, keyPath, installDir) {
  if (!isTauri()) {
    return {
      deployed: true,
      install_dir: installDir,
      deploy_dir: `${installDir}/deploy`,
      compose_file: "docker-compose.yml",
      docker_available: true,
      docker_running: true,
      compose_available: true,
      server_jar: { exists: true, size_bytes: 75633137, modified: "2026-10-19 09:53:37" },
      gateway_classes: { exists: true, size_bytes: 0, modified: "2026-09-28 17:10:22" },
      gateway_lib: { exists: true, size_bytes: 0, modified: "2026-09-30 08:59:48" },
      frontend_dist: { exists: true, size_bytes: 0, modified: "2026-10-19 09:54:55" },
      containers: MOCK_CONTAINERS,
      running_containers: MOCK_CONTAINERS.length,
      total_containers: MOCK_CONTAINERS.length,
      issues: [],
      message: `已检测到部署目录 ${installDir}/deploy，${MOCK_CONTAINERS.length} / ${MOCK_CONTAINERS.length} 个容器正在运行。`
    };
  }
  return await invoke("inspect_remote_deployment", { host, port, user, keyPath, installDir });
}

export async function getRemoteContainersStatus(host, port, user, keyPath, installDir) {
  if (!isTauri()) {
    return (await inspectRemoteDeployment(host, port, user, keyPath, installDir)).containers;
  }
  return await invoke("get_remote_containers_status", { host, port, user, keyPath, installDir });
}

export async function getRemoteServiceLogs(host, port, user, keyPath, containerName, tail = 200) {
  if (!isTauri()) {
    return `[2026-10-19 10:00:00] [INFO] ${containerName} 远程日志 (模拟)\n[2026-10-19 10:00:05] [INFO] Heartbeat check passed\n[2026-10-19 10:00:10] [INFO] Ready for connections.`;
  }
  return await invoke("get_remote_service_logs", { host, port, user, keyPath, containerName, tail });
}

export async function updateRemoteDeployment(host, port, user, keyPath, bundleDir, installDir, components, backup, dryRun = false) {
  if (!isTauri()) {
    return {
      success: true,
      components,
      backup_dir: backup ? `${installDir}/backup/update-20261019-110000` : null,
      output: dryRun
        ? "预检通过：更新包与目标机状态检查通过。未上传文件，也未重启服务。"
        : "模拟更新完成：已上传产物、重启所选服务并通过健康检查。",
      duration_secs: dryRun ? 1 : 18,
      status: null
    };
  }
  return await invoke("update_remote_deployment", {
    host,
    port,
    user,
    keyPath,
    bundleDir,
    installDir,
    components,
    backup,
    dryRun
  });
}

export function onRemoteUpdateProgress(callback) {
  if (!isTauri()) return () => {};
  return listen("remote-update-progress", (event) => callback(event.payload));
}

export function onImageLoadProgress(callback) {
  if (!isTauri()) return () => {};
  return listen("image-load-progress", (event) => callback(event.payload));
}

export function onImageManagerProgress(callback) {
  if (!isTauri()) return () => {};
  return listen("image-manager-progress", (event) => callback(event.payload));
}

const MOCK_MANIFEST_DIGEST =
  "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

export async function loadImageManifest(source) {
  if (!isTauri()) {
    return {
      source,
      manifest: {
        apiVersion: "deploy.desktop/v1",
        kind: "ImageBundle",
        metadata: {
          name: "demo-stack",
          version: "1.2.3",
          description: "示例镜像清单",
          product: "Demo Product",
          channel: "stable",
          createdAt: "2026-10-19T00:00:00Z",
          minToolVersion: "0.1.0"
        },
        spec: {
          registry: "registry.cn-hangzhou.aliyuncs.com",
          namespace: "demo",
          platform: "linux/amd64",
          images: [
            {
              name: "backend",
              repository: "backend",
              digest: MOCK_MANIFEST_DIGEST,
              sizeBytes: 268435456,
              platform: "linux/amd64"
            },
            {
              name: "frontend",
              repository: "frontend",
              digest: MOCK_MANIFEST_DIGEST,
              sizeBytes: 73400320,
              platform: "linux/amd64"
            }
          ],
          artifacts: [{ name: "compose", path: "compose.yaml" }],
          checksums: "sha256sum.txt",
          signature: null
        }
      },
      images: [
        {
          name: "backend",
          repository: "demo/backend",
          reference: `registry.cn-hangzhou.aliyuncs.com/demo/backend@${MOCK_MANIFEST_DIGEST}`,
          digest: MOCK_MANIFEST_DIGEST,
          platform: "linux/amd64",
          size_bytes: 268435456
        },
        {
          name: "frontend",
          repository: "demo/frontend",
          reference: `registry.cn-hangzhou.aliyuncs.com/demo/frontend@${MOCK_MANIFEST_DIGEST}`,
          digest: MOCK_MANIFEST_DIGEST,
          platform: "linux/amd64",
          size_bytes: 73400320
        }
      ],
      total_size_bytes: 341835776,
      warnings: ["清单未包含 signature，建议在正式发布时附加签名"]
    };
  }
  return await invoke("load_image_manifest", { source });
}

export async function pullManifestImagesRemote(
  host,
  port,
  user,
  keyPath,
  source,
  username,
  password,
  imageNames,
  exportDir = null
) {
  if (!isTauri()) {
    return imageNames.map((name) => ({
      name,
      reference: `registry.example.com/demo/${name}@${MOCK_MANIFEST_DIGEST}`,
      digest: MOCK_MANIFEST_DIGEST,
      platform: "linux/amd64",
      exported_path: exportDir ? `${exportDir}/${name}.tar` : null,
      size_bytes: 1024 * 1024,
      message: exportDir ? "已拉取并导出" : "已拉取"
    }));
  }
  return await invoke("pull_manifest_images_remote", {
    host,
    port,
    user,
    keyPath,
    source,
    username,
    password,
    imageNames,
    exportDir
  });
}

export async function pullManifestImagesLocal(
  source,
  username,
  password,
  imageNames,
  exportDir = null
) {
  if (!isTauri()) {
    return imageNames.map((name) => ({
      name,
      reference: `registry.example.com/demo/${name}@${MOCK_MANIFEST_DIGEST}`,
      digest: MOCK_MANIFEST_DIGEST,
      platform: "linux/amd64",
      exported_path: exportDir ? `${exportDir}/${name}.tar` : null,
      size_bytes: 1024 * 1024,
      message: exportDir ? "已拉取并导出" : "已拉取"
    }));
  }
  return await invoke("pull_manifest_images_local", {
    source,
    username,
    password,
    imageNames,
    exportDir
  });
}

export function onRegistryProgress(callback) {
  if (!isTauri()) return () => {};
  return listen("registry-progress", (event) => callback(event.payload));
}

