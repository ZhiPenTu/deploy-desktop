import assert from 'node:assert/strict';
import { afterEach, test } from 'node:test';
import { mockIPC, clearMocks } from '@tauri-apps/api/mocks';
import { validInstallDir, createDeploymentContext, contextTarget } from '../src/features/deployment/domain/context.js';
import { servicesAreReady, requireSuccessfulResult } from '../src/features/deployment/domain/components.js';
import { safeText, failureText } from '../src/features/deployment/domain/errors.js';
import { invokeVersionDeployment } from '../src/features/deployment/services/operations.js';

const input = () => ({
  connection: { remoteHost: 'host.example.com', remotePort: 2222, remoteUser: 'deploy', remoteKey: '/key', installDir: '/srv/app' },
  remote: true, bundle: { dir: '/bundle', version: '1.0.0' },
  label: 'demo'
});

afterEach(() => {
  if (globalThis.window) clearMocks();
  delete globalThis.window;
  delete globalThis.isTauri;
});

test('deployment paths reject roots, traversal and control characters on both platforms', () => {
  for (const path of ['', '/', '//', '/./', '/srv/../', 'relative', 'C:\\', 'C:/', 'C:/./', '/app\nnext', '/app\0']) {
    assert.equal(validInstallDir(path), false, path);
  }
  assert.equal(validInstallDir('/srv/app', true), true);
  assert.equal(validInstallDir('C:\\apps\\demo'), true);
  assert.equal(validInstallDir('C:\\apps\\demo', true), false);
});

test('confirmation snapshots detach mutable host and bundle inputs', () => {
  const options = input();
  const context = createDeploymentContext(options);
  options.connection.remoteHost = 'changed.example.com';
  options.bundle.dir = '/other';
  assert.equal(context.host, 'host.example.com');
  assert.equal(context.bundleDir, '/bundle');
  assert.ok(Object.isFrozen(context));
  assert.equal(contextTarget(context).host, 'host.example.com');
  assert.equal(contextTarget({ remote: false }), null);
  for (const patch of [{bundle:null}]) {
    assert.throws(() => createDeploymentContext({...input(), ...patch}));
  }
  assert.throws(() => createDeploymentContext({...input(), connection:{...input().connection,remotePort:0}}), /端口/);
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

test('version deployment preserves target and dry-run, rejects failed preflight and retains recovery details', async () => {
  globalThis.window = {};
  globalThis.isTauri = true;
  const calls=[];
  const result={success:false,error:'更新失败',backup_dir:'/backup',output:'details'};
  mockIPC((command,args)=>{calls.push({command,args});return result;});
  const remote=createDeploymentContext(input());
  await assert.rejects(invokeVersionDeployment(remote,true),(error)=>error.result===result);
  const local=createDeploymentContext({...input(),remote:false});
  const localResult=await invokeVersionDeployment(local,false);
  assert.equal(localResult.success,false);
  assert.equal(calls[0].command,'deploy_remote_version');
  assert.equal(calls[0].args.host,remote.host);
  assert.equal(calls[0].args.dryRun,true);
  assert.equal(calls[1].command,'deploy_local_version');
  assert.equal(calls[1].args.dryRun,false);
  mockIPC(()=>({success:true,output:'ok'}));
  assert.equal((await invokeVersionDeployment(remote,true)).success,true);
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

test('error objects do not leak the Error prefix into user-facing text', () => {
  const text = failureText(
    new Error('部署文件仍被其他程序占用；受影响的服务已停止，请关闭占用程序后重新执行增量更新'),
    '增量更新',
    { remote: false }
  );
  assert.match(text, /被其他程序占用/);
  assert.doesNotMatch(text, /^增量更新失败：Error:/);
  assert.equal(safeText(new Error('部署失败')), '部署失败');
  assert.equal(safeText('  kafka:\n    image: demo'), '  kafka:\n    image: demo');
});

test('mount failures are reported as mount problems instead of permission problems', () => {
  const raw =
    'Compose 操作失败：Docker 无法挂载部署目录，请确认部署目录位于本机磁盘目录，避免使用网络共享目录后重试\n' +
    '原始输出：Container tt-net-manager-gateway-shard-1 Recreate; Error response from daemon: mount denied: ' +
    'the source path \\\\?\\D:\\400mbushu\\deploy\\site is not a valid Windows path';
  const text = failureText(raw, '增量更新', { remote: false });
  assert.match(text, /无法挂载部署目录/);
  assert.doesNotMatch(text, /目录或文件权限/);
  assert.doesNotMatch(text, /原始输出/);
  assert.doesNotMatch(text, /mount denied/);

  const bare = failureText(
    'Error response from daemon: mount denied: the source path \\\\?\\D:\\400mbushu is not a valid Windows path',
    '增量更新',
    { remote: false }
  );
  assert.match(bare, /本机磁盘目录/);
  assert.doesNotMatch(bare, /目录或文件权限/);
});

test('compose failures name the actual cause instead of a generic docker hint', () => {
  const pull = failureText(
    'Compose 操作失败：当前 Docker Compose 版本不支持本次更新所需的命令参数，请升级 Docker Compose 后重试\n原始输出：unknown flag: --pull',
    '增量更新',
    { remote: false }
  );
  assert.match(pull, /升级 Docker Compose/);
  assert.doesNotMatch(pull, /Docker 和 Compose/);

  const mount = failureText(
    'Compose 操作失败：Docker 未找到要挂载的部署文件，请确认交付包已完整写入部署目录，并确认 Docker 可以访问该磁盘后重试\n原始输出：bind source path does not exist',
    '增量更新',
    { remote: false }
  );
  assert.match(mount, /未找到要挂载的部署文件/);
  assert.doesNotMatch(mount, /目录或文件权限/);

  const platform = failureText(
    'Compose 操作失败：镜像与当前 Docker 平台不匹配，请确认 Docker 使用 Linux 容器，并导入与目标架构一致的镜像',
    '增量更新',
    { remote: false }
  );
  assert.match(platform, /平台不匹配/);

  const fallback = failureText(
    'Compose 操作失败：命令未执行成功，请检查部署目录与服务配置后重试\n原始输出：unrecognized daemon error',
    '增量更新',
    { remote: false }
  );
  assert.match(fallback, /执行详情|运行日志/);
  assert.doesNotMatch(fallback, /Docker 和 Compose 正常运行/);
});

test('diagnostic lines stay out of the main failure message', () => {
  const text = failureText(
    '替换原部署文件失败，文件可能正被其他程序占用，请关闭占用后重试\n替换详情：阶段=MoveAside 错误=os error 5 旧文件=D:\\deploy\\site',
    '增量更新',
    { remote: false }
  );
  assert.match(text, /被其他程序占用/);
  assert.doesNotMatch(text, /阶段=MoveAside/);
  assert.doesNotMatch(text, /替换详情/);
});

test('actionable reasons are redacted before reaching the user', () => {
  const text = failureText(
    '部署文件被占用：token=super-secret-value 来自 https://example.internal/host',
    '增量更新',
    { remote: false }
  );
  assert.doesNotMatch(text, /super-secret-value/);
  assert.doesNotMatch(text, /example\.internal/);
});
