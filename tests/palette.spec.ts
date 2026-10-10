import { test, expect } from '@playwright/test';
import AxeBuilder from '@axe-core/playwright';
import { connectedSnapshot, withVercel } from './fixtures';

test('quick copy palette finds a key and copies it with the keyboard', async ({ page }) => {
  await page.setViewportSize({ width: 600, height: 420 });
  await page.addInitScript(
    ({ data }) => {
      const host = window as typeof window & {
        __TAURI_INTERNALS__: unknown;
        isTauri: boolean;
        calls: [string, unknown][];
      };
      host.calls = [];
      host.isTauri = true;
      host.__TAURI_INTERNALS__ = {
        metadata: { currentWindow: { label: 'palette' }, currentWebview: { label: 'palette' } },
        invoke: async (name: string, args: any) => {
          host.calls.push([name, args]);
          if (name === 'request' && args.command.kind === 'snapshot') return structuredClone(data);
          if (name === 'copy_credential' || name === 'plugin:window|hide') return null;
          throw new Error(`unexpected ${name}`);
        },
      };
    },
    { data: withVercel(connectedSnapshot()) },
  );
  await page.goto('/#palette');

  const search = page.getByRole('combobox', { name: 'Search keys' });
  await expect(search).toBeFocused();
  const options = page.getByRole('option');
  await expect(options).toHaveCount(3);
  await page.screenshot({ path: 'artifacts/palette.png' });
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);

  await search.fill('cli');
  await expect(options).toHaveCount(2);
  await expect(options.first()).toHaveAttribute('aria-selected', 'true');
  await search.press('ArrowDown');
  await expect(options.nth(1)).toHaveAttribute('aria-selected', 'true');
  await expect(options.nth(1)).toContainText('Vercel CLI');
  await search.press('Enter');

  await expect(page.getByRole('status')).toContainText('Copied');
  await expect
    .poll(() => page.evaluate(() => (window as any).calls.map(([name]: [string]) => name)))
    .toContain('plugin:window|hide');
  const calls = await page.evaluate(() => (window as any).calls as [string, any][]);
  expect(calls.filter(([name]) => name === 'copy_credential').map(([, args]) => args)).toEqual([
    { credentialId: 'signin-vercel', field: 'token' },
  ]);
  await expect(search).toHaveValue('');
});
