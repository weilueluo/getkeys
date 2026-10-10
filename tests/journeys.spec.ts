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
        captures: any[];
        reject: boolean;
      };
      const key = (id: string, name: string, origin = 'pasted'): CredentialView => ({
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
        origin,
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
      let prefs = { mcp_enabled: false };
      let agents = [
        {
          client: 'claude_code',
          name: 'Claude Code',
          path: 'C:\\Users\\ada\\.claude.json',
          configured: false,
        },
      ];
      host.commands = [];
      host.captures = [];
      host.reject = false;
      host.isTauri = true;
      host.__TAURI_INTERNALS__ = {
        invoke: async (name, args) => {
          if (name === 'copy_credential' || name === 'export_env') return true;
          if (name === 'cancel_capture') return null;
          if (name === 'capture_key') {
            // The person makes the key on Groq's page and copies it; GetKeys picks it up.
            host.captures.push(args);
            const made = key(`key-${++saved}`, args.name, 'captured');
            if (args.replaces) made.groups = find(args.replaces).groups;
            data.credentials.push(made);
            return made;
          }
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
              const credential = key('found:env:GROQ_API_KEY', 'GROQ_API_KEY', 'found');
              data.credentials.push(credential);
              return { kind: 'key_saved', credential };
            }
            case 'check': {
              const k = find(c.credential_id);
              k.problem =
                host.reject && k.id.startsWith('found:')
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
              const credential = key(`key-${++saved}`, c.name);
              data.credentials.push(credential);
              return credential;
            }
            case 'delete':
              return { kind: 'manual', url: 'https://console.groq.com/keys' };
            case 'confirm_deleted':
              data.credentials = data.credentials.filter((k) => k.id !== c.credential_id);
              return null;
            case 'get_prefs':
              return prefs;
            case 'set_prefs':
              prefs = c.prefs;
              return prefs;
            case 'agent_status':
              return agents;
            case 'connect_agent':
            case 'disconnect_agent':
              agents = agents.map((a) =>
                a.client === c.client ? { ...a, configured: c.kind === 'connect_agent' } : a,
              );
              return agents;
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

const captures = (page: Page) =>
  page.evaluate(() => (window as unknown as { captures: any[] }).captures);

test('a provider without accounts: add, find, check, make, paste, reject, rotate, delete, remove', async ({
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
  const named = (name: string) =>
    keys.getByRole('listitem').filter({ has: page.locator('strong', { hasText: name }) });
  const found = named('GROQ_API_KEY');
  await expect(found).toContainText('Groq · On this device');
  expect(await sent(page, 'check')).toHaveLength(1);
  await page.getByRole('button', { name: 'Details for GROQ_API_KEY' }).click();
  await expect(page.getByRole('region', { name: 'GROQ_API_KEY details' })).toContainText(
    'Found on this device, in $GROQ_API_KEY',
  );
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);

  // New key starts with the provider. Groq makes keys only on its page: GetKeys opens it
  // and picks the new key up from the clipboard, so there is nothing to paste.
  await page.getByRole('button', { name: 'New key' }).click();
  let form = page.getByRole('dialog', { name: 'New key' });
  await form.getByRole('button', { name: /^Groq/ }).click();
  await expect(form.locator('.cascade-step.done')).toHaveText([/Groq/]);
  await expect(form.getByRole('button', { name: 'Change account' })).toHaveCount(0);
  await expect(form.getByLabel('API key')).toHaveCount(0);
  await form.getByLabel('Name').fill('Laptop');
  await page.screenshot({ path: 'artifacts/journey-new-key.png' });
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
  await form.getByRole('button', { name: /Make it on Groq/ }).click();
  await expect(page.getByText('Saved Laptop')).toBeVisible();
  await expect(page.getByRole('dialog')).toHaveCount(0);
  expect(await captures(page)).toEqual([
    {
      pluginId: 'official.groq',
      connectionId: null,
      resource: '',
      name: 'Laptop',
      group: null,
      replaces: null,
    },
  ]);
  const laptop = keys.getByRole('listitem').filter({ hasText: 'Laptop' });
  await page.getByRole('button', { name: 'Details for Laptop' }).click();
  await expect(page.getByRole('region', { name: 'Laptop details' })).toContainText(
    'Made on Groq’s page',
  );

  // Pasting a key made elsewhere is the fallback.
  await page.getByRole('button', { name: 'New key' }).click();
  form = page.getByRole('dialog', { name: 'New key' });
  await form.getByRole('button', { name: /^Groq/ }).click();
  await form.getByRole('button', { name: /Already have a key\? Paste it/ }).click();
  await form.getByLabel('Name').fill('Desk');
  await form.getByLabel('API key').fill('gsk_pasted');
  await form.getByRole('button', { name: 'Save key' }).click();
  await expect(page.getByRole('dialog')).toHaveCount(0);
  await expect(keys.getByRole('listitem').filter({ hasText: 'Desk' })).toContainText(
    'Groq · Added by you',
  );
  expect((await sent(page, 'save_key'))[0]).toMatchObject({
    credential_id: null,
    plugin_id: 'official.groq',
    name: 'Desk',
    fields: { token: 'gsk_pasted' },
  });

  // Reveal and copy work from the row.
  await page.getByRole('button', { name: 'Reveal Laptop' }).click();
  await expect(laptop).toContainText('gsk_value_of_key-1');
  await page.getByRole('button', { name: 'Hide Laptop' }).click();

  // Once Groq rejects the found key, a refresh moves it to Others with the reason. That's
  // not an error: the refresh still succeeds and nothing turns red.
  await page.evaluate(() => ((window as unknown as { reject: boolean }).reject = true));
  await page.getByRole('button', { name: 'Refresh keys' }).click();
  await expect(page.getByText('Keys are up to date')).toBeVisible();
  await expect(found).toHaveCount(0);
  await expect(page.getByRole('alert')).toHaveCount(0);
  await page.getByRole('tab', { name: /^Others/ }).click();
  await expect(keys.getByRole('listitem')).toHaveCount(1);
  await expect(found).toContainText('Rejected by the provider');
  await expect(found).not.toContainText('Invalid');
  await page.screenshot({ path: 'artifacts/journey-others.png', fullPage: true });
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
  await page.evaluate(() => ((window as unknown as { reject: boolean }).reject = false));

  // Rotating it opens Groq's page, keeps the new key and deletes the old one.
  await page.getByRole('button', { name: 'Details for GROQ_API_KEY' }).click();
  await page.getByRole('button', { name: 'Rotate GROQ_API_KEY' }).click();
  const rotate = page.getByRole('dialog', { name: 'Rotate GROQ_API_KEY' });
  await page.screenshot({ path: 'artifacts/journey-rotate.png' });
  await rotate.getByRole('button', { name: /Make it on Groq/ }).click();
  expect((await captures(page))[1]).toMatchObject({
    name: 'GROQ_API_KEY',
    replaces: 'found:env:GROQ_API_KEY',
  });
  // Groq deletes keys only on its page, so GetKeys asks for that last step.
  const retire = page.getByRole('dialog', { name: 'Delete GROQ_API_KEY in Groq' });
  await retire.getByRole('button', { name: 'Done' }).click();
  await expect(page.getByRole('dialog')).toHaveCount(0);
  expect(await sent(page, 'confirm_deleted')).toEqual([
    { kind: 'confirm_deleted', credential_id: 'found:env:GROQ_API_KEY' },
  ]);
  await page.getByRole('tab', { name: /^Ready to use/ }).click();
  await expect(named('GROQ_API_KEY')).toHaveCount(1);
  await expect(page.getByRole('tab', { name: /^Others/ })).toHaveCount(0);

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
  await expect(removing).toContainText('2 keys from Groq');
  await removing.getByRole('button', { name: 'Remove Groq' }).click();
  await page.getByRole('button', { name: /^Keys/ }).click();
  await expect(keys.getByRole('listitem')).toHaveCount(0);
});

test('an agent gets your keys once connected, and stops once disconnected', async ({ page }) => {
  await statefulHost(page, snapshot());
  await page.goto('/');
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  const connect = page.getByRole('button', { name: 'Connect Claude Code' });
  await expect(connect).toBeDisabled();
  await page.getByRole('switch', { name: 'Allow agents' }).click();
  await expect(page.getByText('Agents can use your keys')).toBeVisible();
  await connect.click();
  await expect(page.getByText('Added GetKeys to Claude Code')).toBeVisible();
  await expect(page.getByRole('button', { name: 'Reconnect Claude Code' })).toBeVisible();
  await page.screenshot({ path: 'artifacts/agent-connected.png', fullPage: true });
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
  await page.getByRole('button', { name: 'Disconnect Claude Code' }).click();
  await expect(page.getByText('Removed GetKeys from Claude Code')).toBeVisible();
  await expect(page.getByRole('button', { name: 'Connect Claude Code' })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Disconnect Claude Code' })).toHaveCount(0);
  expect(await sent(page, 'disconnect_agent')).toEqual([
    { kind: 'disconnect_agent', client: 'claude_code' },
  ]);
});
