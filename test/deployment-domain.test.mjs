import assert from 'node:assert/strict';
import { afterEach, test } from 'node:test';
import { mockIPC, clearMocks } from '@tauri-apps/api/mocks';
import { validInstallDir, createDeploymentContext, contextTarget } from '../src/features/deployment/domain/context.js';
import { deploymentComponents, servicesAreReady, requireSuccessfulResult, missingArtifactIssue } from '../src/features/deployment/domain/components.js';
import { safeText, failureText } from '../src/features/deployment/domain/errors.js';
import { invokeIncremental } from '../src/features/deployment/services/operations.js';

const input = () => ({
  connection: { remoteHost: 'host.example.com', remotePort: 2222, remoteUser: 'deploy', remoteKey: '/key', installDir: '/srv/app' },
  remote: true, bundle: { dir: '/bundle', version: '1.0.0' },
  components: ['backend'], backup: true, action: 'incremental', label: 'demo',
  allowedComponents: ['backend', 'config', 'image']
});

afterEach(() => {
  if (globalThis.window) clearMocks();
  delete globalThis.window;
  delete globalThis.isTauri;
});

test('缺失制品必须由本次选择的组件覆盖，首次部署允许待写入', () => {
  const plan = {
    components: [
      { key: 'api', artifacts: [{ key: 'server' }] },
      { key: 'web', artifacts: [{ key: 'site' }] }
    ]
  };
  const review = {
    hasDeployment: true,
    artifacts: [{ key: 'server', label: '服务程序', status: 'missing' }]
  };
  assert.match(missingArtifactIssue(plan, review, ['web']), /服务程序/);
  assert.equal(missingArtifactIssue(plan, review, ['api']), '');
  assert.equal(missingArtifactIssue(plan, { ...review, hasDeployment: false }, []), '');
  assert.equal(missingArtifactIssue(plan, {
    ...review, artifacts: [{ key: 'server', status: 'incomplete' }]
  }, ['web']), '');
  assert.equal(servicesAreReady({
    ...review,
    targetChecked: true,
    services: [{ containerState: 'running', containerHealth: 'healthy' }]
  }), false);
});

test('deployment paths reject roots, traversal and control characters on both platforms', () => {
  for (const path of ['', '/', '//', '/./', '/srv/../', 'relative', 'C:\\', 'C:/', 'C:/./', '/app\nnext', '/app\0']) {
    assert.equal(validInstallDir(path), false, path);
  }
  assert.equal(validInstallDir('/srv/app', true), true);
  assert.equal(validInstallDir('C:\\apps\\demo'), true);
  assert.equal(validInstallDir('C:\\apps\\demo', true), false);
});

test('confirmation snapshots detach mutable host and component inputs', () => {
  const options = input();
  const context = createDeploymentContext(options);
  options.connection.remoteHost = 'changed.example.com';
  options.components.push('config');
  options.bundle.dir = '/other';
  assert.equal(context.host, 'host.example.com');
  assert.equal(context.bundleDir, '/bundle');
  assert.deepEqual(context.components, ['backend']);
  assert.ok(Object.isFrozen(context) && Object.isFrozen(context.components));
  assert.equal(contextTarget(context).host, 'host.example.com');
  assert.equal(contextTarget({ remote: false }), null);
  for (const patch of [{components:[]}, {components:['unknown']}, {components:['constructor']}, {action:'invalid'}, {bundle:null}]) {
    assert.throws(() => createDeploymentContext({...input(), ...patch}));
  }
  assert.throws(() => createDeploymentContext({...input(), connection:{...input().connection,remotePort:0}}), /端口/);
});

test('component choices come from the bundle descriptor instead of fixed names', () => {
  const plan = {
    components: [
      { key: 'web', label: '站点', artifacts: [], services: ['web'] },
      { key: 'agent', label: '采集程序', artifacts: [], services: [] },
      { key: 'web', label: '重复项', artifacts: [], services: [] }
    ],
    images: [{ imageRef: 'demo/web@sha256:abc', bundleFile: '' }]
  };
  assert.deepEqual(deploymentComponents(plan), ['web', 'agent']);
  assert.deepEqual(deploymentComponents({ components: [], images: [] }), []);
  assert.deepEqual(
    deploymentComponents({ ...plan, images: [{ imageRef: 'demo/web@sha256:abc', bundleFile: 'images/web.tar' }] }),
    ['web', 'agent', 'image']
  );
});

test('a successful command is insufficient without checked and ready containers', () => {
  const healthy = { change:'unchanged', containerState:'running', containerHealth:'healthy' };
  assert.equal(servicesAreReady({targetChecked:true,services:[healthy]}), true);
  for (const review of [null, {targetChecked:false,services:[healthy]}, {targetChecked:true,services:[]},
    {targetChecked:true,services:[{...healthy,change:'removed'}]},
    {targetChecked:true,services:[{...healthy,containerHealth:'starting'}]},
    {targetChecked:true,services:[{...healthy,containerState:'exited'}]}]) assert.equal(servicesAreReady(review), false);
  assert.throws(() => requireSuccessfulResult({success:false}, '更新失败'), /更新失败/);
  assert.throws(() => requireSuccessfulResult(null, '更新失败'), /更新失败/);
});

test('operation service preserves target and dry-run, rejects failed responses and retains recovery details', async () => {
  globalThis.window = {};
  globalThis.isTauri = true;
  const calls=[];
  const result={success:false,error:'更新失败',backup_dir:'/backup',output:'details'};
  mockIPC((command,args)=>{calls.push({command,args});return result;});
  const remote=createDeploymentContext(input());
  await assert.rejects(invokeIncremental(remote,true),(error)=>error.result===result);
  const local=createDeploymentContext({...input(),remote:false});
  await assert.rejects(invokeIncremental(local,false), /更新失败/);
  assert.equal(calls[0].command,'update_remote_deployment');
  assert.equal(calls[0].args.host,remote.host);
  assert.equal(calls[0].args.dryRun,true);
  assert.equal(calls[1].command,'update_local_deployment');
  assert.equal(calls[1].args.dryRun,false);
  mockIPC(()=>({success:true,output:'ok'}));
  assert.equal((await invokeIncremental(remote,true)).success,true);
});

test('deployment diagnostics redact credentials and private keys', () => {
  const text=safeText('password=secret-value token="private-token" Bearer opaque-token https://user:password@example.com/path -----BEGIN PRIVATE KEY-----\nprivate-material\n-----END PRIVATE KEY-----');
  for(const secret of ['secret-value','private-token','opaque-token','user:password','private-material']) assert.equal(text.includes(secret),false);
});

test('failure text keeps the actionable reason instead of a generic docker message', () => {
  const notReady = failureText(
    '应用更新失败：服务 tt-net-manager-cloudflared 未在等待时间内就绪。请查看执行详情中的服务日志，处理后重试。\n==> 当前容器状态',
    '增量更新'
  );
  assert.match(notReady, /tt-net-manager-cloudflared/);
  assert.doesNotMatch(notReady, /Docker 和 Compose/);
  const declaration = failureText(
    '交付包声明的服务与当前生效的部署清单 deploy/docker-compose.lan.yml 不一致：cloudflared 未在该清单中声明。请修正交付包后重新制作制品；本次操作未修改目标机。',
    '增量更新'
  );
  assert.match(declaration, /cloudflared/);
  assert.match(declaration, /不一致/);
  assert.match(declaration, /请修正交付包/);
  assert.doesNotMatch(declaration, /Docker 和 Compose/);
  assert.match(failureText('目标机 Docker 守护进程未运行，请先启动 Docker。', '预检'), /未运行/);
  assert.match(failureText('连接超时：请检查网络连通性。', '连接检查'), /检查目标主机连接/);
});

test('local deployment permission failures do not look like login failures', () => {
  const local = failureText('复制更新文件失败，请检查空间和目录权限', '增量更新', {
    remote: false
  });
  assert.match(local, /目录或文件权限/);
  assert.doesNotMatch(local, /登录权限|凭证/);

  const occupied = failureText(
    '部署文件仍被其他程序占用，请关闭占用程序并确认当前账户可以修改部署目录后重试',
    '增量更新',
    { remote: false }
  );
  assert.match(occupied, /占用/);

  const recovery = failureText(
    '写入更新文件失败，原文件未能恢复，请使用备份目录或目标目录中的临时旧文件恢复后再试',
    '增量更新',
    { remote: false }
  );
  assert.match(recovery, /备份目录/);

  assert.match(
    failureText('Permission denied (publickey).', '增量更新', { remote: true }),
    /登录权限和凭证/
  );
  const remoteFilePermission = failureText(
    '无法读取 Docker Compose 清单，请检查文件权限',
    '增量更新',
    { remote: true }
  );
  assert.doesNotMatch(remoteFilePermission, /登录权限和凭证/);

  for (const error of [
    '当前 SSH 用户无权访问 Docker，请把用户加入 docker 用户组后重新登录',
    'ssh: connect to host 10.0.0.1 port 22: Connection refused',
    '镜像源登录失败，请检查用户名、密码、网络和仓库证书',
    '已填写仓库用户名，请同时填写密码'
  ]) {
    assert.doesNotMatch(
      failureText(error, '增量更新', { remote: true }),
      /主机登录权限和凭证/,
      error
    );
  }
});
