import { useCallback, useEffect, useRef, useState } from 'react';
import { isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { Blocks, Check, KeyRound, Lock, Settings, TriangleAlert, X } from 'lucide-react';
import type { AuthProgress, ConnectionView, Discovery, PluginView, Snapshot } from './types';
import { request } from './bridge';
import { ConnectDialog } from './Connect';
import { DialogHost, KeyGlyph, Modal, SearchField } from './ui';
import { Marketplace } from './Marketplace';
import { VaultPanel } from './VaultPanel';
import { SettingsPanel } from './SettingsPanel';
import { JobDialog, useJob, type Outcome } from './Job';
import { listed } from './secrets';
import { onShell, type ShellEvent } from './shell';

type Page = 'vault' | 'plugins' | 'settings';
const navigation = [
  { id: 'vault', label: 'Keys', icon: KeyRound },
  { id: 'plugins', label: 'Providers', icon: Blocks },
] as const;

export function App() {
  const [snapshot, setSnapshot] = useState<Snapshot>();
  const [page, setPage] = useState<Page>('plugins');
  const [search, setSearch] = useState('');
  const searchRef = useRef<HTMLInputElement>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const [notice, setNotice] = useState('');
  const [dialogs, setDialogs] = useState(0);
  const opened = useCallback((delta: number) => setDialogs((n) => n + delta), []);
  const [connecting, setConnecting] = useState<{
    plugin: PluginView;
    connectionId?: string;
    rotate?: boolean;
    /** The sign-ins on this device were already tried and failed. */
    tried?: boolean;
  }>();
  const [found, setFound] = useState<Discovery[]>([]);
  const [auth, setAuth] = useState<{
    plugin: PluginView;
    progress: AuthProgress;
    flowId: string;
  }>();
  const refresh = useCallback(async () => {
    const data = await request<Snapshot>({ kind: 'snapshot' });
    setSnapshot(data);
    return data;
  }, []);
  const scan = useCallback(
    () =>
      request<Discovery[]>({ kind: 'discover' })
        .then(setFound)
        .catch(() => setFound([])),
    [],
  );
  const { job, run, cancel, close } = useJob(refresh, setBusy);
  useEffect(() => {
    void refresh()
      .then((s) => {
        if (s.credentials.length > 0 || s.connections.length > 0) setPage('vault');
      })
      .catch((e) => setError(String(e)));
    void scan();
  }, [refresh, scan]);
  useEffect(() => {
    const key = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.key === 'k') {
        e.preventDefault();
        searchRef.current?.focus();
      }
    };
    window.addEventListener('keydown', key);
    return () => window.removeEventListener('keydown', key);
  }, []);
  useEffect(() => {
    if (!notice) return;
    const timer = setTimeout(() => setNotice(''), 5000);
    return () => clearTimeout(timer);
  }, [notice]);
  const perform = useCallback(
    async <T,>(
      task: () => Promise<T>,
      message?: string | ((result: T) => string | undefined),
    ): Promise<T | undefined> => {
      setBusy(true);
      setError('');
      try {
        const result = await task();
        const text = typeof message === 'function' ? message(result) : message;
        await refresh();
        if (text) setNotice(text);
        return result;
      } catch (e) {
        setError(String(e));
        await refresh().catch(() => {});
        return undefined;
      } finally {
        setBusy(false);
      }
    },
    [refresh],
  );
  const keyCount = (n: number) => `${n} ${n === 1 ? 'key' : 'keys'}`;
  /** Lists an account's keys after it connects. */
  const listKeys = async (connection: ConnectionView) => {
    const outcome = await run(`Adding ${connection.name}`, { connection: connection.id });
    if (outcome && !outcome.failed.length)
      setNotice(`Added ${connection.name} · ${keyCount(outcome.added)}`);
  };
  const proceed = (plugin: PluginView, progress?: AuthProgress, rotated = false) => {
    if (!progress) return;
    setConnecting(undefined);
    if (progress.kind === 'key_saved') {
      setAuth(undefined);
      void scan();
      changePage('vault');
      setNotice(`Saved ${progress.credential.name}`);
    } else if (progress.kind === 'connected') {
      setAuth(undefined);
      void scan();
      if (rotated) setNotice('Sign-in key replaced');
      else {
        changePage('vault');
        void listKeys(progress.connection);
      }
    } else if (progress.kind === 'browser' || progress.kind === 'select_role')
      setAuth({ plugin, progress, flowId: progress.flow_id });
  };
  const startAuth = async (
    plugin: PluginView,
    config: Record<string, string>,
    connectionId?: string,
  ) => {
    const progress = await perform(async () => {
      if (!plugin.installed)
        await request({ kind: 'install', plugin_id: plugin.manifest.id, accept_permissions: true });
      return request<AuthProgress>(
        connectionId
          ? { kind: 'reconnect', connection_id: connectionId }
          : { kind: 'begin_auth', plugin_id: plugin.manifest.id, config },
      );
    });
    proceed(plugin, progress);
  };
  const connect = (plugin: PluginView, connectionId?: string) => {
    setError('');
    setConnecting({ plugin, connectionId });
  };
  const pluginFor = (id: string) => {
    const c = snapshot?.connections.find((c) => c.id === id);
    const p = snapshot?.plugins.find((p) => p.manifest.id === c?.plugin_id);
    return c && p ? { c, p } : undefined;
  };
  const reconnect = (id: string) => {
    const target = pluginFor(id);
    if (!target) return;
    if (target.c.source === 'token') connect(target.p, id);
    else void startAuth(target.p, {}, id);
  };
  /** Opens the provider's token page and asks for the new sign-in key. */
  const rotateSignIn = (id: string) => {
    const target = pluginFor(id);
    if (!target) return;
    void request({ kind: 'open_token_page', plugin_id: target.p.manifest.id }).catch(() => {});
    setError('');
    setConnecting({ plugin: target.p, connectionId: id, rotate: true });
  };
  const failedNote = (o: Outcome) => (o.refused ? ` · ${o.refused} moved to Failed` : '');
  const summary = (o: Outcome) =>
    (o.added ? `Found ${keyCount(o.added)}` : o.cancelled ? 'Stopped' : 'Keys are up to date') +
    failedNote(o);
  /** Looks for new sign-ins on this device, then lists the keys in each account in `scope`. */
  const refreshKeys = async (
    scope: { connection?: string; connections?: string[]; label?: string } = {},
  ) => {
    const title = scope.label
      ? `Refreshing ${scope.label}`
      : scope.connection
        ? 'Refreshing account'
        : 'Refreshing keys';
    const { label: _, ...jobScope } = scope;
    const outcome = await run(title, jobScope);
    if (outcome && (!outcome.failed.length || outcome.cancelled)) setNotice(summary(outcome));
    void scan();
  };
  /** Adds a provider by bringing in every sign-in on this device, then the keys behind them. */
  const add = async (plugin: PluginView) => {
    const name = plugin.manifest.name;
    setError('');
    const outcome = await run(`Adding ${name}`, { plugin: plugin.manifest.id });
    void scan();
    if (!outcome) return;
    if (plugin.manifest.kind === 'declarative') {
      // Nothing to sign in to: its keys come from this device or are added by hand.
      changePage('vault');
      if (!outcome.failed.length || outcome.cancelled)
        setNotice(
          outcome.added
            ? `Added ${name} · ${keyCount(outcome.added)}`
            : `Added ${name} · add its keys with New key`,
        );
      return;
    }
    const accounts = outcome.snapshot.connections.filter(
      (c) => c.plugin_id === plugin.manifest.id && c.state !== 'disconnected',
    );
    if (!accounts.length) {
      if (outcome.cancelled) return;
      // Nothing usable on this device: ask for a token or a browser sign-in instead.
      const failure = outcome.failed.find((s) => s.label.startsWith('Connecting'));
      close();
      if (failure) setError(`The ${name} sign-in on this device didn’t work (${failure.detail}).`);
      setConnecting({ plugin, tried: true });
      return;
    }
    changePage('vault');
    if (!outcome.failed.length || outcome.cancelled)
      setNotice(
        outcome.cancelled
          ? summary(outcome)
          : `Added ${name} · ${keyCount(outcome.added)}${failedNote(outcome)}`,
      );
  };
  const dedicate = (id: string, rotating: boolean) =>
    void perform(
      () => request<ConnectionView>({ kind: 'dedicate', connection_id: id }),
      rotating ? 'Management key rotated' : 'Dedicated management key created',
    );
  useEffect(() => {
    if (!auth || !['browser', 'pending'].includes(auth.progress.kind)) return;
    const interval = 'interval' in auth.progress ? auth.progress.interval : 2;
    let active = true;
    const timer = setTimeout(
      () => {
        request<AuthProgress>({ kind: 'poll_auth', flow_id: auth.flowId })
          .then(async (progress) => {
            if (!active) return;
            if (progress.kind === 'connected') {
              await refresh();
              proceed(auth.plugin, progress);
            } else
              setAuth({
                ...auth,
                progress:
                  progress.kind === 'pending' && auth.progress.kind === 'browser'
                    ? { ...auth.progress, interval: progress.interval }
                    : progress,
              });
          })
          .catch((e) => {
            if (active) {
              setError(String(e));
              setAuth(undefined);
            }
          });
      },
      Math.max(interval, 1) * 1000,
    );
    return () => {
      active = false;
      clearTimeout(timer);
    };
  }, [auth, refresh]);
  const changePage = (next: Page) => {
    setPage(next);
    setSearch('');
  };
  // The tray and getkeys:// links: refresh, open a key, or show a provider.
  const [focusKey, setFocusKey] = useState<{ id: string }>();
  const shell = useRef<(event: ShellEvent, payload: string) => void>(undefined);
  shell.current = (event, payload) => {
    if (event === 'tray-refresh') return void refreshKeys();
    changePage(event === 'open-key' ? 'vault' : 'plugins');
    if (event === 'open-key') setFocusKey({ id: payload });
    const plugin = snapshot?.plugins.find(
      (p) => p.manifest.id === payload || p.manifest.id.endsWith(`.${payload}`),
    );
    if (event === 'open-provider' && plugin) setSearch(plugin.manifest.name);
  };
  useEffect(() => onShell((event, payload) => shell.current?.(event, payload)), []);
  // The CLI and agents changed the vault through the app.
  useEffect(() => {
    if (!isTauri()) return;
    const stop = listen('vault-changed', () => void refresh().catch(() => {}));
    return () => void stop.then((f) => f());
  }, [refresh]);
  const control = snapshot && { snapshot, busy, perform };
  const connections = snapshot?.connections ?? [];
  const searchField = (placeholder: string) => (
    <SearchField ref={searchRef} value={search} onChange={setSearch} placeholder={placeholder} />
  );
  return (
    <DialogHost.Provider value={{ error, opened }}>
      <div className="app-shell">
        <aside className="sidebar">
          <button
            className="brand"
            onClick={() => changePage(connections.length ? 'vault' : 'plugins')}
            aria-label="GetKeys home"
          >
            <span className="brand-tile">
              <KeyGlyph size={16} />
            </span>
            <span>getkeys</span>
          </button>
          <nav aria-label="Main navigation">
            {navigation.map(({ id, label, icon: Icon }) => (
              <button
                key={id}
                aria-current={page === id ? 'page' : undefined}
                className={page === id ? 'nav-item active' : 'nav-item'}
                onClick={() => changePage(id)}
                title={label}
              >
                <Icon size={16} strokeWidth={1.6} />
                <span className="nav-label">{label}</span>
                {id === 'vault' && !!snapshot?.credentials.length && (
                  <span className="nav-count">
                    {snapshot.credentials.filter((c) => listed(c, snapshot.connections)).length}
                  </span>
                )}
              </button>
            ))}
          </nav>
          <div className="sidebar-bottom">
            <button
              className={`nav-item ${page === 'settings' ? 'active' : ''}`}
              aria-current={page === 'settings' ? 'page' : undefined}
              onClick={() => changePage('settings')}
              title="Settings"
            >
              <Settings size={16} strokeWidth={1.6} />
              <span className="nav-label">Settings</span>
            </button>
            <div className="vault-status">
              <Lock size={12} strokeWidth={1.75} />
              <span>Encrypted on this device</span>
            </div>
          </div>
        </aside>
        <div className="main-shell">
          {busy && <div className="progress" aria-hidden="true" />}
          <main>
            {error && !dialogs && (
              <div role="alert" className="alert">
                <TriangleAlert size={16} strokeWidth={1.75} />
                <span>{error}</span>
                <button
                  aria-label="Dismiss error"
                  className="icon-button"
                  onClick={() => setError('')}
                >
                  <X size={14} />
                </button>
              </div>
            )}
            {!control ? (
              <div className="startup">
                <KeyGlyph size={28} />
                <h1>GetKeys</h1>
                <p>
                  {error
                    ? 'Your vault could not be opened. Resolve the message above and try again.'
                    : 'Opening your vault…'}
                </p>
                {error && (
                  <button
                    className="button secondary"
                    onClick={() =>
                      void refresh()
                        .then(() => setError(''))
                        .catch((e) => setError(String(e)))
                    }
                  >
                    Try again
                  </button>
                )}
              </div>
            ) : (
              <>
                {page === 'plugins' && (
                  <Marketplace
                    {...control}
                    search={search}
                    searchField={searchField('Filter providers')}
                    connect={connect}
                    add={(p) => void add(p)}
                    found={found}
                    refreshAccount={(id) => void refreshKeys({ connection: id })}
                    reconnect={reconnect}
                    rotateSignIn={rotateSignIn}
                    dedicate={dedicate}
                  />
                )}
                {page === 'vault' && (
                  <VaultPanel
                    {...control}
                    search={search}
                    searchField={searchField('Filter keys')}
                    browse={() => changePage('plugins')}
                    refreshKeys={(scope) => void refreshKeys(scope)}
                    focus={focusKey}
                  />
                )}
                {page === 'settings' && <SettingsPanel {...control} />}
              </>
            )}
          </main>
        </div>
        {busy && (
          <div role="status" className="visually-hidden">
            Working…
          </div>
        )}
        {notice && (
          <div className="toast" role="status">
            <Check size={14} strokeWidth={2} />
            {notice}
          </div>
        )}
        {job && <JobDialog job={job} cancel={cancel} />}
        {connecting && (
          <ConnectDialog
            plugin={connecting.plugin}
            found={connecting.tried ? [] : found}
            mode={connecting.rotate ? 'rotate' : connecting.connectionId ? 'reconnect' : 'connect'}
            busy={busy}
            onClose={() => {
              setConnecting(undefined);
              setError('');
            }}
            onLocal={(d) =>
              void perform(() =>
                request<AuthProgress>({
                  kind: 'connect_local',
                  source_id: d.id,
                  connection_id: null,
                }),
              ).then((p) => proceed(connecting.plugin, p))
            }
            onToken={(fields) =>
              void perform(() =>
                request<AuthProgress>({
                  kind: 'connect_token',
                  plugin_id: connecting.plugin.manifest.id,
                  fields,
                  connection_id: connecting.connectionId ?? null,
                }),
              ).then((p) => proceed(connecting.plugin, p, connecting.rotate))
            }
            onTokenPage={() =>
              void request({
                kind: 'open_token_page',
                plugin_id: connecting.plugin.manifest.id,
              }).catch((e) => setError(String(e)))
            }
            onBrowser={(config) => void startAuth(connecting.plugin, config)}
          />
        )}
        {auth && (
          <Modal
            title={`Connect ${auth.plugin.manifest.name}`}
            onClose={() => {
              void perform(() => request({ kind: 'cancel_auth', flow_id: auth.flowId }));
              setAuth(undefined);
            }}
          >
            {auth.progress.kind === 'select_role' ? (
              <>
                <p className="quiet">Choose the AWS account and role to use.</p>
                <div className="choice-list">
                  {auth.progress.roles.map((r) => (
                    <button
                      disabled={busy}
                      className="choice"
                      key={`${r.account_id}/${r.role}`}
                      onClick={() =>
                        void perform(() =>
                          request<AuthProgress>({
                            kind: 'choose_role',
                            flow_id: auth.flowId,
                            account_id: r.account_id,
                            role: r.role,
                          }),
                        ).then((p) => proceed(auth.plugin, p))
                      }
                    >
                      <strong>{r.account_name}</strong>
                      <span className="mono">{r.role}</span>
                    </button>
                  ))}
                </div>
              </>
            ) : (
              <div className="auth-wait">
                <p>Finish signing in to {auth.plugin.manifest.name} in your browser.</p>
                {auth.progress.kind === 'browser' && auth.progress.user_code && (
                  <>
                    <span className="field-label">Enter this code when asked</span>
                    <code className="device-code">{auth.progress.user_code}</code>
                  </>
                )}
                <p className="waiting">
                  <span className="pulse" aria-hidden="true" />
                  Waiting for approval
                </p>
              </div>
            )}
          </Modal>
        )}
      </div>
    </DialogHost.Provider>
  );
}
