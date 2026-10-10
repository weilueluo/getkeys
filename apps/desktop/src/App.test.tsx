import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { App } from './App';
import { copyCredential, exportEnv, request } from './bridge';
import { waiting } from './NewKey';
import { connectedSnapshot, emptySnapshot } from '../../../tests/fixtures';
import type {
  AuthProgress,
  Command,
  CredentialView,
  Deletion,
  Discovery,
  Resource,
  Snapshot,
} from './types';

vi.mock('./bridge', () => ({
  request: vi.fn(),
  copyCredential: vi.fn().mockResolvedValue(undefined),
  exportEnv: vi.fn().mockResolvedValue(true),
}));
let snapshot: Snapshot;
let found: Discovery[];
let listed: (command: { connection_id: string; resource: string }) => Promise<void> | void;
let deletion: Deletion;
let resources: Resource[];
const NEW_REF = 'newprojectrefabcdefg';
const resource = (r: Partial<Resource>): Resource => ({
  id: '',
  name: '',
  kind: 'project',
  url: '',
  pending: false,
  note: '',
  ...r,
});
const connected = (pluginId: string, name: string, source: string): AuthProgress => {
  const connection = {
    id: `connection-${name}`,
    plugin_id: pluginId,
    name,
    state: 'connected' as const,
    config: {},
    source,
  };
  snapshot.connections.push(connection);
  return { kind: 'connected', connection };
};
const key = (c: Partial<CredentialView>): CredentialView => ({
  id: 'key',
  plugin_id: '',
  connection_id: 'connection',
  provider_id: 'key',
  name: 'Key',
  credential_type: 'access_token',
  resource: '',
  resource_name: '',
  project: '',
  environment: '',
  created_at: null,
  expires_at: null,
  value_available: false,
  can_rotate: false,
  retired: false,
  groups: [],
  problem: null,
  fields: [{ key: 'token', label: 'Token', env: '', secret: true, value: null, available: false }],
  ...c,
});
/** Adds a Vercel account signed in with the CLI, and a token Vercel lists without its secret. */
const withVercel = () => {
  snapshot.plugins[2].installed = true;
  snapshot.connections.push({
    id: 'vercel',
    plugin_id: 'official.vercel',
    name: 'luo',
    state: 'connected',
    config: {},
    source: 'vercel',
  });
  snapshot.credentials.push(
    key({
      id: 'signin-vercel',
      connection_id: 'vercel',
      provider_id: 'tok_cli',
      name: 'Vercel CLI',
      credential_type: 'sign_in',
      value_available: true,
      fields: [
        { key: 'token', label: 'Token', env: '', secret: true, value: null, available: true },
      ],
    }),
    key({ id: 'deploy', connection_id: 'vercel', provider_id: 'tok_deploy', name: 'deploy-bot' }),
  );
};
beforeEach(() => {
  snapshot = emptySnapshot();
  found = [];
  listed = () => {};
  deletion = { kind: 'deleted' };
  resources = [resource({ id: 'abcdefghijklmnopqrst', name: 'GetKeys' })];
  waiting.every = 1;
  vi.clearAllMocks();
  HTMLDialogElement.prototype.showModal = function () {
    this.setAttribute('open', '');
  };
  HTMLDialogElement.prototype.close = function () {
    this.removeAttribute('open');
  };
  vi.mocked(request).mockImplementation(async (command: Command) => {
    if (command.kind === 'snapshot') return structuredClone(snapshot);
    if (command.kind === 'discover') return found;
    if (command.kind === 'scan_sources') {
      const plugin = snapshot.plugins.find((p) => p.manifest.id === command.plugin_id);
      if (plugin) plugin.installed = true;
      return found.filter(
        (d) => !d.connected && (!command.plugin_id || d.plugin_id === command.plugin_id),
      );
    }
    if (command.kind === 'connect_local') {
      const d = found.find((f) => f.id === command.source_id)!;
      if (d.id.endsWith(':revoked')) throw new Error('Bad credentials');
      d.connected = true;
      return connected(d.plugin_id, d.account || d.source, d.id);
    }
    if (command.kind === 'connect_token') return connected(command.plugin_id, 'octocat', 'token');
    if (command.kind === 'open_token_page') return null;
    if (command.kind === 'sync') {
      await listed(command);
      return [];
    }
    if (command.kind === 'install') {
      snapshot.plugins.find((p) => p.manifest.id === command.plugin_id)!.installed = true;
      return null;
    }
    if (command.kind === 'begin_auth')
      return {
        kind: 'browser',
        flow_id: 'flow',
        url: 'https://github.com/login/device',
        user_code: 'ABCD-1234',
        interval: 5,
      };
    if (command.kind === 'resources') return resources;
    if (command.kind === 'create') {
      const made = key({
        id: 'created',
        connection_id: command.connection_id,
        resource: command.resource,
        name: command.name,
        value_available: true,
      });
      snapshot.credentials.push(made);
      return made;
    }
    if (command.kind === 'owners') {
      const plugin = snapshot.connections.find((c) => c.id === command.connection_id)?.plugin_id;
      return plugin === 'official.github'
        ? [
            { id: '', name: 'octocat' },
            { id: 'acme', name: 'acme' },
          ]
        : plugin === 'official.supabase'
          ? [{ id: 'personal-org', name: 'Personal' }]
          : [{ id: '', name: 'luo' }];
    }
    if (command.kind === 'create_resource') {
      const plugin = snapshot.connections.find((c) => c.id === command.connection_id)!.plugin_id;
      if (plugin === 'official.github')
        return {
          resource: resource({
            id: `octocat/${command.name}`,
            name: `octocat/${command.name}`,
            kind: 'repository',
            url: `https://github.com/octocat/${command.name}`,
          }),
          generated: null,
        };
      if (plugin === 'official.supabase') {
        const password = key({
          id: 'db-password',
          connection_id: '',
          provider_id: 'custom',
          name: `${command.name} database password`,
          credential_type: 'custom',
          value_available: true,
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
        });
        snapshot.credentials.push(password);
        resources.push(resource({ id: NEW_REF, name: command.name }));
        return {
          resource: resource({
            id: NEW_REF,
            name: command.name,
            pending: true,
            url: `https://supabase.com/dashboard/project/${NEW_REF}`,
          }),
          generated: password,
        };
      }
      return { resource: resource({ id: 'prj_1', name: command.name }), generated: null };
    }
    if (command.kind === 'reveal') return { fields: { token: 'test-secret-never-in-inventory' } };
    if (command.kind === 'import') return snapshot.credentials[0];
    if (command.kind === 'save_key') {
      const saved = key({
        id: 'own-1',
        plugin_id: command.plugin_id,
        connection_id: '',
        name: command.name,
        value_available: true,
      });
      snapshot.credentials.push(saved);
      return saved;
    }
    if (command.kind === 'delete') {
      if (deletion.kind === 'deleted')
        snapshot.credentials = snapshot.credentials.filter((c) => c.id !== command.credential_id);
      return deletion;
    }
    if (command.kind === 'confirm_deleted') {
      snapshot.credentials = snapshot.credentials.filter((c) => c.id !== command.credential_id);
      return null;
    }
    if (command.kind === 'dedicate') {
      const c = snapshot.connections.find((c) => c.id === command.connection_id)!;
      c.source = 'getkeys';
      return c;
    }
    if (command.kind === 'disconnect') {
      snapshot.connections.find((c) => c.id === command.connection_id)!.state = 'disconnected';
      return null;
    }
    if (command.kind === 'save_custom') {
      const saved = key({
        id: 'custom-1',
        connection_id: '',
        provider_id: 'custom',
        name: command.name,
        credential_type: 'custom',
        value_available: true,
        fields: command.fields.map((f) => ({
          key: f.variable,
          label: f.variable,
          env: f.variable,
          secret: f.secret,
          value: f.secret ? null : f.value,
          available: true,
        })),
      });
      snapshot.credentials.push(saved);
      return saved;
    }
    if (command.kind === 'set_groups') {
      const c = snapshot.credentials.find((c) => c.id === command.credential_id)!;
      c.groups = command.groups;
      return c;
    }
    if (command.kind === 'uninstall') {
      const ids = snapshot.connections
        .filter((c) => c.plugin_id === command.plugin_id)
        .map((c) => c.id);
      snapshot.connections = snapshot.connections.filter((c) => !ids.includes(c.id));
      snapshot.credentials = snapshot.credentials.filter((c) => !ids.includes(c.connection_id));
      snapshot.plugins.find((p) => p.manifest.id === command.plugin_id)!.installed = false;
      return null;
    }
    if (command.kind === 'check' || command.kind === 'open_vault_folder') return null;
    throw new Error(`Unhandled test command: ${command.kind}`);
  });
});
const commands = () => vi.mocked(request).mock.calls.map(([c]) => c);
const card = async (name: string) =>
  (await screen.findByRole('heading', { name })).closest('article')!;
const item = async (name: string) =>
  (await screen.findByRole('button', { name: `Details for ${name}` })).closest('li')!;
const enabled = async (button: HTMLElement) => {
  await waitFor(() => expect(button).toBeEnabled());
  fireEvent.click(button);
};
const expand = async (name: string) => {
  await enabled(await screen.findByRole('button', { name: `Details for ${name}` }));
  return screen.getByRole('region', { name: `${name} details` });
};
const accounts = async (provider: string) => {
  fireEvent.click(await screen.findByRole('button', { name: 'Providers' }));
  await enabled(await screen.findByRole('button', { name: `${provider} accounts` }));
  return screen.getByRole('region', { name: `${provider} accounts` });
};

describe('GetKeys workflows', () => {
  it('adds a provider by connecting its local sign-ins and listing their keys', async () => {
    found = [
      {
        id: 'supabase:cli',
        plugin_id: 'official.supabase',
        source: 'Supabase CLI',
        account: 'Personal projects',
        connected: false,
        key: false,
      },
    ];
    listed = ({ connection_id }) => {
      snapshot.credentials = connectedSnapshot().credentials.map((c) => ({ ...c, connection_id }));
    };
    render(<App />);
    fireEvent.click(within(await card('Supabase')).getByRole('button', { name: 'Add' }));
    expect(await screen.findByText('Added Supabase · 2 keys')).toBeInTheDocument();
    expect(screen.getByRole('heading', { name: 'Keys' })).toBeInTheDocument();
    expect(commands()).toContainEqual({ kind: 'scan_sources', plugin_id: 'official.supabase' });
    expect(commands()).toContainEqual({
      kind: 'sync',
      connection_id: 'connection-Personal projects',
      resource: 'abcdefghijklmnopqrst',
    });
    expect(commands().some((c) => c.kind === 'begin_auth' || c.kind === 'connect_token')).toBe(
      false,
    );
    const list = screen.getByRole('list', { name: 'Keys' });
    expect(within(list).getAllByRole('listitem')).toHaveLength(2);
    expect(await item('Supabase CLI')).toHaveTextContent('Supabase · Personal projects');
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    const panel = await accounts('Supabase');
    expect(within(panel).getByText('Your Supabase CLI · listed in Keys')).toBeInTheDocument();
    expect(within(panel).getByText('Management key')).toBeInTheDocument();
    expect(within(panel).getByRole('button', { name: 'Add account' })).toBeInTheDocument();
  });
  it('asks for a token only when nothing on this device works', async () => {
    found = [
      {
        id: 'github:revoked',
        plugin_id: 'official.github',
        source: 'GitHub CLI',
        account: 'octocat',
        connected: false,
        key: false,
      },
    ];
    render(<App />);
    fireEvent.click(within(await card('GitHub')).getByRole('button', { name: 'Add' }));
    const dialog = await screen.findByRole('dialog', { name: 'Connect GitHub' });
    expect(within(dialog).getByRole('alert')).toHaveTextContent(
      'The GitHub sign-in on this device didn’t work (Error: Bad credentials)',
    );
    fireEvent.click(within(dialog).getByRole('button', { name: /Open GitHub tokens/ }));
    fireEvent.change(within(dialog).getByLabelText('Token'), {
      target: { value: 'fixture-token' },
    });
    await enabled(within(dialog).getByRole('button', { name: 'Connect' }));
    expect(await screen.findByText('Added octocat · 0 keys')).toBeInTheDocument();
    expect(commands().some((c) => c.kind === 'open_token_page')).toBe(true);
    expect(commands().find((c) => c.kind === 'connect_token')).toMatchObject({
      plugin_id: 'official.github',
      fields: { token: 'fixture-token' },
    });
  });
  it('offers browser sign-in only when it is configured', async () => {
    snapshot.plugins[1].browser_sign_in = true;
    render(<App />);
    fireEvent.click(within(await card('AWS')).getByRole('button', { name: 'Add' }));
    const dialog = await screen.findByRole('dialog', { name: 'Connect AWS' });
    expect(within(dialog).getByText('Sign in with IAM Identity Center')).toBeInTheDocument();
    expect(within(dialog).getByLabelText('IAM Identity Center start URL')).toBeRequired();
    expect(within(dialog).getByLabelText('SSO region')).toHaveValue('');
    expect(commands().some((c) => c.kind === 'begin_auth')).toBe(false);
    fireEvent.click(screen.getByRole('button', { name: 'Close dialog' }));
    await enabled(within(await card('GitHub')).getByRole('button', { name: 'Add' }));
    expect(
      within(await screen.findByRole('dialog', { name: 'Connect GitHub' })).queryByText(
        /Sign in with/,
      ),
    ).not.toBeInTheDocument();
  });
  it('lists each key with its provider, a masked value, and its actions', async () => {
    snapshot = connectedSnapshot();
    render(<App />);
    const key = await item('Production API');
    expect(key).toHaveTextContent('Supabase · GetKeys');
    expect(key).toHaveTextContent('••••');
    for (const action of ['Reveal', 'Copy', 'Rotate', 'Delete'])
      expect(
        within(key).getByRole('button', { name: `${action} Production API` }),
      ).toBeInTheDocument();
    expect(screen.queryByText(/Stored/)).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /Import/ })).not.toBeInTheDocument();
    await enabled(within(key).getByRole('button', { name: 'Copy Production API' }));
    const details = await expand('Production API');
    expect(within(details).getByText('SUPABASE_KEY')).toBeInTheDocument();
    expect(
      within(details).getByText('https://abcdefghijklmnopqrst.supabase.co'),
    ).toBeInTheDocument();
    await enabled(within(details).getByRole('button', { name: 'Copy Project URL' }));
    await enabled(within(details).getByRole('button', { name: /Copy \.env/ }));
    await waitFor(() =>
      expect(vi.mocked(copyCredential).mock.calls).toEqual([
        ['credential', 'token'],
        ['credential', 'url'],
        ['credential', '.env'],
      ]),
    );
    expect(screen.queryByText('test-secret-never-in-inventory')).not.toBeInTheDocument();
  });
  it('lists keys found on this device and changes them where they live', async () => {
    snapshot = connectedSnapshot();
    render(<App />);
    const cli = await item('Supabase CLI');
    expect(cli).toHaveTextContent('Supabase · Personal projects');
    await enabled(within(cli).getByRole('button', { name: 'Rotate Supabase CLI' }));
    let dialog = await screen.findByRole('dialog', { name: 'Rotate Supabase CLI' });
    await enabled(within(dialog).getByRole('button', { name: /Open Supabase/ }));
    expect(commands()).toContainEqual({ kind: 'open_token_page', plugin_id: 'official.supabase' });
    fireEvent.click(within(dialog).getByRole('button', { name: 'Close dialog' }));
    await enabled(within(cli).getByRole('button', { name: 'Delete Supabase CLI' }));
    dialog = await screen.findByRole('dialog', { name: 'Delete Supabase CLI?' });
    await enabled(within(dialog).getByRole('button', { name: 'Remove account' }));
    expect(await screen.findByText('Account removed')).toBeInTheDocument();
    expect(commands()).toContainEqual({ kind: 'disconnect', connection_id: 'connection' });
    expect(commands().some((c) => c.kind === 'delete' || c.kind === 'plan_rotation')).toBe(false);
  });
  it('keeps keys the provider lists without a secret in their own tab', async () => {
    snapshot = connectedSnapshot();
    withVercel();
    render(<App />);
    await item('Production API');
    expect(screen.queryByRole('button', { name: 'Details for deploy-bot' })).toBeNull();
    fireEvent.click(screen.getByRole('tab', { name: /Empty profiles/ }));
    const row = await item('deploy-bot');
    expect(row).toHaveTextContent('Vercel · luo');
    expect(within(row).getByRole('button', { name: 'Fill in deploy-bot' })).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Details for Production API' })).toBeNull();
  });
  it('keeps secret values out of inventory and hides an explicit reveal on blur', async () => {
    snapshot = connectedSnapshot();
    render(<App />);
    const key = await item('Production API');
    expect(commands().some((c) => c.kind === 'reveal')).toBe(false);
    const reveal = within(key).getByRole('button', { name: 'Reveal Production API' });
    await waitFor(() => expect(reveal).toBeEnabled());
    fireEvent.click(reveal);
    expect(await within(key).findByText('test-secret-never-in-inventory')).toBeInTheDocument();
    fireEvent.blur(window);
    await waitFor(() =>
      expect(screen.queryByText('test-secret-never-in-inventory')).not.toBeInTheDocument(),
    );
  });
  it('surfaces host failures without replacing them with demo data', async () => {
    vi.mocked(request).mockRejectedValue(new Error('OS credential store unavailable'));
    render(<App />);
    expect(await screen.findByRole('alert')).toHaveTextContent('OS credential store unavailable');
    expect(screen.queryByRole('heading', { name: 'GitHub' })).not.toBeInTheDocument();
  });
  it('refreshes every provider for new local and remote keys', async () => {
    snapshot = connectedSnapshot();
    render(<App />);
    await enabled(await screen.findByRole('button', { name: 'Refresh keys' }));
    expect(await screen.findByText('Keys are up to date')).toBeInTheDocument();
    expect(commands()).toContainEqual({ kind: 'scan_sources', plugin_id: null });
    expect(commands()).toContainEqual({
      kind: 'sync',
      connection_id: 'connection',
      resource: 'abcdefghijklmnopqrst',
    });
  });
  it('shows refresh progress and stops after the current step when cancelled', async () => {
    snapshot = connectedSnapshot();
    let finish = () => {};
    listed = () => new Promise<void>((resolve) => (finish = resolve));
    render(<App />);
    await enabled(await screen.findByRole('button', { name: 'Refresh keys' }));
    const dialog = await screen.findByRole('dialog', { name: 'Refreshing keys' });
    expect(
      await within(dialog).findByText('Listing keys in Supabase · Personal projects'),
    ).toBeInTheDocument();
    fireEvent.click(within(dialog).getByRole('button', { name: 'Cancel' }));
    expect(within(dialog).getByRole('button', { name: 'Stopping…' })).toBeDisabled();
    finish();
    expect(await screen.findByText('Stopped')).toBeInTheDocument();
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  });
  it('revokes a deleted key with its provider', async () => {
    snapshot = connectedSnapshot();
    render(<App />);
    await enabled(
      within(await item('Production API')).getByRole('button', {
        name: 'Delete Production API',
      }),
    );
    const dialog = screen.getByRole('dialog', { name: 'Delete Production API?' });
    expect(dialog).toHaveTextContent('GetKeys revokes it in Supabase');
    await enabled(within(dialog).getByRole('button', { name: 'Delete key' }));
    expect(await screen.findByText('Key deleted')).toBeInTheDocument();
    expect(commands()).toContainEqual({ kind: 'delete', credential_id: 'credential' });
    expect(screen.queryByRole('button', { name: 'Details for Production API' })).toBeNull();
  });
  it('walks through deleting a key the provider won’t let GetKeys revoke', async () => {
    snapshot = connectedSnapshot();
    deletion = { kind: 'manual', url: 'https://supabase.com/dashboard' };
    render(<App />);
    await enabled(
      within(await item('Production API')).getByRole('button', {
        name: 'Delete Production API',
      }),
    );
    await enabled(screen.getByRole('button', { name: 'Delete key' }));
    const dialog = await screen.findByRole('dialog', { name: 'Delete Production API in Supabase' });
    await enabled(within(dialog).getByRole('button', { name: 'Done' }));
    expect(await screen.findByText('Key deleted')).toBeInTheDocument();
    expect(commands()).toContainEqual({ kind: 'confirm_deleted', credential_id: 'credential' });
  });
  it('creates a dedicated management key for an account', async () => {
    withVercel();
    render(<App />);
    const panel = await accounts('Vercel');
    expect(within(panel).getByText('Vercel CLI')).toBeInTheDocument();
    await enabled(within(panel).getByRole('button', { name: /Create dedicated key/ }));
    expect(await screen.findByText('Dedicated management key created')).toBeInTheDocument();
    expect(commands()).toContainEqual({ kind: 'dedicate', connection_id: 'vercel' });
    expect(await within(panel).findByRole('button', { name: /Rotate key/ })).toBeInTheDocument();
  });
  it('reveals New key one step at a time, starting with the provider', async () => {
    snapshot = connectedSnapshot();
    render(<App />);
    await enabled(await screen.findByRole('button', { name: 'New key' }));
    const dialog = screen.getByRole('dialog', { name: 'New key' });
    const providers = within(dialog).getByRole('group', { name: 'Provider' });
    expect(within(providers).getByRole('button', { name: /^Supabase/ })).toBeInTheDocument();
    expect(
      within(providers).getByRole('button', { name: /Any other service/ }),
    ).toBeInTheDocument();
    // Providers that aren't added aren't offered.
    expect(within(providers).queryByRole('button', { name: /^GitHub/ })).toBeNull();
    expect(within(dialog).queryByRole('group', { name: 'Account' })).toBeNull();
    expect(within(dialog).queryByLabelText('Name')).toBeNull();

    fireEvent.click(within(providers).getByRole('button', { name: /^Supabase/ }));
    const account = within(dialog).getByRole('group', { name: 'Account' });
    expect(within(dialog).queryByRole('group', { name: 'Project' })).toBeNull();
    fireEvent.click(within(account).getByRole('button', { name: /^Personal projects/ }));
    const project = await within(dialog).findByRole('group', { name: 'Project' });
    expect(within(project).getByRole('button', { name: /New project/ })).toBeInTheDocument();
    fireEvent.click(within(project).getByRole('button', { name: /^GetKeys/ }));
    const way = within(dialog).getByRole('group', { name: 'Key' });
    expect(within(dialog).queryByLabelText('Name')).toBeNull();
    fireEvent.click(within(way).getByRole('button', { name: /Create a new key/ }));
    expect(within(dialog).queryByLabelText('API key')).toBeNull();
    fireEvent.change(within(dialog).getByLabelText('Name'), { target: { value: 'Staging' } });
    await enabled(within(dialog).getByRole('button', { name: 'Create key' }));
    expect(await screen.findByText('Created Staging')).toBeInTheDocument();
    expect(commands()).toContainEqual({
      kind: 'create',
      connection_id: 'connection',
      resource: 'abcdefghijklmnopqrst',
      name: 'Staging',
    });
    expect(commands().some((c) => c.kind === 'import')).toBe(false);
  });
  it('goes back a step with Change, and pastes a key the provider checks', async () => {
    snapshot = connectedSnapshot();
    render(<App />);
    await enabled(await screen.findByRole('button', { name: 'New key' }));
    const dialog = screen.getByRole('dialog', { name: 'New key' });
    fireEvent.click(within(dialog).getByRole('button', { name: /^Supabase/ }));
    fireEvent.click(within(dialog).getByRole('button', { name: /^Personal projects/ }));
    fireEvent.click(await within(dialog).findByRole('button', { name: /^GetKeys/ }));
    fireEvent.click(within(dialog).getByRole('button', { name: /Create a new key/ }));
    fireEvent.click(within(dialog).getByRole('button', { name: 'Change key' }));
    fireEvent.click(within(dialog).getByRole('button', { name: /Paste a key/ }));
    fireEvent.change(within(dialog).getByLabelText('Name'), { target: { value: 'Staging' } });
    fireEvent.change(within(dialog).getByLabelText('API key'), {
      target: { value: 'fixture-new-key' },
    });
    await enabled(within(dialog).getByRole('button', { name: 'Save key' }));
    await waitFor(() => expect(commands().some((c) => c.kind === 'import')).toBe(true));
    expect(commands().find((c) => c.kind === 'import')).toMatchObject({
      input: {
        connection_id: 'connection',
        provider_id: null,
        resource: 'abcdefghijklmnopqrst',
        name: 'Staging',
        fields: { token: 'fixture-new-key' },
      },
    });
  });
  it('keeps a pasted key on its own when no account is chosen', async () => {
    snapshot = connectedSnapshot();
    render(<App />);
    await enabled(await screen.findByRole('button', { name: 'New key' }));
    const dialog = screen.getByRole('dialog', { name: 'New key' });
    fireEvent.click(within(dialog).getByRole('button', { name: /^Supabase/ }));
    fireEvent.click(within(dialog).getByRole('button', { name: /^No account/ }));
    // A key without an account can only be pasted, so that step is skipped.
    expect(within(dialog).queryByRole('group', { name: 'Project' })).toBeNull();
    expect(within(dialog).queryByRole('group', { name: 'Key' })).toBeNull();
    fireEvent.change(within(dialog).getByLabelText('Name'), { target: { value: 'Mine' } });
    fireEvent.change(within(dialog).getByLabelText('API key'), { target: { value: 'sb_secret' } });
    await enabled(within(dialog).getByRole('button', { name: 'Save key' }));
    await waitFor(() =>
      expect(commands()).toContainEqual({
        kind: 'save_key',
        credential_id: null,
        plugin_id: 'official.supabase',
        name: 'Mine',
        fields: { token: 'sb_secret' },
      }),
    );
  });
  it('makes a key for a new project, waiting for the project to start', async () => {
    snapshot = connectedSnapshot();
    render(<App />);
    await enabled(await screen.findByRole('button', { name: 'New key' }));
    const dialog = screen.getByRole('dialog', { name: 'New key' });
    fireEvent.click(within(dialog).getByRole('button', { name: /^Supabase/ }));
    fireEvent.click(within(dialog).getByRole('button', { name: /^Personal projects/ }));
    fireEvent.click(await within(dialog).findByRole('button', { name: /New project/ }));
    const form = within(dialog).getByRole('form', { name: 'New project' });
    fireEvent.change(within(form).getByLabelText('Project name'), {
      target: { value: 'My App!' },
    });
    expect(within(form).getByLabelText('Project name')).toHaveValue('my-app-');
    fireEvent.change(within(form).getByLabelText('Region'), { target: { value: 'emea' } });
    await enabled(within(form).getByRole('button', { name: 'Create project' }));
    expect(
      await screen.findByText('Created my-app. Its my-app database password is saved as a key.'),
    ).toBeInTheDocument();
    expect(commands()).toContainEqual({
      kind: 'create_resource',
      connection_id: 'connection',
      owner: 'personal-org',
      name: 'my-app',
      options: { region: 'emea' },
    });
    // The new project is chosen, and the cascade moves on to the key.
    expect(within(dialog).getByText('my-app')).toBeInTheDocument();
    fireEvent.click(within(dialog).getByRole('button', { name: /Create a new key/ }));
    fireEvent.change(within(dialog).getByLabelText('Name'), { target: { value: 'App server' } });
    await enabled(within(dialog).getByRole('button', { name: 'Create key' }));
    await waitFor(() =>
      expect(commands()).toContainEqual({
        kind: 'create',
        connection_id: 'connection',
        resource: NEW_REF,
        name: 'App server',
      }),
    );
  });
  it('offers to connect an account when the provider has none', async () => {
    snapshot = connectedSnapshot();
    snapshot.plugins[0].installed = true;
    render(<App />);
    await enabled(await screen.findByRole('button', { name: 'New key' }));
    const dialog = screen.getByRole('dialog', { name: 'New key' });
    fireEvent.click(within(dialog).getByRole('button', { name: /^GitHub/ }));
    const account = within(dialog).getByRole('group', { name: 'Account' });
    await enabled(within(account).getByRole('button', { name: /Connect a GitHub account/ }));
    await waitFor(() =>
      expect(commands()).toContainEqual({ kind: 'scan_sources', plugin_id: 'official.github' }),
    );
  });
  it('puts a new key in the group being viewed', async () => {
    snapshot = connectedSnapshot();
    snapshot.credentials[0].groups = ['backend'];
    render(<App />);
    fireEvent.click(await screen.findByRole('tab', { name: /^backend/ }));
    await enabled(screen.getByRole('button', { name: 'New key' }));
    const dialog = screen.getByRole('dialog', { name: 'New key' });
    expect(dialog).toHaveTextContent('It joins backend.');
    fireEvent.click(within(dialog).getByRole('button', { name: /^Supabase/ }));
    fireEvent.click(within(dialog).getByRole('button', { name: /^Personal projects/ }));
    fireEvent.click(await within(dialog).findByRole('button', { name: /^GetKeys/ }));
    fireEvent.click(within(dialog).getByRole('button', { name: /Create a new key/ }));
    fireEvent.change(within(dialog).getByLabelText('Name'), { target: { value: 'Worker' } });
    await enabled(within(dialog).getByRole('button', { name: 'Create key' }));
    await waitFor(() =>
      expect(commands()).toContainEqual({
        kind: 'set_groups',
        credential_id: 'created',
        groups: ['backend'],
      }),
    );
    expect(await item('Worker')).toBeInTheDocument();
  });
  it('swaps the management key by opening the provider and pasting a new one', async () => {
    snapshot = connectedSnapshot();
    render(<App />);
    const panel = await accounts('Supabase');
    await enabled(within(panel).getByRole('button', { name: /Use a dedicated key/ }));
    expect(commands()).toContainEqual({ kind: 'open_token_page', plugin_id: 'official.supabase' });
    const dialog = screen.getByRole('dialog');
    expect(
      within(dialog).getByRole('heading', { name: 'Rotate Supabase sign-in key' }),
    ).toBeInTheDocument();
    fireEvent.change(within(dialog).getByLabelText('Access token'), {
      target: { value: 'sbp_fixture' },
    });
    fireEvent.click(within(dialog).getByRole('button', { name: 'Use new key' }));
    expect(await screen.findByText('Sign-in key replaced')).toBeInTheDocument();
    expect(commands()).toContainEqual({
      kind: 'connect_token',
      plugin_id: 'official.supabase',
      fields: { token: 'sbp_fixture' },
      connection_id: 'connection',
    });
  });
  it('shows an active rotation inline on its key', async () => {
    snapshot = connectedSnapshot();
    snapshot.rotations = [
      {
        id: 'rotation',
        credential_id: 'credential',
        connection_id: 'connection',
        resource: 'abcdefghijklmnopqrst',
        plugin_version: '0.1.0',
        name: 'Production API replacement',
        state: { kind: 'ready_to_retire' },
        candidate_id: null,
        created_at: 1791493200,
      },
    ];
    render(<App />);
    expect(await item('Production API')).toHaveTextContent('Rotating');
    const details = await expand('Production API');
    const steps = within(details).getByRole('list', { name: 'Rotation progress' });
    expect(within(steps).getByText('Swap it in').closest('li')).toHaveAttribute(
      'aria-current',
      'step',
    );
    expect(details).toHaveTextContent('Put it wherever you use this key');
    expect(screen.getByRole('button', { name: 'Delete Production API' })).toBeDisabled();
    await enabled(within(details).getByRole('button', { name: /Retire old key/ }));
    expect(screen.getByRole('dialog', { name: 'Retire the old key?' })).toBeInTheDocument();
  });
  it('adds a key from any service without a provider', async () => {
    snapshot = connectedSnapshot();
    render(<App />);
    await enabled(await screen.findByRole('button', { name: 'New key' }));
    let dialog = screen.getByRole('dialog', { name: 'New key' });
    fireEvent.click(within(dialog).getByRole('button', { name: /Any other service/ }));
    dialog = screen.getByRole('dialog', { name: 'New key' });
    fireEvent.change(within(dialog).getByLabelText('Name'), { target: { value: 'OpenAI' } });
    fireEvent.paste(within(dialog).getByLabelText('Variable 1'), {
      clipboardData: { getData: () => 'OPENAI_API_KEY=sk-test\nexport OPENAI_ORG="org-1"' },
    });
    expect(within(dialog).getByLabelText('Variable 2')).toHaveValue('OPENAI_ORG');
    fireEvent.click(within(dialog).getByRole('button', { name: 'OPENAI_ORG is secret' }));
    await enabled(within(dialog).getByRole('button', { name: 'Save key' }));
    expect(await screen.findByText('Key saved')).toBeInTheDocument();
    expect(commands()).toContainEqual({
      kind: 'save_custom',
      credential_id: null,
      name: 'OpenAI',
      fields: [
        { variable: 'OPENAI_API_KEY', value: 'sk-test', secret: true },
        { variable: 'OPENAI_ORG', value: 'org-1', secret: false },
      ],
    });
    expect(await item('OpenAI')).toHaveTextContent('Custom');
  });
  it('exports every key as .env, or only the keys in a group', async () => {
    snapshot = connectedSnapshot();
    snapshot.credentials[0].groups = ['backend'];
    render(<App />);
    await enabled(await screen.findByRole('button', { name: 'Export keys' }));
    let dialog = screen.getByRole('dialog', { name: 'Export keys' });
    const boxes = within(dialog).getAllByRole('checkbox');
    expect(boxes).toHaveLength(3);
    for (const box of boxes) expect(box).toBeChecked();
    fireEvent.click(within(dialog).getByRole('button', { name: /Save \.env/ }));
    await waitFor(() => expect(exportEnv).toHaveBeenCalledTimes(1));
    expect(vi.mocked(exportEnv).mock.calls[0]).toEqual([
      [
        { credential_id: 'credential', field: 'token', variable: 'SUPABASE_KEY' },
        { credential_id: 'credential', field: 'url', variable: 'SUPABASE_URL' },
        {
          credential_id: 'signin-connection',
          field: 'token',
          variable: 'SUPABASE_ACCESS_TOKEN',
        },
      ],
      true,
    ]);
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
    fireEvent.click(screen.getByRole('tab', { name: /^backend/ }));
    expect(screen.queryByRole('button', { name: 'Details for Supabase CLI' })).toBeNull();
    await enabled(screen.getByRole('button', { name: 'Export keys' }));
    dialog = screen.getByRole('dialog', { name: 'Export backend' });
    expect(within(dialog).getAllByRole('checkbox')).toHaveLength(2);
    fireEvent.click(within(dialog).getByRole('button', { name: 'Close dialog' }));
    await enabled(screen.getByRole('button', { name: 'Refresh keys' }));
    await waitFor(() =>
      expect(commands().some((c) => c.kind === 'sync' && c.connection_id === 'connection')).toBe(
        true,
      ),
    );
    expect(commands()).not.toContainEqual({ kind: 'scan_sources', plugin_id: null });
  });
  it('files a key under a group from its details, and its groups open', async () => {
    snapshot = connectedSnapshot();
    snapshot.credentials[1].groups = ['backend'];
    render(<App />);
    let details = await expand('Production API');
    expect(within(details).queryByRole('textbox')).toBeNull();
    fireEvent.click(within(details).getByRole('button', { name: 'Groups for Production API' }));
    const menu = within(details).getByRole('menu');
    expect(within(menu).getByRole('menuitemcheckbox', { name: 'backend' })).toHaveAttribute(
      'aria-checked',
      'false',
    );
    fireEvent.click(within(menu).getByRole('menuitem', { name: 'New group' }));
    const input = within(details).getByLabelText('New group name');
    fireEvent.change(input, { target: { value: 'staging' } });
    fireEvent.keyDown(input, { key: 'Enter' });
    await waitFor(() =>
      expect(commands()).toContainEqual({
        kind: 'set_groups',
        credential_id: 'credential',
        groups: ['staging'],
      }),
    );
    expect(await screen.findByRole('tab', { name: /^staging/ })).toBeInTheDocument();
    details = screen.getByRole('region', { name: 'Production API details' });
    fireEvent.click(await within(details).findByRole('button', { name: 'staging' }));
    expect(screen.getByRole('tab', { name: /^staging/ })).toHaveAttribute('aria-selected', 'true');
    fireEvent.click(screen.getByRole('button', { name: 'Groups for Production API' }));
    fireEvent.click(screen.getByRole('menuitemcheckbox', { name: 'backend' }));
    await waitFor(() =>
      expect(commands()).toContainEqual({
        kind: 'set_groups',
        credential_id: 'credential',
        groups: ['staging', 'backend'],
      }),
    );
  });
  it('filters the keys in the tab being viewed, apart from search', async () => {
    snapshot = connectedSnapshot();
    snapshot.credentials[0].problem = 'Rejected by the provider.';
    render(<App />);
    expect(await screen.findByPlaceholderText('Search keys')).toBeInTheDocument();
    await item('Supabase CLI');
    fireEvent.click(screen.getByRole('button', { name: 'Filter' }));
    fireEvent.click(screen.getByRole('menuitemcheckbox', { name: 'Invalid' }));
    expect(screen.queryByRole('button', { name: 'Details for Supabase CLI' })).toBeNull();
    expect(await item('Production API')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Filter' })).toHaveTextContent('1');
    fireEvent.click(screen.getByRole('menuitem', { name: 'Clear filter' }));
    expect(await item('Supabase CLI')).toBeInTheDocument();
  });
  it('makes a group from the tabs and picks its keys', async () => {
    snapshot = connectedSnapshot();
    render(<App />);
    await item('Production API');
    expect(screen.getByRole('tab', { name: /Ready to use/ })).toHaveAttribute(
      'aria-selected',
      'true',
    );
    fireEvent.click(screen.getByRole('button', { name: 'New group' }));
    const name = screen.getByLabelText('New group name');
    fireEvent.change(name, { target: { value: 'backend' } });
    fireEvent.keyDown(name, { key: 'Enter' });
    const dialog = await screen.findByRole('dialog', { name: 'backend' });
    fireEvent.click(within(dialog).getByRole('checkbox', { name: /Production API/ }));
    await enabled(within(dialog).getByRole('button', { name: 'Save' }));
    await waitFor(() =>
      expect(commands()).toContainEqual({
        kind: 'set_groups',
        credential_id: 'credential',
        groups: ['backend'],
      }),
    );
    const tab = await screen.findByRole('tab', { name: /^backend/ });
    expect(tab).toHaveAttribute('aria-selected', 'true');
    expect(await item('Production API')).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Details for Supabase CLI' })).toBeNull();
    await enabled(screen.getByRole('button', { name: /Delete group/ }));
    await waitFor(() =>
      expect(commands()).toContainEqual({
        kind: 'set_groups',
        credential_id: 'credential',
        groups: [],
      }),
    );
    await waitFor(() => expect(screen.queryByRole('tab', { name: /^backend/ })).toBeNull());
  });
  it('opens a key from anywhere on its row', async () => {
    snapshot = connectedSnapshot();
    render(<App />);
    const row = await item('Production API');
    fireEvent.click(within(row).getByText('Production API'));
    expect(screen.getByRole('region', { name: 'Production API details' })).toBeInTheDocument();
    fireEvent.click(within(row).getByText('Production API'));
    expect(screen.queryByRole('region', { name: 'Production API details' })).toBeNull();
  });
  it('puts keys the provider won’t list in Failed instead of failing the refresh', async () => {
    snapshot = connectedSnapshot();
    listed = () => {
      throw new Error('forbidden: This connection does not have permission for the operation.');
    };
    render(<App />);
    await enabled(await screen.findByRole('button', { name: 'Refresh keys' }));
    expect(await screen.findByText(/1 moved to Failed/)).toBeInTheDocument();
    expect(commands()).toContainEqual({
      kind: 'set_groups',
      credential_id: 'credential',
      groups: ['Failed'],
    });
    expect(screen.queryByRole('dialog')).toBeNull();
    fireEvent.click(await screen.findByRole('tab', { name: /^Failed/ }));
    expect(await item('Production API')).toBeInTheDocument();
    listed = () => {};
    await enabled(screen.getByRole('button', { name: 'Refresh keys' }));
    await waitFor(() =>
      expect(commands()).toContainEqual({
        kind: 'set_groups',
        credential_id: 'credential',
        groups: [],
      }),
    );
  });
  it('opens the provider page where keys are made, once the provider is added', async () => {
    snapshot.plugins[0].installed = true;
    render(<App />);
    fireEvent.click(await screen.findByRole('button', { name: 'Providers' }));
    await card('AWS');
    expect(screen.queryByRole('button', { name: 'Open AWS' })).toBeNull();
    await enabled(await screen.findByRole('button', { name: 'Open GitHub' }));
    await waitFor(() =>
      expect(commands()).toContainEqual({ kind: 'open_token_page', plugin_id: 'official.github' }),
    );
    expect(screen.queryByRole('button', { name: 'Activity' })).toBeNull();
  });
  it('marks keys the provider rejected', async () => {
    snapshot = connectedSnapshot();
    snapshot.credentials[0].problem = 'Rejected by the provider. It was revoked or is wrong.';
    render(<App />);
    expect(await item('Production API')).toHaveTextContent('Invalid');
    const details = await expand('Production API');
    expect(details).toHaveTextContent('Rejected by the provider');
  });
  it('removes a provider with its accounts from the Providers page', async () => {
    snapshot = connectedSnapshot();
    const panel = await (async () => {
      render(<App />);
      return accounts('Supabase');
    })();
    await enabled(within(panel).getByRole('button', { name: /Remove provider/ }));
    const dialog = screen.getByRole('dialog', { name: 'Remove Supabase?' });
    expect(dialog).toHaveTextContent('forgets 1 account and 1 key');
    await enabled(within(dialog).getByRole('button', { name: 'Remove Supabase' }));
    expect(await screen.findByText('Supabase removed')).toBeInTheDocument();
    expect(commands()).toContainEqual({ kind: 'uninstall', plugin_id: 'official.supabase' });
  });
  it('shows where the vault lives', async () => {
    render(<App />);
    fireEvent.click(await screen.findByRole('button', { name: 'Settings' }));
    expect(await screen.findByText(/GetKeys\\vault\.db/)).toBeInTheDocument();
    expect(screen.queryByText(/Plugins?/)).not.toBeInTheDocument();
    expect(screen.queryByText(/Project folders/)).not.toBeInTheDocument();
    await enabled(screen.getByRole('button', { name: /Open folder/ }));
    await waitFor(() => expect(commands()).toContainEqual({ kind: 'open_vault_folder' }));
  });
});
