import { test, expect, type Page } from '@playwright/test';
import AxeBuilder from '@axe-core/playwright';
import { emptySnapshot } from './fixtures';
import type { Snapshot } from '../apps/desktop/src/types';

/** A Supabase account with one project and no keys yet. */
function account(): Snapshot {
  const s = emptySnapshot();
  for (const p of s.plugins) p.installed = p.manifest.id === 'official.supabase';
  s.connections = [
    {
      id: 'sb',
      plugin_id: 'official.supabase',
      name: 'Ada',
      state: 'connected',
      config: {},
      source: 'getkeys',
    },
  ];
  return s;
}

/** A host that makes what it's asked to, with a new project that takes one check to start. */
async function host(page: Page, data: Snapshot) {
  await page.addInitScript(
    ({ data }) => {
      const w = window as any;
      const resource = (r: object) => ({
        kind: 'project',
        url: '',
        pending: false,
        note: '',
        ...r,
      });
      const projects: any[] = [resource({ id: 'oldoldoldoldoldoldol', name: 'website' })];
      w.commands = [];
      w.isTauri = true;
      w.__TAURI_INTERNALS__ = {
        invoke: async (_name: string, args: any) => {
          const c = args.command;
          w.commands.push(c);
          switch (c.kind) {
            case 'snapshot':
              return structuredClone(data);
            case 'discover':
            case 'sync':
              return [];
            case 'owners':
              return [{ id: 'ada-org', name: 'Ada' }];
            case 'resources': {
              const list = structuredClone(projects);
              for (const p of projects) p.pending = false;
              return list;
            }
            case 'create_resource': {
              const made = resource({ id: 'refrefrefrefrefrefre', name: c.name, pending: true });
              projects.push(made);
              const password = {
                id: 'db-password',
                plugin_id: '',
                connection_id: '',
                provider_id: 'custom',
                name: `${c.name} database password`,
                credential_type: 'custom',
                resource: '',
                resource_name: '',
                project: '',
                environment: '',
                created_at: null,
                expires_at: null,
                value_available: true,
                can_rotate: false,
                retired: false,
                groups: [],
                problem: null,
                fields: [
                  {
                    key: 'SUPABASE_DB_PASSWORD',
                    label: 'SUPABASE_DB_PASSWORD',
                    env: 'SUPABASE_DB_PASSWORD',
                    secret: true,
                    value: null,
                    available: true,
                  },
                ],
              };
              data.credentials.push(password as any);
              return { resource: made, generated: password };
            }
            case 'create': {
              const key = {
                id: 'created',
                plugin_id: '',
                connection_id: c.connection_id,
                provider_id: 'secret',
                name: c.name,
                credential_type: 'secret',
                resource: c.resource,
                resource_name: projects.find((p) => p.id === c.resource)?.name ?? '',
                project: '',
                environment: '',
                created_at: null,
                expires_at: null,
                value_available: true,
                can_rotate: true,
                retired: false,
                groups: [],
                problem: null,
                fields: [
                  {
                    key: 'token',
                    label: 'API key',
                    env: 'SUPABASE_KEY',
                    secret: true,
                    value: null,
                    available: true,
                  },
                ],
              };
              data.credentials.push(key as any);
              return key;
            }
          }
          throw new Error(`Unhandled test command: ${c.kind}`);
        },
      };
    },
    { data },
  );
}

const sent = (page: Page, kind: string) =>
  page.evaluate(
    (kind) =>
      (window as unknown as { commands: { kind: string }[] }).commands.filter(
        (c) => c.kind === kind,
      ),
    kind,
  );

test('new key: provider, account, a new project for it, then the key', async ({ page }) => {
  await host(page, account());
  await page.goto('/');
  await page.getByRole('button', { name: 'New key' }).first().click();
  const dialog = page.getByRole('dialog', { name: 'New key' });

  // Each step appears only once the one before it is chosen.
  await expect(dialog.getByRole('group', { name: 'Account' })).toHaveCount(0);
  await expect(dialog.getByRole('button', { name: /^GitHub/ })).toHaveCount(0);
  await expect(dialog.getByRole('button', { name: /^Supabase/ })).toBeVisible();
  await page.waitForTimeout(300);
  await page.screenshot({ path: 'artifacts/journey-new-key-provider.png' });
  expect((await new AxeBuilder({ page }).include('dialog').analyze()).violations).toEqual([]);
  await dialog.getByRole('button', { name: /^Supabase/ }).click();
  // The only account is picked by itself.
  await expect(dialog.getByRole('group', { name: 'Account' })).toHaveCount(0);
  await expect(dialog.locator('.cascade-step.done')).toContainText(['Supabase', 'Ada']);
  const project = dialog.getByRole('group', { name: 'Project' });
  await expect(project.getByRole('button', { name: /^website/ })).toBeVisible();
  await expect(dialog.getByLabel('Name', { exact: true })).toHaveCount(0);

  // A new project is made in place, and picked once it has started.
  await project.getByRole('button', { name: /New project/ }).click();
  const form = dialog.getByRole('form', { name: 'New project' });
  await form.getByLabel('Project name').fill('My App');
  await expect(form.getByLabel('Project name')).toHaveValue('my-app');
  await form.getByLabel('Region').selectOption('emea');
  await page.screenshot({ path: 'artifacts/journey-new-key-project.png' });
  expect((await new AxeBuilder({ page }).include('dialog').analyze()).violations).toEqual([]);
  await form.getByRole('button', { name: 'Create project' }).click();
  await expect(
    page.getByText('Created my-app. Its my-app database password is saved as a key.'),
  ).toBeVisible({
    timeout: 20_000,
  });
  await expect(dialog.locator('.cascade-step.done')).toContainText(['Supabase', 'Ada', 'my-app']);
  expect(await sent(page, 'create_resource')).toMatchObject([
    { connection_id: 'sb', owner: 'ada-org', name: 'my-app', options: { region: 'emea' } },
  ]);

  // Supabase makes the key, so there is nothing to paste.
  await expect(dialog.getByLabel('API key')).toHaveCount(0);
  await dialog.getByLabel('Name', { exact: true }).fill('App server');
  await page.screenshot({ path: 'artifacts/journey-new-key-name.png' });
  await dialog.getByRole('button', { name: 'Create key' }).click();
  await expect(dialog).toHaveCount(0);
  expect(await sent(page, 'create')).toMatchObject([
    { connection_id: 'sb', resource: 'refrefrefrefrefrefre', name: 'App server' },
  ]);
  const keys = page.getByRole('list', { name: 'Keys' });
  await expect(keys).toContainText('App server');
  await expect(keys).toContainText('my-app database password');
});
