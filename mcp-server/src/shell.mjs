// Shared Bash templates are also embedded in the desktop binary with include_str!.
import { readFileSync } from 'node:fs';
import { shellQuote } from './util.mjs';
import { DeployError } from './errors.mjs';
import { run, requireBinary } from './exec.mjs';

const template = (name) => readFileSync(new URL(`../../shell/${name}.sh`, import.meta.url), 'utf8');
const transferBody = template('transfer');
const backupBody = template('backup');
const applyBody = `${template('compose-update')}\n${template('readiness')}\ncompose ps\n`;
export const BASH_STDIN = 'bash --noprofile --norc -s --';
export const stdinProgram = (script) => `{\n${script}\n}\n`;
export const bashCommand = (script) => `bash --noprofile --norc -c ${shellQuote(script)}`;

export function bashScript(body, args = []) {
  if ([body, ...args].some((value) => String(value).includes('\0'))) {
    throw new DeployError('执行参数包含无效字符，请检查部署配置');
  }
  return `set -euo pipefail\nset -- ${args.map(shellQuote).join(' ')}\n${body}\n`;
}
export const transferScript = (action, target, stage = '') => bashScript(transferBody, [action, target, stage]);
export const backupScript = (installDir, targets) => bashScript(backupBody, [installDir, ...targets]);
export const applyScript = ({ deployDir, compose, config, restart, wait }) =>
  bashScript(applyBody, [deployDir, compose, config ? 'yes' : 'no', String(restart.length), ...restart, ...wait]);

export const runBashFile = (file, args, options = {}) =>
  run(requireBinary('bash'), ['--noprofile', '--norc', '--', file, ...args], options);
