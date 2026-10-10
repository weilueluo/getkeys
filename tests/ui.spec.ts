import { test, expect, type Page } from '@playwright/test';
import AxeBuilder from '@axe-core/playwright';
import { emptySnapshot, connectedSnapshot, withVercel } from './fixtures';
import type { Snapshot } from '../apps/desktop/src/types';

async function mockHost(page: Page, connected: boolean | Snapshot = false) {
  await page.addInitScript(
    ({ data }) => {
      const host = window as typeof window & {
        __TAURI_INTERNALS__: { invoke: (name: string, args: any) => Promise<unknown> };
        isTauri: boolean;
        testCommands: any[];
        release: () => void;
      };
      const gate = new Promise<void>((r) => (host.release = r));
      const found = [
        {
          id: 'gh:octocat',
          plugin_id: 'official.github',
          source: 'GitHub CLI',
          account: 'octocat',
          connected: false,
        },
        {
          id: 'vercel',
          plugin_id: 'official.vercel',
          source: 'Vercel CLI',
          account: '',
          connected: false,
        },
      ];
      host.testCommands = [];
      host.isTauri = true;
      host.__TAURI_INTERNALS__ = {
        invoke: async (name, args) => {
          if (name === 'copy_credential') return;
          if (name === 'export_env') return true;
          const c = args.command;
          host.testCommands.push(c);
          if (c.kind === 'snapshot') return structuredClone(data);
          if (c.kind === 'discover') return found;
          if (c.kind === 'scan_sources') {
            const plugin = data.plugins.find((p) => p.manifest.id === c.plugin_id);
            if (plugin) plugin.installed = true;
            return found.filter((d) => !c.plugin_id || d.plugin_id === c.plugin_id);
          }
          if (c.kind === 'connect_local') {
            await gate;
            throw new Error('Bad credentials');
          }
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
          if (c.kind === 'sync') return [];
          if (c.kind === 'open_token_page') return null;
          if (c.kind === 'set_groups') {
            data.credentials.find((k) => k.id === c.credential_id)!.groups = c.groups;
            return null;
          }
          if (c.kind === 'reveal') return { fields: { token: 'only-revealed-on-request' } };
          if (c.kind === 'rotate_now') {
            const i = data.credentials.findIndex((k) => k.id === c.credential_id);
            const made = { ...data.credentials[i], id: 'rotated', origin: 'rotated' };
            data.credentials.splice(i, 1, made);
            return made;
          }
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
              created_at: 1791493200,
            };
            data.rotations.push(r as any);
            return r;
          }
          throw new Error(`Unhandled test command: ${c.kind}`);
        },
      };
    },
    {
      data:
        typeof connected === 'object'
          ? connected
          : connected
            ? withVercel(connectedSnapshot())
            : emptySnapshot(),
    },
  );
}
const noOverflow = (page: Page) =>
  page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth);

test('adding a provider shows progress, then asks for a token when local sign-ins fail', async ({
  page,
}) => {
  await mockHost(page);
  await page.goto('/');
  await expect(page.getByRole('heading', { name: 'Providers', exact: true })).toBeVisible();
  await expect(page.locator('.provider-card')).toHaveCount(4);
  await expect(page.locator('.provider-card').first()).toContainText(
    'Signed in on this device via GitHub CLI',
  );
  await page.screenshot({ path: 'artifacts/marketplace.png', fullPage: true });
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
  await page.keyboard.press('Control+k');
  const search = page.getByRole('textbox', { name: 'Search', exact: true });
  await expect(search).toBeFocused();
  await search.fill('github');
  await expect(page.locator('.provider-card')).toHaveCount(1);
  await page.getByRole('button', { name: 'Add', exact: true }).click();
  const job = page.getByRole('dialog', { name: 'Adding GitHub' });
  await expect(job.getByText('Connecting octocat')).toBeVisible();
  await expect(job.getByRole('button', { name: 'Cancel' })).toBeVisible();
  await page.screenshot({ path: 'artifacts/progress.png' });
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
  await page.evaluate(() => (window as unknown as { release: () => void }).release());
  const dialog = page.getByRole('dialog', { name: 'Connect GitHub' });
  // Why the sign-in didn't work goes to the local log, not the screen.
  await expect(dialog.getByLabel('Token', { exact: true })).toBeVisible();
  await expect(dialog.getByRole('alert')).toHaveCount(0);
  await expect(page.getByText('Bad credentials')).toHaveCount(0);
  await expect(dialog.getByLabel('Token', { exact: true })).toBeVisible();
  await page.screenshot({ path: 'artifacts/connect.png' });
  await page.keyboard.press('Escape');
  await expect(page.getByRole('dialog')).toHaveCount(0);
});
test('keys stay masked, have firm actions, and rotate from their details', async ({ page }) => {
  await mockHost(page, true);
  await page.goto('/');
  const keys = page.getByRole('list', { name: 'Keys' });
  // Keys found on this device (here, the CLI sign-ins) are listed with the rest.
  await expect(keys.getByRole('listitem')).toHaveCount(3);
  await expect(page.getByRole('button', { name: /Import/ })).toHaveCount(0);
  await expect(keys.getByRole('listitem').filter({ hasText: 'Vercel CLI' })).toBeVisible();
  for (const action of ['Reveal', 'Copy', 'Delete'])
    await expect(page.getByRole('button', { name: `${action} Production API` })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Rotate Production API' })).toHaveCount(0);
  await page.screenshot({ path: 'artifacts/keys.png', fullPage: true });
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
  await expect(page.getByText('only-revealed-on-request')).toHaveCount(0);
  await page.getByRole('button', { name: 'Reveal Production API' }).click();
  await expect(page.getByText('only-revealed-on-request')).toBeVisible();
  await page.getByRole('button', { name: 'Hide Production API' }).click();
  await expect(page.getByText('only-revealed-on-request')).toHaveCount(0);
  await page.getByRole('tab', { name: /Others/ }).click();
  await expect(keys.getByRole('listitem')).toHaveCount(1);
  await expect(keys).toContainText('No secret on this device');
  await expect(page.getByRole('button', { name: 'Fill in deploy-bot' })).toBeVisible();
  await page.screenshot({ path: 'artifacts/keys-secretless.png', fullPage: true });
  await page.getByRole('tab', { name: /Ready to use/ }).click();
  await page.getByRole('button', { name: 'Details for Production API' }).click();
  const details = page.getByRole('region', { name: 'Production API details' });
  await expect(details).toContainText('SUPABASE_URL');
  await expect(details).toContainText('Listed by Supabase in Personal projects');
  await page.screenshot({ path: 'artifacts/key-details.png', fullPage: true });
  await details.getByRole('button', { name: 'Rotate Production API' }).click();
  const rotate = page.getByRole('dialog', { name: 'Rotate Production API?' });
  await expect(rotate).toContainText('Supabase makes a new key in its place');
  await page.screenshot({ path: 'artifacts/rotate.png' });
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
  await rotate.getByRole('button', { name: 'Rotate', exact: true }).click();
  await expect(page.getByText('Key rotated')).toBeVisible();
  await expect(page.getByRole('dialog')).toHaveCount(0);
  await page.getByRole('button', { name: 'Details for Production API' }).click();
  await expect(page.getByRole('region', { name: 'Production API details' })).toContainText(
    'Made by GetKeys to replace an older key',
  );
  await page.screenshot({ path: 'artifacts/rotated.png', fullPage: true });
});
test('management keys live with their accounts under Providers', async ({ page }) => {
  await mockHost(page, true);
  await page.goto('/');
  await page.getByRole('button', { name: 'Providers', exact: true }).click();
  await page.getByRole('button', { name: 'Vercel accounts' }).click();
  const panel = page.getByRole('region', { name: 'Vercel accounts' });
  await expect(panel).toContainText('Vercel CLI');
  await expect(panel.getByRole('button', { name: /Create dedicated key/ })).toBeVisible();
  await expect(panel.getByRole('button', { name: 'Add account' })).toBeVisible();
  await page.screenshot({ path: 'artifacts/accounts.png', fullPage: true });
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
  await page.emulateMedia({ colorScheme: 'dark' });
  await page.screenshot({ path: 'artifacts/accounts-dark.png', fullPage: true });
});
test('compact layout has no horizontal overflow', async ({ page }) => {
  await mockHost(page, true);
  await page.setViewportSize({ width: 640, height: 850 });
  await page.goto('/');
  await page.getByRole('button', { name: 'Details for Production API' }).click();
  await expect(page.getByRole('region', { name: 'Production API details' })).toBeVisible();
  expect(await noOverflow(page)).toBe(true);
  await page.screenshot({ path: 'artifacts/compact-keys.png', fullPage: true });
  await page.getByRole('button', { name: 'Providers', exact: true }).click();
  await page.getByRole('button', { name: 'Supabase accounts' }).click();
  await expect(page.getByRole('region', { name: 'Supabase accounts' })).toBeVisible();
  expect(await noOverflow(page)).toBe(true);
  await page.screenshot({ path: 'artifacts/compact.png', fullPage: true });
});
test('groups, rejected keys, keys without a provider, and export', async ({ page }) => {
  const data = withVercel(connectedSnapshot());
  const [api, cli] = data.credentials;
  api.groups = ['backend'];
  cli.problem = 'Rejected by the provider. It was revoked or is wrong.';
  data.credentials.push({
    ...api,
    id: 'custom',
    connection_id: '',
    provider_id: 'custom',
    credential_type: 'custom',
    name: 'OpenAI',
    resource: '',
    resource_name: '',
    can_rotate: false,
    groups: ['backend'],
    problem: null,
    fields: [
      {
        key: 'OPENAI_API_KEY',
        label: 'OPENAI_API_KEY',
        env: 'OPENAI_API_KEY',
        secret: true,
        value: null,
        available: true,
      },
    ],
  });
  await mockHost(page, data);
  await page.goto('/');
  const keys = page.getByRole('list', { name: 'Keys' });
  await expect(keys.getByRole('listitem')).toHaveCount(3);
  await expect(keys.getByRole('listitem').filter({ hasText: 'Supabase CLI' })).toHaveCount(0);
  await expect(keys.getByRole('listitem').filter({ hasText: 'OpenAI' })).toContainText('Custom');
  await page.screenshot({ path: 'artifacts/keys-groups.png', fullPage: true });
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
  await page.getByRole('tab', { name: /^backend/ }).click();
  await expect(keys.getByRole('listitem')).toHaveCount(2);
  // Keys GetKeys can't use wait under Others with the reason, not as errors.
  await page.getByRole('tab', { name: /^Others/ }).click();
  await expect(keys.getByRole('listitem')).toHaveCount(2);
  await expect(keys.getByRole('listitem').filter({ hasText: 'Supabase CLI' })).toContainText(
    'Rejected by the provider',
  );
  await expect(keys.getByRole('listitem').filter({ hasText: 'Invalid' })).toHaveCount(0);
  await page.screenshot({ path: 'artifacts/keys-others.png', fullPage: true });
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
  await page.getByRole('button', { name: 'New group' }).click();
  await page.getByLabel('New group name').fill('staging');
  await page.keyboard.press('Enter');
  const picking = page.getByRole('dialog', { name: 'staging' });
  await expect(picking.getByRole('checkbox')).toHaveCount(5);
  await page.screenshot({ path: 'artifacts/group-new.png' });
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
  await page.keyboard.press('Escape');
  await page.getByRole('tab', { name: /^backend/ }).click();
  await page.getByRole('button', { name: 'Export keys' }).click();
  const exporting = page.getByRole('dialog', { name: 'Export backend' });
  await expect(exporting.getByRole('checkbox')).toHaveCount(3);
  await expect(exporting).toContainText('OPENAI_API_KEY');
  await page.screenshot({ path: 'artifacts/export.png' });
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
  await exporting.getByRole('button', { name: /Save \.env/ }).click();
  await expect(page.getByText('Saved 3 variables')).toBeVisible();
  await page.getByRole('button', { name: 'New key' }).click();
  const form = page.getByRole('dialog', { name: 'New key' });
  await form.getByRole('button', { name: /Any other service/ }).click();
  await form.getByRole('button', { name: 'Add value' }).click();
  await expect(form.getByLabel('Variable 2', { exact: true })).toBeVisible();
  await page.screenshot({ path: 'artifacts/custom-key.png' });
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
  await page.keyboard.press('Escape');
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  await expect(page.getByText('Vault location')).toBeVisible();
  await page.screenshot({ path: 'artifacts/settings.png', fullPage: true });
  await page.getByRole('button', { name: /^Keys/ }).click();
  await page.emulateMedia({ colorScheme: 'dark' });
  await page.getByRole('button', { name: 'Details for OpenAI' }).click();
  await page.screenshot({ path: 'artifacts/keys-dark.png', fullPage: true });
});
test('a key’s groups link to them, and Filter narrows only the tab in view', async ({ page }) => {
  const data = withVercel(connectedSnapshot());
  data.credentials[0].groups = ['backend'];
  data.credentials[1].problem = "This account isn't allowed to list it, so it can't be checked";
  await mockHost(page, data);
  await page.goto('/');
  const keys = page.getByRole('list', { name: 'Keys' });
  await expect(page.getByPlaceholder('Search keys')).toBeVisible();

  // Groups are plain links in the details, with a small + to file the key elsewhere.
  await page.getByRole('button', { name: 'Details for Production API' }).click();
  const details = page.getByRole('region', { name: 'Production API details' });
  await expect(details.getByRole('textbox')).toHaveCount(0);
  await details.getByRole('button', { name: 'Groups for Production API' }).click();
  const menu = details.getByRole('menu');
  await menu.getByRole('menuitem', { name: 'New group' }).click();
  await details.getByLabel('New group name').fill('staging');
  await page.keyboard.press('Enter');
  await expect(details.getByRole('button', { name: 'staging' })).toBeVisible();
  await page.screenshot({ path: 'artifacts/key-groups.png', fullPage: true });
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
  await details.getByRole('button', { name: 'backend' }).click();
  await expect(page.getByRole('tab', { name: /^backend/ })).toHaveAttribute(
    'aria-selected',
    'true',
  );
  await expect(keys.getByRole('listitem')).toHaveCount(1);

  // Filter applies to the tab in view and resets when the tab changes.
  await page.getByRole('tab', { name: /^Ready to use/ }).click();
  await expect(keys.getByRole('listitem')).toHaveCount(3);
  await page.getByRole('button', { name: 'Filter', exact: true }).click();
  await page.getByRole('menuitemcheckbox', { name: 'Not checked' }).click();
  await expect(keys.getByRole('listitem')).toHaveCount(1);
  await expect(keys).toContainText('Supabase CLI');
  await page.screenshot({ path: 'artifacts/filter.png', fullPage: true });
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
  await page.keyboard.press('Escape');
  await page.getByRole('tab', { name: /^backend/ }).click();
  await page.getByRole('tab', { name: /^Ready to use/ }).click();
  await expect(keys.getByRole('listitem')).toHaveCount(3);

  // Providers only offer Open once they're added.
  await page.getByRole('button', { name: 'Providers', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Open Supabase' })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Open GitHub' })).toHaveCount(0);
});
