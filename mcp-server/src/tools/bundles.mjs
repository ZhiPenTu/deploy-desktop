import { z } from 'zod';
import { DeployError } from '../errors.mjs';
import { loadProfile } from '../profile.mjs';
import { composeAction, dbImport, inspectLocalDeployment, inspectRemoteDeployment } from '../deploy.mjs';
import { inspectRemoteBundle, loadLocalImages, loadRemoteImages, removeRemoteBundleFiles, resolveImagesDir, scanLocalBundle, uploadBundle } from '../images.mjs';
import { targetSchema, hostSchema, remoteFields, remoteReadOnly, mutating, destructive, hostFrom } from './shared.mjs';

export function registerBundlesTools(register) {
  register('deploy_bundle_scan', {
    title: '扫描离线镜像包',
    description: '扫描本地或远程交付包 images 目录下的 tar/tgz 文件、总大小与交付包期望文件缺口。',
    inputSchema: {
      target: targetSchema,
      ...remoteFields,
      dir: z.string().optional().describe('本地镜像包目录'),
      bundleDir: z.string().optional().describe('远程镜像包根目录')
    },
    annotations: remoteReadOnly
  }, async (args) => {
    const { profile } = await loadProfile();
    if (args.target === 'remote') {
      const host = await hostFrom(args);
      return inspectRemoteBundle(profile, host, args.bundleDir || host.bundleDir || profile.bundleDir);
    }
    return scanLocalBundle(profile, args.dir || profile.bundleDir);
  });

  register('deploy_bundle_upload', {
    title: '上传离线镜像包',
    description: '把本地镜像包上传到目标机 <remoteBundleDir>/images；默认跳过大小一致的文件。',
    inputSchema: {
      ...remoteFields,
      host: hostSchema,
      localDir: z.string().min(1).describe('本地镜像包目录或 images 目录'),
      remoteBundleDir: z.string().optional().describe('远程镜像包根目录；默认使用当前项目绑定的交付包目录'),
      files: z.array(z.string()).optional(),
      overwrite: z.boolean().optional().default(false)
    },
    annotations: mutating
  }, async (args) => {
    const host = await hostFrom(args);
    const { imagesDir } = await resolveImagesDir(args.localDir);
    const remoteBundleDir = args.remoteBundleDir || host.bundleDir;
    if (!remoteBundleDir) throw new DeployError('请填写远程镜像包目录');
    const messages = await uploadBundle(host, args.localDir, remoteBundleDir, args.files || [], { overwrite: args.overwrite });
    return { localDir: imagesDir, remoteDir: `${remoteBundleDir.replace(/\/+$/, '')}/images`, messages };
  });

  register('deploy_bundle_import', {
    title: '导入目标机离线镜像',
    description: '在目标机对 <remoteBundleDir>/images 中的镜像包执行 docker load。',
    inputSchema: {
      ...remoteFields,
      host: hostSchema,
      remoteBundleDir: z.string().optional().describe('远程镜像包根目录；默认使用当前项目绑定的交付包目录'),
      files: z.array(z.string()).optional()
    },
    annotations: mutating
  }, async (args) => {
    const host = await hostFrom(args);
    const remoteBundleDir = args.remoteBundleDir || host.bundleDir;
    if (!remoteBundleDir) throw new DeployError('请填写远程镜像包目录');
    return { loaded: await loadRemoteImages(host, remoteBundleDir, args.files || []) };
  });

  register('deploy_bundle_deploy', {
    title: '一键离线部署',
    description: '串联离线镜像上传/导入、Compose 启动与可选数据库导入，结束后返回部署状态。首次部署或整机重建使用，需要 confirm=true。',
    inputSchema: {
      target: targetSchema,
      ...remoteFields,
      bundleDir: z.string().optional().describe('本地交付包目录（读取其 images/ 子目录）；默认使用当前项目绑定的交付包'),
      remoteBundleDir: z.string().optional().describe('远程镜像包根目录；默认使用当前项目绑定的交付包目录'),
      files: z.array(z.string()).optional().describe('只处理指定镜像包文件名；为空表示全部'),
      upload: z.boolean().optional().default(true).describe('远程模式先上传本地镜像包'),
      service: z.string().optional().describe('只启动指定服务，默认全部'),
      sqlFile: z.string().optional().describe('启动成功后可选导入的本地 SQL 文件'),
      confirm: z.boolean().optional()
    },
    annotations: destructive
  }, async (args) => {
    if (args.confirm !== true) {
      throw new DeployError('一键部署会导入镜像并启动服务，需要 confirm=true', { code: 'CONFIRM_REQUIRED' });
    }
    const { profile } = await loadProfile({ bundleDir: args.bundleDir });
    const bundleDir = args.bundleDir || profile.bundleDir;
    const files = args.files || [];
    if (args.target === 'remote') {
      const host = await hostFrom(args);
      const remoteBundleDir = args.remoteBundleDir || host.bundleDir;
      if (!remoteBundleDir) throw new DeployError('请填写远程镜像包目录');
      let uploads = [];
      if (args.upload !== false) {
        uploads = await uploadBundle(host, bundleDir, remoteBundleDir, files, { overwrite: false });
      }
      const loaded = await loadRemoteImages(host, remoteBundleDir, files);
      const compose = await composeAction(profile, {
        target: 'remote',
        hostConfig: host,
        action: 'up',
        service: args.service,
        confirm: true
      });
      const database = args.sqlFile
        ? await dbImport(profile, { target: 'remote', hostConfig: host, sqlFile: args.sqlFile, confirm: true })
        : null;
      const status = await inspectRemoteDeployment(profile, host, host.installDir).catch(() => null);
      return { target: 'remote', remoteBundleDir, uploads, loaded, compose, database, status };
    }
    const loaded = await loadLocalImages(bundleDir, files);
    const compose = await composeAction(profile, {
      target: 'local',
      action: 'up',
      service: args.service,
      confirm: true
    });
    const database = args.sqlFile
      ? await dbImport(profile, { target: 'local', sqlFile: args.sqlFile, confirm: true })
      : null;
    const status = await inspectLocalDeployment(profile).catch(() => null);
    return { target: 'local', loaded, compose, database, status };
  });

  register('deploy_bundle_remove', {
    title: '删除目标机镜像包文件',
    description: '删除目标机 <remoteBundleDir>/images 下的指定文件，不删除已导入的 Docker 镜像。',
    inputSchema: {
      ...remoteFields,
      host: hostSchema,
      remoteBundleDir: z.string().optional().describe('远程镜像包根目录；默认使用当前项目绑定的交付包目录'),
      files: z.array(z.string()).min(1),
      confirm: z.boolean().optional()
    },
    annotations: destructive
  }, async (args) => {
    if (args.confirm !== true) throw new DeployError('删除镜像包文件需要 confirm=true', { code: 'CONFIRM_REQUIRED' });
    const host = await hostFrom(args);
    const remoteBundleDir = args.remoteBundleDir || host.bundleDir;
    if (!remoteBundleDir) throw new DeployError('请填写远程镜像包目录');
    return removeRemoteBundleFiles(host, remoteBundleDir, args.files);
  });
}
