import { readFileSync } from 'node:fs';
import { expect, test } from '@playwright/test';

const { version } = JSON.parse(readFileSync(new URL('../package.json', import.meta.url), 'utf8'));

const pages = [
  { nav: '项目管理', crumb: '项目管理', content: '还没有项目' },
  { nav: '凭证管理', crumb: '凭证管理', content: '还没有 SSH 凭证' },
  { nav: '主机管理', crumb: '主机管理', content: '还没有保存的主机' },
  { nav: '部署向导', crumb: '部署向导', content: '当前没有可用项目' },
  { nav: '制品中心', crumb: '制品中心', content: '还没有可用的制品源' },
  { nav: '镜像与离线包', crumb: '镜像与离线包', content: '本机 Docker 镜像' },
  { nav: 'MCP 服务', crumb: 'MCP 服务', content: '能力分类' },
  { nav: '设置', crumb: '设置', content: '还没有配置云效制品源' }
];

// 界面只面向使用者：出现这些词说明开发说明或中间状态漏到了页面上。
const developerText = [
  'TODO',
  'FIXME',
  'console.log',
  '[object Object]',
  'undefined',
  'NaN',
  'Error:',
  '模拟数据',
  '前端',
  '后端',
  '接口',
  '组件名',
  '开发说明'
];

function collectPageProblems(page) {
  const problems = [];
  page.on('pageerror', (error) => problems.push(`未捕获异常：${error.message}`));
  page.on('console', (message) => {
    if (message.type() === 'error') problems.push(`控制台错误：${message.text()}`);
  });
  return problems;
}

async function openApp(page) {
  await page.goto('/');
  await expect(page.locator('.app-sidebar')).toBeVisible();
}

test('启动后显示完整导航、当前版本与默认页面', async ({ page }) => {
  const problems = collectPageProblems(page);
  await openApp(page);

  const navItems = page.locator('.nav-menu .nav-item');
  await expect(navItems).toHaveCount(pages.length);
  for (const entry of pages) {
    await expect(page.locator('.nav-menu .nav-item', { hasText: entry.nav })).toHaveCount(1);
  }

  await expect(page.locator('.sidebar-footer .version')).toHaveText(`v${version}`);
  await expect(page.locator('.crumb-current')).toHaveText('部署向导');
  expect(problems).toEqual([]);
});

test('每个页面都能进入并显示对应的空状态', async ({ page }) => {
  const problems = collectPageProblems(page);
  await openApp(page);

  for (const entry of pages) {
    await page.locator('.nav-menu .nav-item', { hasText: entry.nav }).click();
    await expect(page.locator('.crumb-current')).toHaveText(entry.crumb);
    await expect(page.locator('.content-panel')).toContainText(entry.content);
  }

  expect(problems).toEqual([]);
});

test('页面文本不包含开发说明、脚本错误或未填充占位', async ({ page }) => {
  await openApp(page);

  for (const entry of pages) {
    await page.locator('.nav-menu .nav-item', { hasText: entry.nav }).click();
    await expect(page.locator('.crumb-current')).toHaveText(entry.crumb);
    const text = await page.locator('body').innerText();
    for (const word of developerText) {
      expect(text, `${entry.nav} 页面出现了开发说明「${word}」`).not.toContain(word);
    }
  }
});

test('浏览器预览不伪造数据：保存主机时明确提示需在桌面应用重试', async ({ page }) => {
  await openApp(page);
  await page.locator('.nav-menu .nav-item', { hasText: '主机管理' }).click();

  await page.getByRole('button', { name: '新增主机' }).click();
  const dialog = page.locator('.el-dialog');
  await expect(dialog).toBeVisible();
  await dialog.getByPlaceholder('例如：本机').fill('自动化测试主机');
  await dialog.getByPlaceholder('例如：/opt/deploy').fill('/opt/deploy');
  await dialog.getByRole('button', { name: '保存' }).click();

  const message = page.locator('.el-message__content').last();
  await expect(message).toContainText('暂不可用，请在桌面应用中重试');
  await expect(message).not.toContainText('Error:');
  await expect(page.locator('.content-panel')).toContainText('0 台主机');
  await expect(page.locator('.content-panel')).not.toContainText('已保存');
});

test('项目表单在缺少目标主机时给出中文校验提示且不创建项目', async ({ page }) => {
  await openApp(page);
  await page.locator('.nav-menu .nav-item', { hasText: '项目管理' }).click();

  await page.getByRole('button', { name: '新建项目' }).click();
  const dialog = page.locator('.el-dialog');
  await dialog.getByPlaceholder('例如：生产环境').fill('自动化测试项目');
  await dialog.getByPlaceholder('例如：/opt/demo').fill('/opt/demo');
  await dialog.getByRole('button', { name: '保存' }).click();

  await expect(page.locator('.el-message__content').last()).toContainText('请选择目标主机');
  await dialog.getByRole('button', { name: '取消' }).click();
  await expect(page.locator('.content-panel')).toContainText('还没有项目');
});
