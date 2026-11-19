import { expect, test } from '@playwright/test';
import { useDesktopBackend } from './support/desktop.js';

const mediaDir = '/Volumes/TRANSFER';
const mediaRoot = `${mediaDir}/部署舱介质`;
const offlineDir = '/Users/demo/Documents/部署舱/离线镜像包';

const localBundle = {
  dir: '/Users/demo/Library/Application Support/deploy-desktop/bundles/source-1/1.2.3',
  label: '演示产品',
  product: 'demo',
  version: '1.2.3',
  commit: 'abc1234',
  sourceName: '云效流水线',
  sizeBytes: 1_048_576,
  modifiedMs: 1_760_000_000_000
};

const offlinePackage = {
  name: 'registry.demo/app_1.2.3.tar',
  imageRefs: ['registry.demo/app:1.2.3'],
  sizeBytes: 2_097_152,
  modifiedMs: 1_760_000_000_000
};

const artifactEntry = {
  id: 'artifact-1234567890abcdef',
  kind: 'artifact',
  label: '演示产品',
  version: '1.2.3',
  product: 'demo',
  displayName: '演示产品',
  images: [],
  file: 'artifacts/演示产品-1.2.3.tgz',
  filePath: `${mediaRoot}/artifacts/演示产品-1.2.3.tgz`,
  sizeBytes: 1_048_576,
  sha256: 'a'.repeat(64),
  addedAt: '2026-11-19T08:30:00Z',
  present: true,
  issue: ''
};

const imageEntry = {
  id: 'image-abcdef1234567890',
  kind: 'image',
  label: 'registry.demo/app_1.2.3.tar',
  version: '',
  product: '',
  displayName: '',
  images: ['registry.demo/app:1.2.3'],
  file: 'images/registry.demo/app_1.2.3.tar',
  filePath: `${mediaRoot}/images/registry.demo/app_1.2.3.tar`,
  sizeBytes: 2_097_152,
  sha256: 'b'.repeat(64),
  addedAt: '2026-11-19T08:31:00Z',
  present: true,
  issue: ''
};

function mediaScan(entries) {
  return {
    selectedDir: mediaDir,
    rootDir: mediaRoot,
    exists: true,
    writable: true,
    entries,
    totalBytes: entries.reduce((sum, item) => sum + item.sizeBytes, 0)
  };
}

async function openUsbPage(page) {
  await page.goto('/');
  await page.locator('.nav-menu .nav-item', { hasText: 'USB 中转' }).click();
  await expect(page.locator('.page-title')).toHaveText('USB 中转');
  await page.locator('.header-actions').getByRole('button', { name: '选择介质目录' }).click();
  await expect(page.locator('.media-summary .path-text')).toHaveText(mediaRoot);
}

test('写入 USB 会带上选中的交付包与镜像包，并刷新介质内容', async ({ page }) => {
  await useDesktopBackend(page, {
    dialogPath: mediaDir,
    usbMedia: mediaScan([artifactEntry]),
    usbLocalBundles: [localBundle],
    offlineDir,
    offlinePackages: [offlinePackage],
    usbExportResult: {
      written: 2,
      failed: 0,
      items: [
        { kind: 'artifact', label: '演示产品', success: true, message: '已写入' },
        { kind: 'image', label: offlinePackage.name, success: true, message: '已写入' }
      ],
      scan: mediaScan([artifactEntry, imageEntry])
    }
  });

  await openUsbPage(page);
  const bundleTable = page.locator('.usb-write .usb-card', { hasText: '本机交付包' });
  const packageTable = page.locator('.usb-write .usb-card', { hasText: '本机离线镜像包' });
  await expect(bundleTable).toContainText('演示产品');
  await expect(packageTable).toContainText(offlinePackage.name);

  await bundleTable.locator('tbody tr').first().locator('.el-checkbox').click();
  await packageTable.locator('tbody tr').first().locator('.el-checkbox').click();
  await page.getByRole('button', { name: '写入 USB' }).click();

  await expect(page.locator('.el-message-box')).toContainText(mediaRoot);
  await page.locator('.el-message-box__btns .el-button--primary').click();

  await expect(page.getByText('已写入 2 项到 USB 介质')).toBeVisible();
  const calls = await page.evaluate(() =>
    window.__deployTestCalls
      .filter((item) => item.command === 'export_usb_media')
      .map((item) => ({
        mediaDir: item.args.mediaDir,
        artifactDirs: item.args.artifactDirs,
        imageFiles: item.args.imageFiles
      }))
  );
  expect(calls).toEqual([
    {
      mediaDir,
      artifactDirs: [localBundle.dir],
      imageFiles: [
        {
          path: `${offlineDir}/${offlinePackage.name}`,
          images: offlinePackage.imageRefs
        }
      ]
    }
  ]);

  await expect(page.locator('.summary-meta')).toContainText('交付包 1 个');
  await expect(page.locator('.summary-meta')).toContainText('镜像包 1 个');
  await page.locator('.el-tabs__item', { hasText: '从 USB 读取' }).click();
  const readTable = page.locator('.usb-read .usb-table');
  await expect(readTable).toContainText('演示产品');
  await expect(readTable).toContainText(offlinePackage.name);
});

test('从 USB 读取交付包会校验后带入部署向导', async ({ page }) => {
  const prepared = {
    dir: '/Users/demo/Library/Application Support/deploy-desktop/bundles/local/local-9',
    version: '1.2.3',
    commit: 'abc1234',
    md5: '',
    sizeBytes: 1_048_576,
    files: 6
  };
  await useDesktopBackend(page, {
    dialogPath: mediaDir,
    usbMedia: mediaScan([artifactEntry]),
    usbLocalBundles: [],
    usbVerification: { valid: true, message: '校验通过' },
    preparedBundle: prepared
  });

  await openUsbPage(page);
  await page.locator('.el-tabs__item', { hasText: '从 USB 读取' }).click();
  await page.getByRole('button', { name: '用于部署' }).click();

  await expect(page.locator('.crumb-current')).toHaveText('部署向导');
  const calls = await page.evaluate(() =>
    window.__deployTestCalls
      .filter((item) => ['verify_usb_entry', 'prepare_local_artifact_bundle'].includes(item.command))
      .map((item) => ({ command: item.command, args: item.args }))
  );
  expect(calls.some((item) => item.command === 'verify_usb_entry' && item.args.entryId === artifactEntry.id))
    .toBe(true);
  expect(calls.some((item) =>
    item.command === 'prepare_local_artifact_bundle' && item.args.path === artifactEntry.filePath
  )).toBe(true);
});

test('从 USB 复制镜像包到离线目录并提示结果', async ({ page }) => {
  await useDesktopBackend(page, {
    dialogPath: mediaDir,
    usbMedia: mediaScan([imageEntry]),
    usbImageImport: {
      path: `${offlineDir}/registry.demo`,
      message: '已复制到离线包目录'
    }
  });

  await openUsbPage(page);
  await page.locator('.el-tabs__item', { hasText: '从 USB 读取' }).click();
  await page.getByRole('button', { name: '复制到离线目录' }).click();

  await expect(page.locator('.el-message__content', { hasText: '已复制到离线包目录' })).toBeVisible();
  await expect(page.locator('.usb-read .el-alert')).toContainText(`${offlineDir}/registry.demo`);
  const calls = await page.evaluate(() =>
    window.__deployTestCalls
      .filter((item) => item.command === 'import_usb_image')
      .map((item) => ({ mediaDir: item.args.mediaDir, entryId: item.args.entryId }))
  );
  expect(calls).toEqual([{ mediaDir, entryId: imageEntry.id }]);
});
