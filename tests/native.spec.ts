import { test, expect, chromium, type Browser, type Page } from '@playwright/test';
import { spawn, spawnSync, type ChildProcess } from 'node:child_process';
import { existsSync, mkdirSync, readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { createServer } from 'node:net';

let child: ChildProcess;
let browser: Browser;
let page: Page;
const data = resolve('.local/native-test');
test.beforeAll(async () => {
  if (process.platform !== 'win32')
    throw new Error(
      'Native validation currently targets Windows. Run the native project on Windows.',
    );
  const executable = resolve('target/debug/getkeys-desktop.exe');
  if (!existsSync(executable))
    throw new Error('Build the native test app first: cargo build -p getkeys-desktop');
  const server = createServer();
  await new Promise<void>((r) => server.listen(0, '127.0.0.1', r));
  const address = server.address();
  if (!address || typeof address === 'string') throw new Error('No debug port');
  const port = address.port;
  await new Promise<void>((r) => server.close(() => r()));
  mkdirSync(data, { recursive: true });
  const env = {
    ...process.env,
    GETKEYS_DATA_DIR: data,
    GETKEYS_REGISTRY: resolve('registry/dist'),
    WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${port}`,
    WEBVIEW2_USER_DATA_FOLDER: resolve(data, 'webview'),
    // Hide this machine's GitHub CLI sign-in so adding GitHub falls back to a pasted token.
    GH_CONFIG_DIR: resolve(data, 'gh'),
  };
  for (const name of ['GH_TOKEN', 'GITHUB_TOKEN', 'GH_ENTERPRISE_TOKEN']) delete env[name];
  delete env.GETKEYS_GITHUB_CLIENT_ID;
  delete env.GETKEYS_BROKER_URL;
  if (test.info().project.metadata.packaged) delete env.GETKEYS_REGISTRY;
  let output = '';
  child = spawn(executable, [], { env, windowsHide: true, stdio: 'pipe' });
  child.stdout?.on('data', (chunk) => {
    output += chunk;
  });
  child.stderr?.on('data', (chunk) => {
    output += chunk;
  });
  await expect
    .poll(
      async () => {
        if (child.exitCode !== null)
          throw new Error(`Native app exited (${child.exitCode}): ${output}`);
        try {
          return (await fetch(`http://127.0.0.1:${port}/json/version`)).ok;
        } catch {
          return false;
        }
      },
      { timeout: 30_000, message: 'Native WebView2 must expose its test debugging endpoint' },
    )
    .toBe(true);
  browser = await chromium.connectOverCDP(`http://127.0.0.1:${port}`);
  await expect.poll(() => browser.contexts()[0]?.pages().length ?? 0).toBeGreaterThan(0);
  page = browser.contexts()[0].pages()[0];
});
test.afterAll(async () => {
  await browser?.close();
  if (child?.pid && child.exitCode === null)
    spawnSync('taskkill', ['/pid', String(child.pid), '/T', '/F'], {
      windowsHide: true,
      stdio: 'ignore',
    });
});
test('real native IPC installs a signed plugin and uses an encrypted OS-keyed vault', async () => {
  await expect(page.getByRole('heading', { name: 'Providers', exact: true })).toBeVisible();
  if (test.info().project.metadata.packaged)
    expect(page.url()).toMatch(/^(http:\/\/tauri\.localhost|tauri:\/\/localhost)/);
  await expect(page.locator('.provider-card')).toHaveCount(4);
  const github = page
    .locator('.provider-card')
    .filter({ has: page.getByRole('heading', { name: 'GitHub', exact: true }) });
  await github.getByRole('button', { name: 'Add', exact: true }).click();
  // No sign-in on this device, so the scan finds nothing and GetKeys asks for a token.
  const dialog = page.getByRole('dialog', { name: 'Connect GitHub' });
  await dialog.getByLabel('Token', { exact: true }).fill('not-a-real-token');
  await dialog.getByRole('button', { name: 'Connect', exact: true }).click();
  await expect(dialog.getByRole('alert')).toBeVisible({ timeout: 30_000 });
  await page.keyboard.press('Escape');
  await expect(github.getByRole('button', { name: 'Add', exact: true })).toBeVisible();
  await page.screenshot({ path: 'artifacts/native-app.png', fullPage: true });
  const header = readFileSync(resolve(data, 'vault.db')).subarray(0, 16).toString();
  expect(header).not.toBe('SQLite format 3\0');
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'Local vault' })).toBeVisible();
});
