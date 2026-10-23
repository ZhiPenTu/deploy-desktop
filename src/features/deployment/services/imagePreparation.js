import {
  exportDeploymentImages,
  importRemoteImageBundle,
  inspectDeploymentReview,
  loadOfflineImages,
  scanLocalImageBundle,
  uploadRemoteImageBundle
} from '../../../api/tauri.js';

export async function prepareImagePackages(context, onPhase) {
  if (context.source === 'docker') {
    onPhase('正在从本机导出缺失镜像');
    const exported = await exportDeploymentImages(context.bundleDir, [...context.references], context.target);
    if (!Array.isArray(exported)) throw new Error('镜像导出结果未确认，请检查本机离线包后重试');
    const failed = exported.find((item) => item.success !== true);
    if (failed) throw new Error(`导出镜像失败：${failed.message || '请检查本机 Docker 后重试'}`);
    if (!Array.isArray(exported) || exported.length !== context.references.length ||
        context.references.some((reference) => !exported.some((item) => item.reference === reference))) {
      throw new Error('镜像导出结果不完整，请检查本机离线包后重试');
    }
    const packages = exported.map((item) => {
      const path = String(item.path || '').replace(/\\/g, '/');
      const separator = path.lastIndexOf('/');
      if (separator < 0 || !item.fileName || path.slice(separator + 1) !== item.fileName) {
        throw new Error('镜像包保存位置未确认，请重新导出');
      }
      return { root: path.slice(0, separator), name: item.fileName };
    });
    if (new Set(packages.map((item) => item.root)).size !== 1) {
      throw new Error('镜像包保存位置不一致，请重新导出');
    }
    return { root: packages[0].root, files: packages.map((item) => item.name) };
  }
  onPhase('正在检查离线镜像包');
  const [scan, checked] = await Promise.all([
    scanLocalImageBundle(context.imageDir),
    inspectDeploymentReview(context.bundleDir, context.installDir, context.target, context.imageDir)
  ]);
  const files = new Set();
  for (const reference of context.references) {
    const image = checked.images?.find((item) => item.imageRef === reference);
    const file = scan.files?.find((item) => image?.localFile &&
      (item.name === image.localFile || `images/${item.name}` === image.localFile));
    if (!file) throw new Error(`离线目录中未找到镜像 ${reference} 对应的镜像包，请选择其他目录`);
    files.add(file.name);
  }
  if (!files.size) throw new Error('没有可导入的镜像包，请重新比对');
  return { root: context.imageDir, files: [...files] };
}

export async function importImagePackages(context, packages, onPhase) {
  if (context.target) {
    const { host, port, user, keyPath } = context.target;
    onPhase('正在向目标主机传输镜像包');
    const uploaded = await uploadRemoteImageBundle(host, port, user, keyPath, packages.root,
      context.remoteDir, packages.files, false);
    if (!Array.isArray(uploaded) || uploaded.length !== packages.files.length) {
      throw new Error('镜像包传输结果不完整，请检查目标主机后重新比对');
    }
    onPhase('正在目标主机解压并导入镜像');
    const imported = await importRemoteImageBundle(host, port, user, keyPath, context.remoteDir, packages.files);
    if (!Array.isArray(imported) || imported.length !== packages.files.length) {
      throw new Error('镜像导入结果不完整，请检查目标主机后重新比对');
    }
  } else {
    onPhase('正在本机导入镜像');
    const imported = await loadOfflineImages(packages.root);
    if (!Array.isArray(imported) || !imported.length) throw new Error('镜像导入结果未确认，请重新比对');
  }
}
