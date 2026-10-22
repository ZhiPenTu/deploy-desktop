import { expect, test } from '@playwright/test';

test('部署向导展示完整步骤条并标记当前步骤', async ({ page }) => {
  const problems = [];
  page.on('pageerror', (error) => problems.push(`未捕获异常：${error.message}`));

  await page.goto('/');
  await page.locator('.nav-menu .nav-item', { hasText: '部署向导' }).click();

  const steps = page.locator('.steps-header .stepper .step');
  await expect(steps).toHaveCount(6);
  await expect(steps).toHaveText([
    '1选择项目',
    '2检测环境',
    '3选择制品',
    '4解压制品',
    '5清单比对',
    '6选择更新'
  ]);
  await expect(page.locator('.stepper [aria-current="step"]')).toHaveText('1选择项目');
  await expect(page.locator('.steps-header')).toBeVisible();

  expect(problems).toEqual([]);
});
