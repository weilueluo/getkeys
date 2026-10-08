import { useCallback, useEffect, useRef, useState } from 'react';
import {
  Activity,
  ArrowUpRight,
  Check,
  ChevronRight,
  CircleHelp,
  KeyRound,
  LoaderCircle,
  Package,
  RotateCw,
  Search,
  Settings,
  ShieldCheck,
  X,
} from 'lucide-react';
import type { AuthProgress, PluginView, Snapshot } from './types';
import { request } from './bridge';
import { Field, Modal, ProviderMark, humanize } from './ui';
import { Marketplace } from './Marketplace';
import { VaultPanel } from './VaultPanel';
import { Rotations } from './Rotations';
import { SettingsPanel } from './SettingsPanel';

type Page = 'vault' | 'plugins' | 'rotations' | 'activity' | 'settings';
const navigation = [
  { id: 'vault', label: 'My keys', icon: KeyRound },
  { id: 'plugins', label: 'Plugins', icon: Package },
  { id: 'rotations', label: 'Rotations', icon: RotateCw },
  { id: 'activity', label: 'Activity', icon: Activity },
] as const;

export function App() {
  const [snapshot, setSnapshot] = useState<Snapshot>();
  const [page, setPage] = useState<Page>('plugins');
  const [search, setSearch] = useState('');
  const searchRef = useRef<HTMLInputElement>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const [notice, setNotice] = useState('');
  const [account, setAccount] = useState('');
  const [configure, setConfigure] = useState<PluginView>();
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
  useEffect(() => {
    void refresh()
      .then((s) => {
        if (s.credentials.length > 0) setPage('vault');
      })
      .catch((e) => setError(String(e)));
  }, [refresh]);
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
    async <T,>(task: () => Promise<T>, message?: string): Promise<T | undefined> => {
      setBusy(true);
      setError('');
      try {
        const result = await task();
        await refresh();
        if (message) setNotice(message);
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
  const startAuth = async (
    plugin: PluginView,
    config: Record<string, string>,
    connectionId?: string,
  ) => {
    setConfigure(undefined);
    const progress = await perform(async () => {
      if (!plugin.installed)
        await request({ kind: 'install', plugin_id: plugin.manifest.id, accept_permissions: true });
      return request<AuthProgress>(
        connectionId
          ? { kind: 'reconnect', connection_id: connectionId }
          : { kind: 'begin_auth', plugin_id: plugin.manifest.id, config },
      );
    });
    if (progress?.kind === 'browser') setAuth({ plugin, progress, flowId: progress.flow_id });
  };
  const connect = (plugin: PluginView) =>
    plugin.manifest.fields.length ? setConfigure(plugin) : void startAuth(plugin, {});
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
              setAuth(undefined);
              setAccount(progress.connection.id);
              setPage('vault');
              setNotice(`Connected to ${auth.plugin.manifest.name}`);
              await refresh();
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
  const control = snapshot && { snapshot, busy, perform };
  return (
    <div className="app-shell">
      <aside className="sidebar">
        <button className="brand" onClick={() => changePage('plugins')} aria-label="GetKeys home">
          <span className="brand-symbol">
            <KeyRound size={19} />
          </span>
          getkeys<span className="brand-dot">.</span>
        </button>
        <div className="workspace-label">
          <span className="workspace-avatar">P</span>
          <div>
            Personal workspace<small>On this device</small>
          </div>
        </div>
        <nav aria-label="Main navigation">
          {navigation.map(({ id, label, icon: Icon }) => (
            <button
              key={id}
              aria-current={page === id ? 'page' : undefined}
              className={page === id ? 'nav-item active' : 'nav-item'}
              onClick={() => changePage(id)}
            >
              <Icon size={17} strokeWidth={1.65} />
              {label}
              {id === 'vault' && !!snapshot?.credentials.length && (
                <span className="nav-count">
                  {snapshot.credentials.filter((c) => !c.retired).length}
                </span>
              )}
            </button>
          ))}
        </nav>
        <div className="account-heading">CONNECTED ACCOUNTS</div>
        <div className="account-list">
          {snapshot?.connections
            .filter((c) => c.state !== 'disconnected')
            .map((c) => (
              <button
                key={c.id}
                className="account-item"
                onClick={() => {
                  setAccount(c.id);
                  changePage('vault');
                }}
              >
                <ProviderMark
                  small
                  name={
                    snapshot.plugins.find((p) => p.manifest.id === c.plugin_id)?.manifest.name ??
                    c.plugin_id
                  }
                />
                <span>{c.name}</span>
                <span className={`status-dot ${c.state}`} />
              </button>
            ))}
          {!snapshot?.connections.some((c) => c.state !== 'disconnected') && (
            <p className="account-empty">Your accounts will appear here.</p>
          )}
        </div>
        <div className="sidebar-bottom">
          <button
            className={`nav-item ${page === 'settings' ? 'active' : ''}`}
            onClick={() => changePage('settings')}
          >
            <Settings size={17} strokeWidth={1.65} />
            Settings
          </button>
          <div className="vault-status">
            <ShieldCheck size={14} />
            <span>Local & encrypted</span>
            <span className="status-dot connected" />
          </div>
        </div>
      </aside>
      <div className="main-shell">
        <header className="topbar">
          <div className="breadcrumb">
            Workspace <ChevronRight size={13} />
            <strong>{page === 'vault' ? 'My keys' : page[0].toUpperCase() + page.slice(1)}</strong>
          </div>
          <div className="search">
            <Search size={16} />
            <input
              ref={searchRef}
              aria-label="Search"
              placeholder={page === 'plugins' ? 'Find a provider…' : 'Search keys…'}
              value={search}
              onChange={(e) => setSearch(e.target.value)}
            />
            <kbd>⌘ K</kbd>
          </div>
        </header>
        <main>
          {error && (
            <div role="alert" className="alert">
              <CircleHelp size={18} />
              <span>{error}</span>
              <button
                aria-label="Dismiss error"
                className="icon-button"
                onClick={() => setError('')}
              >
                <X size={15} />
              </button>
            </div>
          )}
          {!control ? (
            <div className="empty">
              <KeyRound size={28} />
              <h1>Welcome to GetKeys.</h1>
              <p>
                {error
                  ? 'Your vault could not be opened. Resolve the message above and try again.'
                  : 'Opening your local vault…'}
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
              {page === 'plugins' && <Marketplace {...control} search={search} connect={connect} />}
              {page === 'vault' && (
                <VaultPanel
                  {...control}
                  search={search}
                  account={account}
                  setAccount={setAccount}
                  browse={() => changePage('plugins')}
                  onRotation={() => changePage('rotations')}
                />
              )}
              {page === 'rotations' && <Rotations {...control} />}
              {page === 'settings' && (
                <SettingsPanel
                  {...control}
                  reconnect={(id) => {
                    const c = snapshot.connections.find((c) => c.id === id)!;
                    const p = snapshot.plugins.find((p) => p.manifest.id === c.plugin_id)!;
                    void startAuth(p, {}, id);
                  }}
                />
              )}
              {page === 'activity' && (
                <>
                  <div className="page-heading">
                    <div className="eyebrow">LOCAL AUDIT TRAIL</div>
                    <h1>Activity</h1>
                    <p>A record of actions. Secret values are never included.</p>
                  </div>
                  <div className="activity-list">
                    {snapshot.activity.map((a) => (
                      <div key={a.id}>
                        <span className="activity-icon">
                          <Check size={14} />
                        </span>
                        <div>
                          <strong>{humanize(a.action)}</strong>
                          <small className="mono">{a.subject}</small>
                        </div>
                        <time>{new Date(a.timestamp * 1000).toLocaleString()}</time>
                      </div>
                    ))}
                    {snapshot.activity.length === 0 && <p className="quiet">No activity yet.</p>}
                  </div>
                </>
              )}
            </>
          )}
        </main>
        <footer className="app-footer">
          <span>
            GETKEYS <span className="quiet">/</span> YOUR KEYS, IN YOUR HANDS
          </span>
          <span>v0.1.0</span>
        </footer>
      </div>
      {busy && (
        <div role="status" className="busy-indicator">
          <LoaderCircle size={15} className="spin" /> Working…
        </div>
      )}
      {notice && (
        <div className="toast" role="status">
          <Check size={16} />
          {notice}
        </div>
      )}
      {configure && (
        <Modal title={`Connect ${configure.manifest.name}`} onClose={() => setConfigure(undefined)}>
          <p className="quiet">
            Enter your account details, then authorize GetKeys in your browser.
          </p>
          <form
            onSubmit={(e) => {
              e.preventDefault();
              const form = new FormData(e.currentTarget);
              void startAuth(
                configure,
                Object.fromEntries([...form].map(([k, v]) => [k, String(v)])),
              );
            }}
          >
            {configure.manifest.fields.map((f) => (
              <Field key={f.key} label={f.label}>
                <input
                  name={f.key}
                  placeholder={f.placeholder}
                  required={f.required}
                  autoComplete="off"
                />
              </Field>
            ))}
            <button className="button full" disabled={busy}>
              Continue in browser <ArrowUpRight size={16} />
            </button>
          </form>
        </Modal>
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
              {auth.progress.roles.map((r) => (
                <button
                  disabled={busy}
                  className="role-choice"
                  key={`${r.account_id}/${r.role}`}
                  onClick={() =>
                    void perform(() =>
                      request<AuthProgress>({
                        kind: 'choose_role',
                        flow_id: auth.flowId,
                        account_id: r.account_id,
                        role: r.role,
                      }),
                    ).then((p) => {
                      if (p?.kind === 'connected') {
                        setAuth(undefined);
                        setAccount(p.connection.id);
                        setPage('vault');
                      }
                    })
                  }
                >
                  <strong>{r.account_name}</strong>
                  <span>{r.role}</span>
                  <ArrowUpRight size={16} />
                </button>
              ))}
            </>
          ) : (
            <div className="auth-wait">
              <ProviderMark name={auth.plugin.manifest.name} />
              <h3>Finish connecting in your browser.</h3>
              {auth.progress.kind === 'browser' && auth.progress.user_code && (
                <>
                  <p>Enter this code when prompted:</p>
                  <code className="device-code">{auth.progress.user_code}</code>
                </>
              )}
              <p className="quiet">This window will update when authorization is complete.</p>
              <LoaderCircle size={19} className="spin" />
            </div>
          )}
        </Modal>
      )}
    </div>
  );
}
