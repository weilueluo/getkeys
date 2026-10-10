import { test, expect, type Page } from '@playwright/test';
import AxeBuilder from '@axe-core/playwright';
import { emptySnapshot } from './fixtures';
import type { CredentialView, Manifest, Snapshot } from '../apps/desktop/src/types';

/** A provider that needs no account: its keys come from this device or are pasted in. */
const groq = {
  id: 'official.groq',
  name: 'Groq',
  publisher: 'GetKeys',
  version: '1.0.0',
  description: 'Fast inference for open models.',
  homepage: 'https://groq.com',
  kind: 'declarative',
  category: 'ai',
  hosts: ['api.groq.com'],
  operations: [],
  fields: [],
  account_fields: [],
  account_credentials: false,
  token_page: 'https://console.groq.com/keys',
  key_pattern: 'gsk_[A-Za-z0-9]{52}',
  sources: [],
  credential_fields: [
    {
      key: 'token',
      label: 'API key',
      env: 'GROQ_API_KEY',
      secret: true,
      required: true,
      placeholder: 'gsk_…',
    },
  ],
} as unknown as Manifest;

function snapshot(): Snapshot {
  const s = emptySnapshot();
  s.plugins.push({
    manifest: groq,
    installed: false,
    update_available: false,
    browser_sign_in: false,
  });
  return s;
}

/** A host that keeps state across commands, so a whole journey can run against it. */
async function statefulHost(page: Page, data: Snapshot) {
  await page.addInitScript(
    ({ data }) => {
      const host = window as typeof window & {
        __TAURI_INTERNALS__: { invoke: (name: string, args: any) => Promise<unknown> };
        isTauri: boolean;
        commands: any[];
        reject: boolean;
      };
      const key = (id: string, name: string): CredentialView => ({
        id,
        plugin_id: 'official.groq',
        connection_id: '',
        provider_id: 'local',
        name,
        credential_type: 'api_key',
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
            key: 'token',
            label: 'API key',
            env: 'GROQ_API_KEY',
            secret: true,
            value: null,
            available: true,
          },
        ],
      });
      const find = (id: string) => data.credentials.find((c) => c.id === id)!;
      let saved = 0;
      host.commands = [];
      host.reject = false;
      host.isTauri = true;
      host.__TAURI_INTERNALS__ = {
        invoke: async (name, args) => {
          if (name === 'copy_credential' || name === 'export_env') return true;
          if (name.startsWith('plugin:')) return false;
          const c = args.command;
          host.commands.push(c);
          switch (c.kind) {
            case 'snapshot':
              return structuredClone(data);
            case 'discover':
              return [];
            case 'scan_sources': {
              const plugin = data.plugins.find((p) => p.manifest.id === c.plugin_id);
              if (plugin) plugin.installed = true;
              return data.credentials.some((k) => k.id === 'found:env:GROQ_API_KEY')
                ? []
                : [
                    {
                      id: 'env:GROQ_API_KEY',
                      plugin_id: 'official.groq',
                      source: 'GROQ_API_KEY',
                      account: '',
                      connected: false,
                      key: true,
                    },
                  ];
            }
            case 'connect_local': {
              const credential = key('found:env:GROQ_API_KEY', 'GROQ_API_KEY');
              data.credentials.push(credential);
              return { kind: 'key_saved', credential };
            }
            case 'check': {
              const k = find(c.credential_id);
              k.problem = host.reject
                ? 'Rejected by the provider. It was revoked or is wrong.'
                : null;
              return k;
            }
            case 'set_groups':
              find(c.credential_id).groups = c.groups;
              return null;
            case 'reveal':
              return { fields: { token: `gsk_value_of_${c.credential_id}` } };
            case 'open_token_page':
              return null;
            case 'save_key': {
              if (c.credential_id) {
                const k = find(c.credential_id);
                k.problem = null;
                k.groups = k.groups.filter((g) => g !== 'Failed');
                return k;
              }
              const credential = key(`key-${++saved}`, c.name);
              data.credentials.push(credential);
              return credential;
            }
            case 'delete':
              return { kind: 'manual', url: 'https://console.groq.com/keys' };
            case 'confirm_deleted':
              data.credentials = data.credentials.filter((k) => k.id !== c.credential_id);
              return null;
            case 'set_project_folders':
              data.project_folders = c.folders;
              return null;
            case 'get_prefs':
              return { mcp_enabled: false };
            case 'agent_status':
              return [];
            case 'uninstall':
              data.plugins.find((p) => p.manifest.id === c.plugin_id)!.installed = false;
              data.credentials = data.credentials.filter((k) => k.plugin_id !== c.plugin_id);
              return null;
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

test('a provider without accounts: add, find, check, add by hand, rotate, delete, remove', async ({
  page,
}) => {
  await statefulHost(page, snapshot());
  await page.goto('/');
  const keys = page.getByRole('list', { name: 'Keys' });

  // Adding the provider saves the key found on this device and lands on Keys.
  await page.getByRole('textbox', { name: 'Search', exact: true }).fill('groq');
  const card = page.locator('.provider-card').filter({ hasText: 'Groq' });
  await expect(card).toContainText('AI');
  await card.getByRole('button', { name: 'Add', exact: true }).click();
  await expect(page.getByText('Added Groq · 1 key')).toBeVisible();
  const found = keys.getByRole('listitem').filter({ hasText: 'GROQ_API_KEY' });
  await expect(found).toContainText('Groq · On this device');
  expect(await sent(page, 'check')).toHaveLength(1);
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);

  // A refresh checks the key again; once Groq rejects it, it moves to Failed.
  await page.evaluate(() => ((window as unknown as { reject: boolean }).reject = true));
  await page.getByRole('button', { name: 'Refresh keys' }).click();
  await expect(page.getByText('moved to Failed')).toBeVisible();
  await expect(found).toContainText('Invalid');
  await page.getByRole('tab', { name: /^Failed/ }).click();
  await expect(keys.getByRole('listitem')).toHaveCount(1);
  await page.getByRole('tab', { name: /^Ready to use/ }).click();
  await page.evaluate(() => ((window as unknown as { reject: boolean }).reject = false));

  // New key offers the provider on its own, with no account to pick.
  await page.getByRole('button', { name: 'New key' }).click();
  const form = page.getByRole('dialog', { name: 'New key' });
  await expect(form.getByLabel('Provider')).toHaveValue('plugin:official.groq');
  await form.getByLabel('Name').fill('Laptop');
  await form.getByLabel('API key').fill('gsk_pasted');
  await page.screenshot({ path: 'artifacts/journey-new-key.png' });
  await form.getByRole('button', { name: 'Save key' }).click();
  await expect(page.getByRole('dialog')).toHaveCount(0);
  const laptop = keys.getByRole('listitem').filter({ hasText: 'Laptop' });
  await expect(laptop).toContainText('Groq · Added by you');
  expect((await sent(page, 'save_key'))[0]).toMatchObject({
    credential_id: null,
    plugin_id: 'official.groq',
    name: 'Laptop',
    fields: { token: 'gsk_pasted' },
  });

  // Reveal and copy work from the row.
  await page.getByRole('button', { name: 'Reveal Laptop' }).click();
  await expect(laptop).toContainText('gsk_value_of_key-1');
  await page.getByRole('button', { name: 'Hide Laptop' }).click();

  // Rotating opens Groq's key page and takes the new value in place of the old.
  await page.getByRole('button', { name: 'Rotate GROQ_API_KEY' }).click();
  const rotate = page.getByRole('dialog', { name: 'Rotate GROQ_API_KEY' });
  await rotate.getByRole('button', { name: 'Open Groq' }).click();
  expect(await sent(page, 'open_token_page')).toHaveLength(1);
  await rotate.getByLabel('New API key').fill('gsk_rotated');
  await page.screenshot({ path: 'artifacts/journey-rotate.png' });
  await rotate.getByRole('button', { name: 'Save new key' }).click();
  await expect(page.getByText('Key rotated')).toBeVisible();
  await expect(found).not.toContainText('Invalid');
  expect((await sent(page, 'save_key'))[1]).toMatchObject({
    credential_id: 'found:env:GROQ_API_KEY',
    fields: { token: 'gsk_rotated' },
  });

  // Deleting sends the person to Groq, then forgets the key once they're done.
  await page.getByRole('button', { name: 'Delete Laptop' }).click();
  const deleting = page.getByRole('dialog', { name: 'Delete Laptop?' });
  await expect(deleting).toContainText('opens Groq');
  await deleting.getByRole('button', { name: 'Delete key' }).click();
  await page
    .getByRole('dialog', { name: 'Delete Laptop in Groq' })
    .getByRole('button', { name: 'Done' })
    .click();
  await expect(laptop).toHaveCount(0);

  // Removing the provider from Providers forgets its keys.
  await page.getByRole('button', { name: 'Providers', exact: true }).click();
  await page.getByRole('button', { name: 'Groq details' }).click();
  await expect(page.getByRole('region', { name: 'Groq details' })).toContainText('need no account');
  await page.screenshot({ path: 'artifacts/journey-provider.png', fullPage: true });
  await page.getByRole('button', { name: 'Remove provider' }).click();
  const removing = page.getByRole('dialog', { name: 'Remove Groq?' });
  await expect(removing).toContainText('1 key from Groq');
  await removing.getByRole('button', { name: 'Remove Groq' }).click();
  await page.getByRole('button', { name: /^Keys/ }).click();
  await expect(keys.getByRole('listitem')).toHaveCount(0);
});

test('project folders: add a folder for refresh to read .env files from, then remove it', async ({
  page,
}) => {
  await statefulHost(page, snapshot());
  await page.goto('/');
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  await expect(page.getByText('No folders yet.')).toBeVisible();
  await page.getByRole('button', { name: 'Add folder' }).click();
  const adding = page.getByRole('dialog', { name: 'Add a project folder' });
  await adding.getByLabel('Folder path').fill('C:\\code');
  await adding.getByRole('button', { name: 'Add folder' }).click();
  await expect(adding).toHaveCount(0);
  await expect(page.getByText('C:\\code', { exact: true })).toBeVisible();
  expect(await sent(page, 'set_project_folders')).toEqual([
    { kind: 'set_project_folders', folders: ['C:\\code'] },
  ]);
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
  await page.getByRole('button', { name: 'Remove C:\\code' }).click();
  await expect(page.getByText('No folders yet.')).toBeVisible();
});
