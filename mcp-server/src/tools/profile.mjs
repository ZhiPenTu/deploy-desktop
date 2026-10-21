import { appVersion, loadProfile } from '../profile.mjs';
import { readOnly } from './shared.mjs';

export function registerProfileTools(register) {
  register('deploy_profile_get', {
    title: '读取当前部署目标',
    description: '读取当前项目绑定的部署目标与交付包内容：服务端口、数据库、镜像与可用组件。',
    inputSchema: {},
    annotations: readOnly
  }, async () => {
    const { profile, project, bundle } = await loadProfile();
    return {
      project,
      bundle: {
        product: bundle.product(),
        displayName: bundle.metadata.displayName,
        version: bundle.version(),
        branch: bundle.metadata.branch,
        commitShort: bundle.metadata.commitShort,
        builtAt: bundle.metadata.builtAt,
        containerPrefix: profile.containerPrefix,
        compose: profile.compose,
        components: profile.components,
        services: profile.services,
        database: profile.database,
        images: profile.images,
        steps: profile.steps
      },
      profile,
      appVersion: await appVersion()
    };
  });
}
