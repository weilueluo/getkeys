import { useState, type ReactNode } from 'react';
import {
  ChevronDown,
  Copy,
  Eye,
  EyeOff,
  ExternalLink,
  KeyRound,
  Plus,
  RefreshCw,
  RotateCw,
  Trash2,
} from 'lucide-react';
import type { ConnectionView, Discovery, PluginView } from './types';
import type { Controller } from './controller';
import { copyCredential, request } from './bridge';
import { inputs, Modal, PageHeader, ProviderMark, humanize, Tabs } from './ui';
import { borrowed, COPIED, MASK, primaryField, shown, SIGN_IN, useReveal } from './secrets';

const CATEGORIES: Record<string, string> = {
  ai: 'AI',
  devtools: 'Developer tools',
  cloud: 'Cloud',
  database: 'Database',
  payments: 'Payments',
  email: 'Email',
};
const categoryName = (c: string) => CATEGORIES[c] ?? humanize(c);

type Actions = {
  refreshAccount: (id: string) => void;
  reconnect: (id: string) => void;
  rotateSignIn: (id: string) => void;
  dedicate: (id: string, rotating: boolean) => void;
};

export function Marketplace({
  snapshot,
  busy,
  perform,
  search,
  searchField,
  connect,
  add,
  found,
  ...actions
}: Controller &
  Actions & {
    search: string;
    searchField: ReactNode;
    connect: (plugin: PluginView) => void;
    add: (plugin: PluginView) => void;
    found: Discovery[];
  }) {
  const [filter, setFilter] = useState<'all' | 'added'>('all');
  const [open, setOpen] = useState<string[]>([]);
  const [removing, setRemoving] = useState<PluginView>();
  const accountsOf = (p: PluginView) =>
    snapshot.connections.filter((c) => c.plugin_id === p.manifest.id && c.state !== 'disconnected');
  const declarative = (p: PluginView) => p.manifest.kind === 'declarative';
  const added = (p: PluginView) => (declarative(p) ? p.installed : accountsOf(p).length > 0);
  const plugins = snapshot.plugins.filter(
    (p) =>
      (filter === 'all' || added(p)) &&
      `${p.manifest.name} ${p.manifest.description} ${p.manifest.category}`
        .toLowerCase()
        .includes(search.toLowerCase()),
  );
  const toggle = (id: string) =>
    setOpen((o) => (o.includes(id) ? o.filter((x) => x !== id) : [...o, id]));
  return (
    <>
      <PageHeader title="Providers" search={searchField} />
      <Tabs
        label="Provider filter"
        value={filter}
        onChange={setFilter}
        tabs={[
          { id: 'all', label: 'All', count: snapshot.plugins.length },
          { id: 'added', label: 'Added', count: snapshot.plugins.filter(added).length },
        ]}
      />
      <div className="provider-list">
        {plugins.map((p) => {
          const accounts = accountsOf(p);
          const expandable = accounts.length > 0 || (declarative(p) && p.installed);
          const expanded = expandable && open.includes(p.manifest.id);
          const keyCount = snapshot.credentials.filter(
            (k) => k.plugin_id === p.manifest.id && !k.connection_id && !k.retired,
          ).length;
          const local = found.filter((d) => d.plugin_id === p.manifest.id);
          return (
            <article className={`provider-card ${expanded ? 'open' : ''}`} key={p.manifest.id}>
              <div className="provider-head">
                <ProviderMark name={p.manifest.name} />
                <div className="provider-body">
                  <div className="provider-title">
                    <h2>{p.manifest.name}</h2>
                    {p.manifest.category && (
                      <span className="badge subtle">{categoryName(p.manifest.category)}</span>
                    )}
                    {accounts.length > 0 && (
                      <span className="badge">
                        {accounts.length} {accounts.length === 1 ? 'account' : 'accounts'}
                      </span>
                    )}
                    {declarative(p) && p.installed && (
                      <span className="badge">
                        {keyCount} {keyCount === 1 ? 'key' : 'keys'}
                      </span>
                    )}
                    {p.update_available && <span className="badge warning">Update</span>}
                  </div>
                  <p>{p.manifest.description}</p>
                  {!accounts.length && local.length > 0 && (
                    <p className="provider-found">
                      Signed in on this device via{' '}
                      {[...new Set(local.map((d) => d.source))].join(', ')}
                    </p>
                  )}
                </div>
                <div className="provider-actions">
                  {p.installed && p.manifest.token_page && (
                    <button
                      className="button outline"
                      title={`Open the ${p.manifest.name} page where its keys are made`}
                      aria-label={`Open ${p.manifest.name}`}
                      onClick={() =>
                        void perform(() =>
                          request({ kind: 'open_token_page', plugin_id: p.manifest.id }),
                        )
                      }
                    >
                      <ExternalLink size={14} strokeWidth={1.75} /> Open
                    </button>
                  )}
                  {expandable ? (
                    <button
                      className="icon-button outline provider-toggle"
                      aria-expanded={expanded}
                      aria-label={`${p.manifest.name} ${declarative(p) ? 'details' : 'accounts'}`}
                      title={expanded ? 'Hide' : 'Show'}
                      onClick={() => toggle(p.manifest.id)}
                    >
                      <ChevronDown size={16} strokeWidth={1.75} />
                    </button>
                  ) : (
                    <button
                      disabled={busy}
                      className={local.length ? 'button' : 'button outline'}
                      onClick={() => add(p)}
                    >
                      <Plus size={14} /> Add
                    </button>
                  )}
                </div>
              </div>
              {expanded && (
                <div
                  className="provider-panel"
                  role="region"
                  aria-label={`${p.manifest.name} ${declarative(p) ? 'details' : 'accounts'}`}
                >
                  {declarative(p) && (
                    <p className="provider-note">
                      {p.manifest.name} keys need no account. GetKeys picks them up from this device
                      on Refresh, and New key adds one by hand.
                    </p>
                  )}
                  <ul className="account-list">
                    {accounts.map((c) => (
                      <Account
                        key={c.id}
                        account={c}
                        plugin={p}
                        control={{ snapshot, busy, perform }}
                        actions={actions}
                      />
                    ))}
                  </ul>
                  <div className="provider-footer">
                    {!declarative(p) && (
                      <button
                        disabled={busy}
                        className="button outline small"
                        onClick={() => connect(p)}
                      >
                        <Plus size={13} /> Add account
                      </button>
                    )}
                    {p.update_available && (
                      <button
                        disabled={busy}
                        className="button outline small"
                        onClick={() =>
                          void perform(
                            () =>
                              request({
                                kind: 'install',
                                plugin_id: p.manifest.id,
                                accept_permissions: true,
                              }),
                            'Provider updated',
                          )
                        }
                      >
                        Install update
                      </button>
                    )}
                    <button
                      disabled={busy}
                      className="button ghost small danger provider-remove"
                      onClick={() => setRemoving(p)}
                    >
                      <Trash2 size={13} strokeWidth={1.75} /> Remove provider
                    </button>
                    <details className="permissions">
                      <summary>Permissions</summary>
                      <dl>
                        <dt>Hosts</dt>
                        <dd className="mono">{p.manifest.hosts.join('  ')}</dd>
                        <dt>Operations</dt>
                        <dd>
                          {p.manifest.operations.map(humanize).join(', ') ||
                            'None, it only checks keys'}
                        </dd>
                        <dt>Publisher</dt>
                        <dd>
                          {p.manifest.publisher} ·{' '}
                          <span className="mono">v{p.manifest.version}</span>
                        </dd>
                      </dl>
                    </details>
                  </div>
                </div>
              )}
            </article>
          );
        })}
        {plugins.length === 0 && (
          <p className="no-results">
            {search ? `No providers match “${search}”.` : 'No providers added yet.'}
          </p>
        )}
      </div>
      <p className="footnote">
        Each provider is signed and can reach only the hosts listed under Permissions. Your vault
        stays on this device.
      </p>
      {removing && (
        <RemoveProvider
          plugin={removing}
          accounts={accountsOf(removing)}
          control={{ snapshot, busy, perform }}
          close={() => setRemoving(undefined)}
        />
      )}
    </>
  );
}

/** Confirms removing a provider along with its accounts and their keys. */
function RemoveProvider({
  plugin,
  accounts,
  control: { snapshot, busy, perform },
  close,
}: {
  plugin: PluginView;
  accounts: ConnectionView[];
  control: Controller;
  close: () => void;
}) {
  const { name, id } = plugin.manifest;
  const ids = new Set(accounts.map((a) => a.id));
  const keys = snapshot.credentials.filter(
    (k) =>
      (ids.has(k.connection_id) || (!k.connection_id && k.plugin_id === id)) &&
      !k.retired &&
      k.credential_type !== SIGN_IN,
  ).length;
  const count = (n: number, word: string) => `${n} ${word}${n === 1 ? '' : 's'}`;
  return (
    <Modal title={`Remove ${name}?`} onClose={close}>
      <p>
        GetKeys forgets {accounts.length ? `${count(accounts.length, 'account')} and ` : ''}
        {count(keys, 'key')} from {name}. Nothing is deleted at {name}, except the management keys
        GetKeys created for itself.
      </p>
      <div className="modal-footer">
        <button className="button outline" onClick={close}>
          Cancel
        </button>
        <button
          className="button destructive"
          disabled={busy}
          onClick={() =>
            void perform(
              () => request({ kind: 'uninstall', plugin_id: id }),
              `${name} removed`,
            ).then((r) => {
              if (r !== undefined) close();
            })
          }
        >
          Remove {name}
        </button>
      </div>
    </Modal>
  );
}

/** One connected account and the management key GetKeys uses to reach it. */
function Account({
  account: c,
  plugin,
  control,
  actions,
}: {
  account: ConnectionView;
  plugin: PluginView;
  control: Controller;
  actions: Actions;
}) {
  const { snapshot, busy, perform } = control;
  const { manifest } = plugin;
  const [removing, setRemoving] = useState(false);
  const key = snapshot.credentials.find(
    (k) => k.connection_id === c.id && k.credential_type === SIGN_IN && !k.retired,
  );
  const [secret, reveal] = useReveal(key?.id ?? '', perform);
  const field = key && primaryField(key);
  const value = field && shown(field, secret);
  const ops = manifest.operations;
  const automatic =
    manifest.account_credentials &&
    ops.includes('create_credential') &&
    ops.includes('revoke_credential');
  const pastes = !!manifest.token_page && inputs(manifest.account_fields).length > 0;
  const dedicated = c.source === 'getkeys';
  const local = borrowed(c.source);
  const keyAction = dedicated
    ? { label: 'Rotate key', run: () => actions.dedicate(c.id, true) }
    : automatic
      ? { label: 'Create dedicated key', run: () => actions.dedicate(c.id, false) }
      : pastes
        ? {
            label: c.source === 'token' ? 'Rotate key' : 'Use a dedicated key',
            run: () => actions.rotateSignIn(c.id),
          }
        : undefined;
  const live = c.state === 'connected';
  return (
    <li className="account">
      <div className="account-head">
        <div className="account-name">
          <strong>{c.name}</strong>
          <small>
            {key?.name ?? 'Browser sign-in'}
            {dedicated && ' · created by GetKeys for this account'}
          </small>
        </div>
        {c.state === 'needs_reauth' && <span className="badge warning">Needs sign-in</span>}
        <div className="account-buttons">
          {!live && (
            <button
              className="button small"
              disabled={busy}
              onClick={() => actions.reconnect(c.id)}
            >
              Reconnect
            </button>
          )}
          <button
            className="button outline small"
            disabled={busy || !live}
            title="Look for new keys in this account"
            aria-label={`Refresh ${c.name}`}
            onClick={() => actions.refreshAccount(c.id)}
          >
            <RefreshCw size={13} strokeWidth={1.75} /> Refresh
          </button>
          <button
            className="icon-button outline danger"
            disabled={busy}
            title="Remove account"
            aria-label={`Remove ${c.name}`}
            onClick={() => setRemoving(true)}
          >
            <Trash2 size={15} strokeWidth={1.75} />
          </button>
        </div>
      </div>
      {key && field && (
        <div className="management-key">
          <span className="management-label">
            <KeyRound size={13} strokeWidth={1.75} /> Management key
          </span>
          {local ? (
            <span className="management-note">Your {key.name} · listed in Keys</span>
          ) : (
            <code className={value === undefined ? 'masked' : ''}>{value ?? MASK}</code>
          )}
          <div className="key-buttons">
            {!local && (
              <>
                <button
                  className="icon-button outline"
                  disabled={busy || !field.available}
                  title={secret ? 'Hide' : 'Reveal for 30 seconds'}
                  aria-label={`${secret ? 'Hide' : 'Reveal'} ${c.name} management key`}
                  onClick={reveal}
                >
                  {secret ? <EyeOff size={15} /> : <Eye size={15} />}
                </button>
                <button
                  className="icon-button outline"
                  disabled={busy || !field.available}
                  title="Copy"
                  aria-label={`Copy ${c.name} management key`}
                  onClick={() => void perform(() => copyCredential(key.id, field.key), COPIED)}
                >
                  <Copy size={15} strokeWidth={1.75} />
                </button>
              </>
            )}
            {keyAction && (
              <button
                className="button outline small"
                disabled={busy || !live}
                title={
                  dedicated || c.source === 'token'
                    ? 'Replace this key with a new one'
                    : 'Stop borrowing your CLI sign-in and use a key just for GetKeys'
                }
                onClick={keyAction.run}
              >
                <RotateCw size={13} strokeWidth={1.75} /> {keyAction.label}
              </button>
            )}
          </div>
        </div>
      )}
      {removing && (
        <Modal title={`Remove ${c.name}?`} onClose={() => setRemoving(false)}>
          <p>
            GetKeys stops reaching this {manifest.name} account
            {dedicated ? ' and revokes the key it created for it' : ''}. Its keys stay on this
            device.
          </p>
          <div className="modal-footer">
            <button className="button outline" onClick={() => setRemoving(false)}>
              Cancel
            </button>
            <button
              className="button destructive"
              disabled={busy}
              onClick={() =>
                void perform(
                  () => request({ kind: 'disconnect', connection_id: c.id }),
                  'Account removed',
                ).then((r) => {
                  if (r !== undefined) setRemoving(false);
                })
              }
            >
              Remove account
            </button>
          </div>
        </Modal>
      )}
    </li>
  );
}
