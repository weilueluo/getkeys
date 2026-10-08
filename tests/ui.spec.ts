import { test, expect, type Page } from '@playwright/test';
import AxeBuilder from '@axe-core/playwright';
import { emptySnapshot, connectedSnapshot } from './fixtures';

async function mockHost(page: Page, connected = false) {
  await page.addInitScript(
    ({ data }) => {
      const host = window as typeof window & {
        __TAURI_INTERNALS__: { invoke: (name: string, args: any) => Promise<unknown> };
        isTauri: boolean;
        testCommands: any[];
      };
      host.testCommands = [];
      host.isTauri = true;
      host.__TAURI_INTERNALS__ = {
        invoke: async (name, args) => {
          if (name === 'copy_credential') return;
          const c = args.command;
          host.testCommands.push(c);
          if (c.kind === 'snapshot') return structuredClone(data);
          if (c.kind === 'install') {
            data.plugins.find((p) => p.manifest.id === c.plugin_id)!.installed = true;
            return null;
          }
          if (c.kind === 'begin_auth')
            return {
              kind: 'browser',
              flow_id: 'flow',
              url: 'https://github.com/login/device',
              user_code: 'ABCD-1234',
              interval: 1,
            };
          if (c.kind === 'poll_auth') return { kind: 'pending', interval: 10 };
          if (c.kind === 'cancel_auth') return null;
          if (c.kind === 'resources')
            return [{ id: 'abcdefghijklmnopqrst', name: 'GetKeys', kind: 'project' }];
          if (c.kind === 'sync') return data.credentials;
          if (c.kind === 'reveal') return { fields: { token: 'only-revealed-on-request' } };
          if (c.kind === 'plan_rotation') {
            const r = {
              id: 'rotation',
              credential_id: c.credential_id,
              connection_id: 'connection',
              resource: 'abcdefghijklmnopqrst',
              plugin_version: '0.1.0',
              name: c.name,
              state: { kind: 'planned' },
              candidate_id: null,
              bindings: [],
              updated_bindings: [],
              verified_bindings: [],
              created_at: 1791493200,
            };
            data.rotations.push(r as any);
            return r;
          }
          throw new Error(`Unhandled test command: ${c.kind}`);
        },
      };
    },
    { data: connected ? connectedSnapshot() : emptySnapshot() },
  );
}
test('marketplace, browser authorization, and keyboard navigation', async ({ page }) => {
  await mockHost(page);
  await page.goto('/');
  await expect(page.getByRole('heading', { name: 'One place for every key.' })).toBeVisible();
  await expect(page.locator('.plugin-card')).toHaveCount(4);
  await page.screenshot({ path: 'artifacts/marketplace.png', fullPage: true });
  const results = await new AxeBuilder({ page }).analyze();
  expect(results.violations).toEqual([]);
  await page.keyboard.press('Control+k');
  await expect(page.getByRole('textbox', { name: 'Search', exact: true })).toBeFocused();
  await page.getByRole('textbox', { name: 'Search', exact: true }).fill('github');
  await expect(page.locator('.plugin-card')).toHaveCount(1);
  await page.getByRole('button', { name: 'Add plugin' }).click();
  await expect(page.getByRole('dialog')).toBeVisible();
  await expect(page.getByText('ABCD-1234')).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(page.getByRole('dialog')).toHaveCount(0);
});
test('secrets stay masked and rotation requires an explicit plan', async ({ page }) => {
  await mockHost(page, true);
  await page.goto('/');
  await page.getByRole('button', { name: /Production API/ }).click();
  await expect(page.getByText('only-revealed-on-request')).toHaveCount(0);
  await page.getByRole('button', { name: 'Reveal for 30 seconds' }).click();
  await expect(page.getByText('only-revealed-on-request')).toBeVisible();
  await page.getByRole('button', { name: 'Hide values' }).click();
  await page.getByRole('button', { name: 'Rotate this key', exact: true }).click();
  await expect(page.getByText('No consumers are registered.', { exact: false })).toBeVisible();
  await page.getByRole('button', { name: 'Review rotation plan' }).click();
  await expect(page.getByRole('button', { name: 'Create replacement' })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Retire old key' })).toHaveCount(0);
  await page.screenshot({ path: 'artifacts/rotation.png', fullPage: true });
});
test('compact layout has no horizontal overflow', async ({ page }) => {
  await mockHost(page);
  await page.setViewportSize({ width: 640, height: 850 });
  await page.goto('/');
  await expect(page.locator('.plugin-card')).toHaveCount(4);
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(
    true,
  );
  await page.screenshot({ path: 'artifacts/compact.png', fullPage: true });
});
