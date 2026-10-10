import { useEffect, useRef, useState, type ReactNode } from 'react';
import {
  ArrowUpRight,
  Check,
  ChevronDown,
  CircleAlert,
  Copy,
  Download,
  Eye,
  EyeOff,
  ListChecks,
  ListFilter,
  PenLine,
  Plus,
  RefreshCw,
  RotateCw,
  SquareTerminal,
  Trash2,
} from 'lucide-react';
import type {
  ConnectionView,
  CredentialView,
  Deletion,
  KeyField,
  Manifest,
  PluginView,
  Rotation,
} from './types';
import type { Controller } from './controller';
import { copyCredential, exportEnv, request } from './bridge';
import {
  Checkbox,
  Empty,
  Field,
  inputs,
  Menu,
  Modal,
  PageHeader,
  ProviderMark,
  sentence,
  Tabs,
} from './ui';
import { formatDay, isFinished, RotationProgress, rotationsFor } from './RotationProgress';
import { CustomKeyForm, NewKeyDialog } from './NewKey';
import {
  COPIED,
  CUSTOM,
  DOTENV,
  FAILED,
  listed,
  MASK,
  primaryField,
  shown,
  SIGN_IN,
  useReveal,
} from './secrets';

function formatDate(value: string) {
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? value : date.toLocaleDateString();
}
const plural = (n: number, word: string) => `${n} ${word}${n === 1 ? '' : 's'}`;

type Item = {
  c: CredentialView;
  /** None for a key that belongs to no provider. */
  connection?: ConnectionView;
  manifest?: Manifest;
  provider: string;
  /** Where the key lives: its project or repository, or the account itself. */
  place: string;
  /** Listed by the provider, which never shows its secret again. */
  secretless: boolean;
  /** Found on this device; its account signs in with it, so the device keeps it current. */
  local: boolean;
  custom: boolean;
  /** A provider's key kept on its own, outside any account. */
  own: boolean;
};
/** Keys with a secret, keys without one, or a group's keys (`group:<name>`). */
type View = 'ready' | 'empty' | `group:${string}`;
const GROUPS_KEY = 'getkeys.groups';
/** Groups made with + that may not have keys yet. */
const storedGroups = (): string[] => {
  try {
    const saved: unknown = JSON.parse(localStorage.getItem(GROUPS_KEY) ?? '[]');
    return Array.isArray(saved) ? saved.filter((g) => typeof g === 'string') : [];
  } catch {
    return [];
  }
};
/** Narrows the keys in the tab being viewed. */
type Filter = { providers: string[]; states: Status[] };
type Status = 'invalid' | 'rotating';
const NO_FILTER: Filter = { providers: [], states: [] };
const STATUSES: { id: Status; label: string }[] = [
  { id: 'invalid', label: 'Invalid' },
  { id: 'rotating', label: 'Rotating' },
];
/** Which keys Refresh covers. */
export type RefreshScope = { connections?: string[]; keys?: string[]; label?: string };

type Props = Controller & {
  search: string;
  searchField: ReactNode;
  browse: () => void;
  refreshKeys: (scope?: RefreshScope) => void;
  /** Connects a provider, from this device's sign-ins or by signing in. */
  add: (plugin: PluginView) => void;
  /** A key to show, from a getkeys://key link. */
  focus?: { id: string };
};
export function VaultPanel({
  snapshot,
  busy,
  perform,
  search,
  searchField,
  browse,
  refreshKeys,
  add,
  focus,
}: Props) {
  const [view, setView] = useState<View>('ready');
  const [created, setCreated] = useState(storedGroups);
  const [naming, setNaming] = useState(false);
  const [picking, setPicking] = useState<string>();
  const [open, setOpen] = useState('');
  const [editing, setEditing] = useState<CredentialView>();
  const [adding, setAdding] = useState(false);
  const [exporting, setExporting] = useState(false);
  const [filter, setFilter] = useState<Filter>(NO_FILTER);
  const query = search.trim().toLowerCase();
  const all: Item[] = snapshot.credentials
    .flatMap<Item>((c) => {
      if (!listed(c, snapshot.connections)) return [];
      if (c.credential_type === CUSTOM)
        return [
          {
            c,
            provider: 'Custom',
            place: 'No provider',
            secretless: false,
            local: false,
            custom: true,
            own: false,
          },
        ];
      if (c.plugin_id) {
        const manifest = snapshot.plugins.find((p) => p.manifest.id === c.plugin_id)?.manifest;
        if (!manifest) return [];
        return [
          {
            c,
            manifest,
            provider: manifest.name,
            place: c.id.startsWith('found:') ? 'On this device' : 'Added by you',
            secretless: false,
            local: false,
            custom: false,
            own: true,
          },
        ];
      }
      const connection = snapshot.connections.find((x) => x.id === c.connection_id);
      const manifest = snapshot.plugins.find(
        (p) => p.manifest.id === connection?.plugin_id,
      )?.manifest;
      if (!connection || !manifest) return [];
      const local = c.credential_type === SIGN_IN;
      const place = c.resource ? c.resource_name || c.resource : connection.name;
      const secretless = !c.value_available && !manifest.operations.includes('read_credential');
      return [
        {
          c,
          connection,
          manifest,
          provider: manifest.name,
          place,
          secretless,
          local,
          custom: false,
          own: false,
        },
      ];
    })
    .sort(
      (a, b) =>
        Number(a.custom) - Number(b.custom) ||
        a.provider.localeCompare(b.provider) ||
        (a.connection?.name ?? '').localeCompare(b.connection?.name ?? '') ||
        a.place.localeCompare(b.place) ||
        a.c.name.localeCompare(b.c.name),
    );
  const groups = [...new Set([...created, ...all.flatMap((i) => i.c.groups)])].sort(
    (a, b) => Number(b === FAILED) - Number(a === FAILED) || a.localeCompare(b),
  );
  const searched = all.filter((i) =>
    [
      i.c.name,
      i.provider,
      i.place,
      i.connection?.name,
      ...i.c.groups,
      ...i.c.fields.map((f) => f.value),
    ]
      .join(' ')
      .toLowerCase()
      .includes(query),
  );
  const named = view.startsWith('group:') ? view.slice(6) : '';
  const activeGroup = groups.includes(named) ? named : '';
  const hasEmpty = all.some((i) => i.secretless);
  const shown: View = activeGroup ? view : view === 'empty' && hasEmpty ? 'empty' : 'ready';
  const inView = (i: Item, v: View) =>
    v.startsWith('group:') ? i.c.groups.includes(v.slice(6)) : i.secretless === (v === 'empty');
  const inTab = searched.filter((i) => inView(i, shown));
  const status = (i: Item, s: Status) =>
    s === 'invalid'
      ? !!i.c.problem
      : rotationsFor(snapshot, i.c.id).some((r) => r.credential_id === i.c.id && !isFinished(r));
  const filtering = filter.providers.length + filter.states.length;
  const items = inTab.filter(
    (i) =>
      (!filter.providers.length || filter.providers.includes(i.provider)) &&
      (!filter.states.length || filter.states.some((s) => status(i, s))),
  );
  const tabProviders = [...new Set(all.filter((i) => inView(i, shown)).map((i) => i.provider))];
  const toggleFilter = <K extends keyof Filter>(key: K, value: Filter[K][number]) =>
    setFilter((f) => ({
      ...f,
      [key]: (f[key] as string[]).includes(value)
        ? (f[key] as string[]).filter((x) => x !== value)
        : [...f[key], value],
    }));
  // A filter belongs to the tab it was set in.
  useEffect(() => setFilter(NO_FILTER), [shown]);
  const inGroup = all.filter((i) => !activeGroup || i.c.groups.includes(activeGroup));
  const exportable = inGroup.filter((i) => !i.secretless && envFields(i.c).length > 0);
  const views: { id: View; label: string }[] = [
    { id: 'ready', label: 'Ready to use' },
    ...(hasEmpty ? [{ id: 'empty' as const, label: 'Empty profiles' }] : []),
    ...groups.map((g) => ({ id: `group:${g}` as const, label: g })),
  ];
  const tabs = views.map((t) => ({ ...t, count: searched.filter((i) => inView(i, t.id)).length }));
  const remember = (next: string[]) => {
    setCreated(next);
    localStorage.setItem(GROUPS_KEY, JSON.stringify(next));
  };
  const createGroup = (input: string) => {
    setNaming(false);
    const name = input.trim().slice(0, 40);
    if (!name) return;
    if (!groups.includes(name)) {
      remember([...created, name]);
      setPicking(name);
    }
    setView(`group:${name}`);
  };
  const deleteGroup = (name: string) => {
    remember(created.filter((g) => g !== name));
    setView('ready');
    const members = all.filter((i) => i.c.groups.includes(name));
    if (members.length)
      void perform(async () => {
        for (const i of members)
          await request({
            kind: 'set_groups',
            credential_id: i.c.id,
            groups: i.c.groups.filter((g) => g !== name),
          });
      }, `Deleted ${name}`);
  };
  const retired = snapshot.credentials.find((c) => c.id === open)?.retired;
  useEffect(() => {
    // A completed rotation retires the old key; follow it to its replacement.
    if (retired)
      setOpen(
        (old) =>
          snapshot.rotations.find((r) => r.credential_id === old && r.state.kind === 'complete')
            ?.candidate_id ?? '',
      );
  }, [retired]);
  const focused = useRef(focus);
  useEffect(() => {
    const item = focus && all.find((i) => i.c.id === focus.id);
    if (!item || focused.current === focus) return;
    focused.current = focus;
    setView(item.secretless ? 'empty' : 'ready');
    setOpen(item.c.id);
    setTimeout(() =>
      document.querySelector('.key-item.open')?.scrollIntoView?.({ block: 'nearest' }),
    );
  }, [focus, snapshot]);
  const active = snapshot.connections.filter((c) => c.state !== 'disconnected');
  // Refresh covers the accounts behind the group's keys, or every account.
  const scope: RefreshScope = activeGroup
    ? {
        connections: [
          ...new Set(
            inGroup.flatMap((i) =>
              i.connection?.state === 'disconnected' ? [] : (i.connection?.id ?? []),
            ),
          ),
        ],
        keys: inGroup.filter((i) => i.own).map((i) => i.c.id),
        label: activeGroup,
      }
    : {};
  const canRefresh = scope.connections
    ? scope.connections.length + (scope.keys?.length ?? 0) > 0
    : active.length > 0 || snapshot.plugins.some((p) => p.installed);
  const newKey = (
    <button className="button" disabled={busy} onClick={() => setAdding(true)}>
      <Plus size={14} /> New key
    </button>
  );
  const dialogs = (
    <>
      {adding && (
        <NewKeyDialog
          {...{ snapshot, busy, perform, browse }}
          group={activeGroup || undefined}
          connect={add}
          close={() => setAdding(false)}
        />
      )}
      {editing && (
        <KeyForm
          {...{ snapshot, busy, perform }}
          credential={editing}
          close={() => setEditing(undefined)}
        />
      )}
    </>
  );
  if (!active.length && !all.length)
    return (
      <>
        <PageHeader title="Keys" />
        <Empty
          title="No keys yet"
          action={
            <div className="button-row">
              <button className="button" onClick={browse}>
                Add a provider <ArrowUpRight size={14} />
              </button>
              <button className="button outline" disabled={busy} onClick={() => setAdding(true)}>
                <Plus size={14} /> New key
              </button>
            </div>
          }
        >
          Add a provider such as OpenAI, GitHub, or AWS. GetKeys brings in the keys already on this
          device and in your accounts, and makes new ones without visiting their sites.
        </Empty>
        {dialogs}
      </>
    );
  return (
    <>
      <PageHeader title="Keys">
        {searchField}
        <div className="button-group" role="group" aria-label="All keys">
          <button
            className="button outline"
            title={
              activeGroup
                ? `Refresh the accounts behind ${activeGroup}`
                : 'Look for new keys on this device and in your accounts'
            }
            aria-label="Refresh keys"
            disabled={busy || !canRefresh}
            onClick={() => refreshKeys(scope)}
          >
            <RefreshCw size={14} strokeWidth={1.75} /> Refresh
          </button>
          <button
            className="button outline"
            title={activeGroup ? `Export ${activeGroup} as .env` : 'Export keys as .env'}
            aria-label="Export keys"
            disabled={busy || !exportable.length}
            onClick={() => setExporting(true)}
          >
            <Download size={14} strokeWidth={1.75} /> Export
          </button>
        </div>
        {newKey}
      </PageHeader>
      <div className="key-toolbar">
        <div className="tab-row">
          <Tabs label="Key groups" value={shown} onChange={setView} tabs={tabs} />
          {naming ? (
            <input
              className="tab-input"
              aria-label="New group name"
              placeholder="Group name"
              maxLength={40}
              autoFocus
              onKeyDown={(e) => {
                if (e.key !== 'Enter' && e.key !== 'Escape') return;
                // The key would otherwise press the button the new dialog focuses.
                e.preventDefault();
                if (e.key === 'Escape') e.currentTarget.value = '';
                e.currentTarget.blur();
              }}
              onBlur={(e) => createGroup(e.currentTarget.value)}
            />
          ) : (
            <button
              type="button"
              className="icon-button outline"
              aria-label="New group"
              title="New group"
              onClick={() => setNaming(true)}
            >
              <Plus size={15} strokeWidth={1.75} />
            </button>
          )}
        </div>
        <div className="group-actions">
          <Menu
            label="Filter"
            title={`Filter ${tabs.find((t) => t.id === shown)?.label ?? 'keys'}`}
            className={`button ghost small ${filtering ? 'active' : ''}`}
            align="end"
            button={
              <>
                <ListFilter size={14} strokeWidth={1.75} /> Filter
                {filtering > 0 && <span className="count">{filtering}</span>}
              </>
            }
          >
            {(close) => (
              <>
                <div className="menu-heading">Provider</div>
                {tabProviders.map((p) => (
                  <button
                    key={p}
                    type="button"
                    role="menuitemcheckbox"
                    aria-checked={filter.providers.includes(p)}
                    className="menu-item"
                    onClick={() => toggleFilter('providers', p)}
                  >
                    <Check size={13} strokeWidth={2.25} className="menu-check" />
                    <ProviderMark small name={p} /> {p}
                  </button>
                ))}
                <div className="menu-heading">Status</div>
                {STATUSES.map((st) => (
                  <button
                    key={st.id}
                    type="button"
                    role="menuitemcheckbox"
                    aria-checked={filter.states.includes(st.id)}
                    className="menu-item"
                    onClick={() => toggleFilter('states', st.id)}
                  >
                    <Check size={13} strokeWidth={2.25} className="menu-check" /> {st.label}
                  </button>
                ))}
                {filtering > 0 && (
                  <>
                    <div className="menu-separator" />
                    <button
                      type="button"
                      role="menuitem"
                      className="menu-item"
                      onClick={() => {
                        setFilter(NO_FILTER);
                        close();
                      }}
                    >
                      Clear filter
                    </button>
                  </>
                )}
              </>
            )}
          </Menu>
          {activeGroup && (
            <>
              <button
                className="button ghost small"
                disabled={busy}
                onClick={() => setPicking(activeGroup)}
              >
                <ListChecks size={14} strokeWidth={1.75} /> Choose keys
              </button>
              <button
                className="button ghost small danger"
                disabled={busy}
                onClick={() => deleteGroup(activeGroup)}
              >
                <Trash2 size={14} strokeWidth={1.75} /> Delete group
              </button>
            </>
          )}
        </div>
      </div>
      {shown === 'empty' && (
        <p className="tab-note">
          These profiles exist in your accounts, but their providers show a secret only once, when
          it’s created. Fill one in if you have it, rotate it for a fresh one, or delete it.
        </p>
      )}
      {activeGroup === FAILED && (
        <p className="tab-note">
          The provider refused these keys during the last refresh: the account can’t list them, or
          the key was rejected. They leave this group once a refresh gets through.
        </p>
      )}
      {items.length ? (
        <ul className="key-items" aria-label="Keys">
          {items.map((i) => (
            <KeyItem
              key={i.c.id}
              item={i}
              control={{ snapshot, busy, perform }}
              open={open === i.c.id}
              toggle={() => setOpen(open === i.c.id ? '' : i.c.id)}
              expand={() => setOpen(i.c.id)}
              edit={() => setEditing(i.c)}
              refreshKeys={refreshKeys}
              groups={groups}
              openGroup={(g) => setView(`group:${g}`)}
            />
          ))}
        </ul>
      ) : (
        <Empty
          title={search || filtering ? 'No matching keys' : 'No keys yet'}
          action={
            filtering > 0 && (
              <button className="button outline" onClick={() => setFilter(NO_FILTER)}>
                Clear filter
              </button>
            )
          }
        >
          {search || filtering
            ? 'Try a different search or filter.'
            : activeGroup
              ? `Nothing in ${activeGroup} yet. Choose keys to add them.`
              : 'Refresh to look for keys again, or add one with New key.'}
        </Empty>
      )}
      {dialogs}
      {picking && (
        <GroupDialog
          {...{ snapshot, busy, perform }}
          group={picking}
          items={all}
          close={() => setPicking(undefined)}
        />
      )}
      {exporting && (
        <ExportDialog
          {...{ snapshot, busy, perform }}
          items={exportable}
          group={activeGroup}
          close={() => setExporting(false)}
        />
      )}
    </>
  );
}

/** The fields of a key that a `.env` file carries. */
const envFields = (c: CredentialView) => c.fields.filter((f) => f.env && f.available);
const token = (s: string) =>
  s
    .toUpperCase()
    .replace(/[^A-Z0-9]+/g, '_')
    .replace(/^_+|_+$/g, '');

/** Variable names for the chosen keys; names used twice get the key's project or name added. */
export function envLines(items: { c: CredentialView; place: string }[]) {
  const all = items.flatMap((i) => envFields(i.c).map((f) => ({ i, f })));
  const count = (env: string) => all.filter((x) => x.f.env === env).length;
  const used = new Set<string>();
  return all.map(({ i, f }) => {
    let variable = f.env;
    if (count(f.env) > 1) {
      const base = `${f.env}_${token(i.place === 'No provider' ? i.c.name : i.place) || 'KEY'}`;
      variable = base;
      for (let n = 2; used.has(variable); n++) variable = `${base}_${n}`;
    }
    used.add(variable);
    return { credential_id: i.c.id, field: f.key, variable };
  });
}

function ExportDialog({
  items,
  group,
  close,
  busy,
  perform,
}: Controller & { items: Item[]; group: string; close: () => void }) {
  const [chosen, setChosen] = useState(() => new Set(items.map((i) => i.c.id)));
  const picked = items.filter((i) => chosen.has(i.c.id));
  const lines = envLines(picked);
  const all = chosen.size === items.length;
  const toggle = (id: string) =>
    setChosen((s) => {
      const next = new Set(s);
      if (!next.delete(id)) next.add(id);
      return next;
    });
  const run = (save: boolean) =>
    void perform(
      () => exportEnv(lines, save),
      (done) =>
        !done
          ? undefined
          : save
            ? `Saved ${plural(lines.length, 'variable')}`
            : `Copied ${plural(lines.length, 'variable')}. Clipboard clears in 60 seconds.`,
    ).then((done) => {
      if (done) close();
    });
  return (
    <Modal
      title={group ? `Export ${group}` : 'Export keys'}
      description="Writes the chosen keys as a .env file. Names used by more than one key get the project added."
      onClose={close}
    >
      <div className="check-list-heading">
        <Checkbox
          checked={all}
          mixed={!all && chosen.size > 0}
          onChange={() => setChosen(all ? new Set() : new Set(items.map((i) => i.c.id)))}
          label={`${chosen.size} of ${plural(items.length, 'key')}`}
        />
        <span className="quiet">{plural(lines.length, 'variable')}</span>
      </div>
      <ul className="check-list" aria-label="Keys to export">
        {items.map((i) => {
          const names = lines.filter((l) => l.credential_id === i.c.id).map((l) => l.variable);
          return (
            <li key={i.c.id}>
              <Checkbox
                checked={chosen.has(i.c.id)}
                onChange={() => toggle(i.c.id)}
                label={
                  <span className="check-item">
                    <ProviderMark small name={i.provider} />
                    <span className="check-item-text">
                      <strong>{i.c.name}</strong>
                      <small className="mono">
                        {(names.length ? names : envFields(i.c).map((f) => f.env)).join('  ')}
                      </small>
                    </span>
                  </span>
                }
              />
            </li>
          );
        })}
      </ul>
      <div className="modal-footer">
        <button
          className="button outline"
          disabled={busy || !lines.length}
          onClick={() => run(false)}
        >
          <Copy size={14} strokeWidth={1.75} /> Copy
        </button>
        <button className="button" disabled={busy || !lines.length} onClick={() => run(true)}>
          <Download size={14} strokeWidth={1.75} /> Save .env
        </button>
      </div>
    </Modal>
  );
}

/** Picks the keys in a group. */
function GroupDialog({
  group,
  items,
  close,
  busy,
  perform,
}: Controller & { group: string; items: Item[]; close: () => void }) {
  const [chosen, setChosen] = useState(
    () => new Set(items.filter((i) => i.c.groups.includes(group)).map((i) => i.c.id)),
  );
  const changed = items.filter((i) => i.c.groups.includes(group) !== chosen.has(i.c.id));
  const toggle = (id: string) =>
    setChosen((s) => {
      const next = new Set(s);
      if (!next.delete(id)) next.add(id);
      return next;
    });
  const save = () =>
    void perform(
      async () => {
        for (const i of changed)
          await request({
            kind: 'set_groups',
            credential_id: i.c.id,
            groups: chosen.has(i.c.id)
              ? [...i.c.groups, group]
              : i.c.groups.filter((g) => g !== group),
          });
        return true;
      },
      `${group} has ${plural(chosen.size, 'key')}`,
    ).then((done) => {
      if (done) close();
    });
  return (
    <Modal title={group} description="Choose the keys in this group." onClose={close}>
      <div className="check-list-heading">
        <span>{plural(chosen.size, 'key')} chosen</span>
      </div>
      <ul className="check-list" aria-label={`Keys in ${group}`}>
        {items.map((i) => (
          <li key={i.c.id}>
            <Checkbox
              checked={chosen.has(i.c.id)}
              onChange={() => toggle(i.c.id)}
              label={
                <span className="check-item">
                  <ProviderMark small name={i.provider} />
                  <span className="check-item-text">
                    <strong>{i.c.name}</strong>
                    <small>{i.custom ? i.provider : `${i.provider} · ${i.place}`}</small>
                  </span>
                </span>
              }
            />
          </li>
        ))}
      </ul>
      <div className="modal-footer">
        <button className="button outline" onClick={close}>
          Cancel
        </button>
        <button className="button" disabled={busy || !changed.length} onClick={save}>
          Save
        </button>
      </div>
    </Modal>
  );
}

function KeyItem({
  item,
  control,
  open,
  toggle,
  expand,
  edit,
  refreshKeys,
  groups,
  openGroup,
}: {
  item: Item;
  control: Controller;
  open: boolean;
  toggle: () => void;
  expand: () => void;
  /** Fills in a listed key's secret, or replaces a custom key's values. */
  edit: () => void;
  refreshKeys: (scope?: RefreshScope) => void;
  groups: string[];
  openGroup: (group: string) => void;
}) {
  const { c, connection, manifest, provider, place, secretless, local, custom } = item;
  const { busy, perform, snapshot } = control;
  const [secret, reveal] = useReveal(c.id, perform);
  const [dialog, setDialog] = useState<'rotate' | 'guided' | 'native' | 'paste' | 'delete'>();
  const primary = primaryField(c);
  const value = primary && shown(primary, secret);
  const hidden = c.fields.some((f) => f.secret && f.available);
  const rotation = rotationsFor(snapshot, c.id).find((r) => !isFinished(r));
  const guided = !!manifest?.operations.includes('guide');
  const native = !!connection && !!manifest?.operations.includes('rotate_credential');
  // Without a way to rotate through the provider, its page opens and the new value is pasted.
  const pastes = !!manifest?.token_page && inputs(manifest.credential_fields).length > 0;
  const rotate = rotation
    ? expand
    : custom
      ? edit
      : c.can_rotate || local
        ? () => setDialog('rotate')
        : native
          ? () => setDialog('native')
          : guided
            ? () => setDialog('guided')
            : pastes
              ? () => setDialog('paste')
              : undefined;
  const copy = (f: KeyField) =>
    void perform(
      () => copyCredential(c.id, f.key),
      f.secret ? COPIED : `Copied ${f.label.toLowerCase()}`,
    );
  const close = () => setDialog(undefined);
  return (
    <li className={`key-item ${open ? 'open' : ''} ${c.problem ? 'invalid' : ''}`}>
      <div
        className="key-line"
        onClick={(e) => {
          // The row opens its details, except from its buttons or while selecting text.
          const target = e.target as HTMLElement;
          if (!target.closest('button, input, a') && !window.getSelection()?.toString()) toggle();
        }}
      >
        <ProviderMark small name={provider} />
        <div className="key-name">
          <span className="key-title">
            <strong>{c.name}</strong>
            {rotation && <span className="badge active">Rotating</span>}
            {c.problem && (
              <span className="badge invalid" title={c.problem}>
                <CircleAlert size={11} strokeWidth={2.25} /> Invalid
              </span>
            )}
          </span>
          <small>
            {custom ? provider : `${provider} · ${place}`}
            {c.groups.map((g) => (
              <span className="group-tag" key={g}>
                {g}
              </span>
            ))}
          </small>
        </div>
        {secretless ? (
          <span className="key-value missing">No secret on this device</span>
        ) : (
          <code
            className={`key-value ${primary && value === undefined ? 'masked' : ''}`}
            title={primary?.label}
          >
            {primary ? (value ?? MASK) : ''}
          </code>
        )}
        <div className="key-buttons">
          {secretless ? (
            <button
              className="button outline small"
              disabled={busy}
              aria-label={`Fill in ${c.name}`}
              onClick={edit}
            >
              <PenLine size={13} strokeWidth={1.75} /> Fill in
            </button>
          ) : (
            <>
              <button
                className="icon-button outline"
                title={secret ? 'Hide' : 'Reveal for 30 seconds'}
                aria-label={`${secret ? 'Hide' : 'Reveal'} ${c.name}`}
                disabled={busy || !hidden}
                onClick={reveal}
              >
                {secret ? <EyeOff size={15} /> : <Eye size={15} />}
              </button>
              <button
                className="icon-button outline"
                title={primary ? `Copy ${primary.label.toLowerCase()}` : 'Nothing stored to copy'}
                aria-label={`Copy ${c.name}`}
                disabled={busy || !primary}
                onClick={() => primary && copy(primary)}
              >
                <Copy size={15} strokeWidth={1.75} />
              </button>
            </>
          )}
          <button
            className="icon-button outline"
            title={
              rotation
                ? 'Rotation in progress'
                : rotate
                  ? 'Rotate'
                  : `${provider} can’t rotate this key`
            }
            aria-label={`Rotate ${c.name}`}
            disabled={busy || connection?.state === 'disconnected' || !rotate}
            onClick={rotate}
          >
            <RotateCw size={15} strokeWidth={1.75} />
          </button>
          <button
            className="icon-button outline danger"
            title={rotation ? 'Finish the rotation first' : 'Delete'}
            aria-label={`Delete ${c.name}`}
            disabled={busy || !!rotation}
            onClick={() => setDialog('delete')}
          >
            <Trash2 size={15} strokeWidth={1.75} />
          </button>
          <button
            className="icon-button key-toggle"
            aria-expanded={open}
            aria-label={`Details for ${c.name}`}
            title="Details"
            onClick={toggle}
          >
            <ChevronDown size={16} strokeWidth={1.75} />
          </button>
        </div>
      </div>
      {open && (
        <KeyDetails
          {...control}
          item={item}
          secret={secret}
          copy={copy}
          groups={groups}
          openGroup={openGroup}
        />
      )}
      {dialog && local ? (
        <LocalKeyDialog
          {...control}
          item={item}
          close={close}
          deleting={dialog === 'delete'}
          refreshKeys={refreshKeys}
        />
      ) : (
        <>
          {dialog === 'rotate' && (
            <RotateDialog {...control} item={item} close={close} done={expand} />
          )}
          {dialog === 'guided' && (
            <GuidedDialog {...control} item={item} close={close} done={expand} />
          )}
          {dialog === 'native' && <NativeRotateDialog {...control} item={item} close={close} />}
          {dialog === 'paste' && <PasteRotateDialog {...control} item={item} close={close} />}
          {dialog === 'delete' && <DeleteDialog {...control} item={item} close={close} />}
        </>
      )}
    </li>
  );
}

function KeyDetails({
  item,
  secret,
  copy,
  groups,
  openGroup,
  ...control
}: Controller & {
  item: Item;
  secret?: Record<string, string>;
  copy: (f: KeyField) => void;
  groups: string[];
  openGroup: (group: string) => void;
}) {
  const { c: credential, connection, custom, own } = item;
  const { busy, perform, snapshot } = control;
  const rotations = rotationsFor(snapshot, credential.id);
  const active = rotations.find((r) => !isFinished(r));
  const last = rotations.find(isFinished);
  const exported = credential.fields.filter((f) => f.env && f.available).length;
  return (
    <div className="key-details" role="region" aria-label={`${credential.name} details`}>
      <div className="key-details-main">
        {credential.problem && (
          <p className="callout danger">
            <CircleAlert size={14} strokeWidth={2} /> {credential.problem}
          </p>
        )}
        <dl className="key-fields">
          {credential.fields.map((f) => {
            const value = shown(f, secret);
            return (
              <div className="key-field" key={f.key}>
                <dt>
                  <span>{f.label}</span>
                  {f.env && <span className="key-env mono">{f.env}</span>}
                </dt>
                <dd>
                  {f.available ? (
                    <code className={value === undefined ? 'masked' : 'revealed'}>
                      {value ?? MASK}
                    </code>
                  ) : (
                    <span className="key-missing">Not on this device</span>
                  )}
                  <button
                    className="icon-button"
                    title="Copy"
                    aria-label={`Copy ${f.label}`}
                    disabled={busy || !f.available}
                    onClick={() => copy(f)}
                  >
                    <Copy size={14} strokeWidth={1.75} />
                  </button>
                </dd>
              </div>
            );
          })}
        </dl>
        {credential.value_available && exported > 0 && (
          <div className="key-actions">
            <button
              className="button outline small"
              disabled={busy}
              title="Copy as environment variables. Clipboard clears in 30 seconds."
              onClick={() =>
                void perform(
                  () => copyCredential(credential.id, DOTENV),
                  `Copied ${plural(exported, 'variable')} as .env. Clipboard clears in 30 seconds.`,
                )
              }
            >
              <SquareTerminal size={14} /> Copy .env
            </button>
          </div>
        )}
        {active && <RotationProgress {...control} rotation={active} />}
        {!active && last && (
          <p className="rotation-history quiet">
            Last rotation {sentence(last.state.kind).toLowerCase()} · {formatDay(last.created_at)}
          </p>
        )}
      </div>
      <div className="key-details-side">
        <dl className="metadata">
          <dt>{custom || own ? 'Provider' : 'Account'}</dt>
          <dd>
            {own
              ? `${item.provider}, kept on its own`
              : (connection?.name ?? 'None, added by hand')}
          </dd>
          {credential.resource && (
            <>
              <dt>Belongs to</dt>
              <dd className="mono">{credential.resource_name || credential.resource}</dd>
            </>
          )}
          {credential.created_at && (
            <>
              <dt>Created</dt>
              <dd>{formatDate(credential.created_at)}</dd>
            </>
          )}
          {credential.expires_at && (
            <>
              <dt>Expires</dt>
              <dd>{formatDate(credential.expires_at)}</dd>
            </>
          )}
        </dl>
        <GroupEditor {...control} credential={credential} groups={groups} open={openGroup} />
      </div>
    </div>
  );
}

/** The groups a key is in; each opens its group, and + files the key under another. */
function GroupEditor({
  credential,
  groups,
  open,
  busy,
  perform,
}: Controller & { credential: CredentialView; groups: string[]; open: (group: string) => void }) {
  const [naming, setNaming] = useState(false);
  const save = (next: string[]) =>
    void perform(() =>
      request<CredentialView>({ kind: 'set_groups', credential_id: credential.id, groups: next }),
    );
  const toggle = (g: string) =>
    save(
      credential.groups.includes(g)
        ? credential.groups.filter((x) => x !== g)
        : [...credential.groups, g],
    );
  return (
    <div className="detail-section">
      <div className="detail-section-heading">
        <h2>Groups</h2>
      </div>
      <div className="group-links">
        {credential.groups.map((g) => (
          <button
            type="button"
            className="group-link"
            key={g}
            title={`Show ${g}`}
            onClick={() => open(g)}
          >
            {g}
          </button>
        ))}
        {!credential.groups.length && <span className="quiet">None</span>}
        <Menu
          label={`Groups for ${credential.name}`}
          title="Add to a group"
          className="icon-button small"
          button={<Plus size={13} strokeWidth={2} />}
        >
          {(close) =>
            naming ? (
              <input
                className="menu-input"
                aria-label="New group name"
                placeholder="Group name"
                maxLength={40}
                autoFocus
                onKeyDown={(e) => {
                  if (e.key !== 'Enter') return;
                  e.preventDefault();
                  const name = e.currentTarget.value.trim();
                  setNaming(false);
                  close();
                  if (name && !credential.groups.includes(name)) save([...credential.groups, name]);
                }}
                onBlur={() => setNaming(false)}
              />
            ) : (
              <>
                {groups.map((g) => (
                  <button
                    key={g}
                    type="button"
                    role="menuitemcheckbox"
                    aria-checked={credential.groups.includes(g)}
                    className="menu-item"
                    disabled={busy}
                    onClick={() => toggle(g)}
                  >
                    <Check size={13} strokeWidth={2.25} className="menu-check" /> {g}
                  </button>
                ))}
                {groups.length > 0 && <div className="menu-separator" />}
                <button
                  type="button"
                  role="menuitem"
                  className="menu-item"
                  onClick={() => setNaming(true)}
                >
                  <Plus size={13} strokeWidth={2} className="menu-check" /> New group
                </button>
              </>
            )
          }
        </Menu>
      </div>
    </div>
  );
}

type DialogProps = Controller & { item: Item; close: () => void; done?: () => void };

function RotateDialog({ item, close, done, busy, perform }: DialogProps) {
  const { c } = item;
  return (
    <Modal title={`Rotate ${c.name}`} onClose={close}>
      <p>
        GetKeys creates a replacement and keeps this key working until you’ve swapped the new one in
        wherever you use it. Then it retires this one.
      </p>
      <form
        onSubmit={(e) => {
          e.preventDefault();
          const name = String(new FormData(e.currentTarget).get('name'));
          void perform(() =>
            request<Rotation>({ kind: 'plan_rotation', credential_id: c.id, name }),
          ).then((r) => {
            if (!r) return;
            close();
            done?.();
          });
        }}
      >
        <Field label="Replacement name">
          <input name="name" defaultValue={`${c.name} replacement`} required />
        </Field>
        <div className="modal-footer">
          <button type="button" className="button outline" onClick={close}>
            Cancel
          </button>
          <button className="button" disabled={busy}>
            Review rotation plan
          </button>
        </div>
      </form>
    </Modal>
  );
}

function GuidedDialog({ item, close, done, snapshot, busy, perform }: DialogProps) {
  const { c: credential } = item;
  const manifest = item.manifest!;
  const [message, setMessage] = useState('');
  return (
    <Modal title={`Replace ${credential.name}`} onClose={close}>
      <p>
        Create a replacement in {manifest.name} with the access you need, add it with New key, then
        choose it here. GetKeys retires this key once you’ve swapped the new one in.
      </p>
      <button
        className="button outline full"
        disabled={busy}
        onClick={() =>
          void perform(() =>
            request<string>({
              kind: 'guide',
              connection_id: credential.connection_id,
              resource: credential.resource,
            }),
          ).then((m) => {
            if (m) setMessage(m);
          })
        }
      >
        Open {manifest.name} <ArrowUpRight size={14} />
      </button>
      {message && <p className="callout">{message}</p>}
      <form
        onSubmit={(e) => {
          e.preventDefault();
          const replacementId = String(new FormData(e.currentTarget).get('replacement'));
          void perform(() =>
            request<Rotation>({
              kind: 'plan_replacement',
              credential_id: credential.id,
              replacement_id: replacementId,
            }),
          ).then((r) => {
            if (!r) return;
            close();
            done?.();
          });
        }}
      >
        <Field label="Replacement key">
          <select name="replacement" required defaultValue="">
            <option value="" disabled>
              Choose a different saved key
            </option>
            {snapshot.credentials
              .filter(
                (c) =>
                  c.id !== credential.id &&
                  c.connection_id === credential.connection_id &&
                  c.resource === credential.resource &&
                  c.value_available &&
                  !c.retired,
              )
              .map((c) => (
                <option key={c.id} value={c.id}>
                  {c.name}
                </option>
              ))}
          </select>
        </Field>
        <div className="modal-footer">
          <button type="button" className="button outline" onClick={close}>
            Cancel
          </button>
          <button className="button" disabled={busy}>
            Use this replacement
          </button>
        </div>
      </form>
    </Modal>
  );
}

/** A key found on this device lives in its CLI, profile, or variable, so it changes there. */
function LocalKeyDialog({
  item,
  close,
  deleting,
  refreshKeys,
  busy,
  perform,
}: DialogProps & { deleting: boolean; refreshKeys: () => void }) {
  const { c } = item;
  const connection = item.connection!;
  const manifest = item.manifest!;
  if (deleting)
    return (
      <Modal title={`Delete ${c.name}?`} onClose={close}>
        <p>
          This key lives in your {c.name}, so GetKeys can’t delete it. Remove the {connection.name}{' '}
          account to stop using it here; the key itself stays where it is.
        </p>
        <div className="modal-footer">
          <button type="button" className="button outline" onClick={close}>
            Cancel
          </button>
          <button
            className="button destructive"
            disabled={busy}
            onClick={() =>
              void perform(
                () => request({ kind: 'disconnect', connection_id: connection.id }),
                'Account removed',
              ).then((r) => {
                if (r !== undefined) close();
              })
            }
          >
            <Trash2 size={14} strokeWidth={1.75} /> Remove account
          </button>
        </div>
      </Modal>
    );
  return (
    <Modal title={`Rotate ${c.name}`} onClose={close}>
      <p>
        This key comes from your {c.name}, and GetKeys reads it from there. Replace it in{' '}
        {manifest.name} and update your {c.name}, then refresh to pick up the new one.
      </p>
      <div className="modal-footer">
        {manifest.token_page && (
          <button
            className="button outline"
            disabled={busy}
            onClick={() =>
              void perform(() => request({ kind: 'open_token_page', plugin_id: manifest.id }))
            }
          >
            Open {manifest.name} <ArrowUpRight size={14} />
          </button>
        )}
        <button
          className="button"
          disabled={busy}
          onClick={() => {
            close();
            refreshKeys();
          }}
        >
          <RefreshCw size={14} strokeWidth={1.75} /> Refresh
        </button>
      </div>
    </Modal>
  );
}

/** Has the provider replace the key with a new one in a single step. */
function NativeRotateDialog({ item, close, busy, perform }: DialogProps) {
  const { c, manifest } = item;
  return (
    <Modal title={`Rotate ${c.name}?`} onClose={close}>
      <p>
        {manifest?.name} replaces this key with a new one and the old value stops working right
        away. Update anything that uses it with the new value.
      </p>
      <div className="modal-footer">
        <button type="button" className="button outline" onClick={close}>
          Cancel
        </button>
        <button
          className="button"
          disabled={busy}
          onClick={() =>
            void perform(
              () => request<CredentialView>({ kind: 'rotate_now', credential_id: c.id }),
              'Key rotated',
            ).then((r) => {
              if (r) close();
            })
          }
        >
          <RotateCw size={14} strokeWidth={1.75} /> Rotate
        </button>
      </div>
    </Modal>
  );
}

/** Opens the provider's page to make a new key, then takes its value in place of the old one. */
function PasteRotateDialog({ item, close, busy, perform }: DialogProps) {
  const { c } = item;
  const manifest = item.manifest!;
  const fields = inputs(manifest.credential_fields);
  return (
    <Modal title={`Rotate ${c.name}`} onClose={close}>
      <p>
        Create a new key in {manifest.name} and paste it here. GetKeys checks it, then keeps it in
        place of the old one. Delete the old key in {manifest.name} once nothing uses it.
      </p>
      <button
        type="button"
        className="button outline full"
        disabled={busy}
        onClick={() =>
          void perform(() => request({ kind: 'open_token_page', plugin_id: manifest.id }))
        }
      >
        Open {manifest.name} <ArrowUpRight size={14} />
      </button>
      <form
        onSubmit={(e) => {
          e.preventDefault();
          const data = new FormData(e.currentTarget);
          void perform(
            () =>
              request<CredentialView>({
                kind: 'save_key',
                credential_id: c.id,
                plugin_id: manifest.id,
                name: '',
                fields: Object.fromEntries(
                  fields
                    .map((f) => [f.key, String(data.get(f.key) ?? '').trim()])
                    .filter(([, value]) => value !== ''),
                ),
              }),
            'Key rotated',
          ).then((r) => {
            if (r) close();
          });
        }}
      >
        {fields
          .filter((f) => f.secret)
          .map((f, i) => (
            <Field
              key={f.key}
              label={`New ${/^[A-Z]{2}/.test(f.label) ? f.label : f.label.toLowerCase()}`}
            >
              <input
                type="password"
                name={f.key}
                required={f.required}
                placeholder={f.placeholder}
                autoComplete="new-password"
                spellCheck={false}
                autoFocus={i === 0}
              />
            </Field>
          ))}
        <div className="modal-footer">
          <button type="button" className="button outline" onClick={close}>
            Cancel
          </button>
          <button className="button" disabled={busy}>
            Save new key
          </button>
        </div>
      </form>
    </Modal>
  );
}

/** Revokes a key with its provider, or walks the person through deleting it there. */
function DeleteDialog({ item, close, busy, perform }: DialogProps) {
  const { c } = item;
  const manifest = item.manifest;
  const [manual, setManual] = useState(false);
  const revokes =
    !!manifest &&
    !c.provider_id.startsWith('import-') &&
    manifest.operations.includes('revoke_credential');
  if (manual && manifest)
    return (
      <Modal title={`Delete ${c.name} in ${manifest.name}`} onClose={close}>
        <p>
          {manifest.name} doesn’t let apps delete this key, so it’s open in your browser. Delete it
          there, then come back and press Done.
        </p>
        <div className="modal-footer">
          <button
            className="button outline"
            disabled={busy}
            onClick={() =>
              void perform(() => request({ kind: 'open_token_page', plugin_id: manifest.id }))
            }
          >
            Open {manifest.name} <ArrowUpRight size={14} />
          </button>
          <button
            className="button"
            disabled={busy}
            onClick={() =>
              void perform(
                () => request({ kind: 'confirm_deleted', credential_id: c.id }),
                'Key deleted',
              ).then((r) => {
                if (r !== undefined) close();
              })
            }
          >
            <Check size={14} /> Done
          </button>
        </div>
      </Modal>
    );
  return (
    <Modal title={`Delete ${c.name}?`} onClose={close}>
      <p>
        {!manifest || (item.own && !manifest.token_page)
          ? 'GetKeys removes it from this device. Nothing changes anywhere else.'
          : revokes
            ? `GetKeys revokes it in ${manifest.name} and removes it from this device. Anything still using it stops working.`
            : `GetKeys removes it from this device and opens ${manifest.name} so you can delete it there too.`}
      </p>
      <div className="modal-footer">
        <button type="button" className="button outline" onClick={close}>
          Cancel
        </button>
        <button
          className="button destructive"
          disabled={busy}
          onClick={() =>
            void perform(
              () => request<Deletion>({ kind: 'delete', credential_id: c.id }),
              (r) => (r.kind === 'deleted' ? 'Key deleted' : undefined),
            ).then((r) => {
              if (r?.kind === 'manual') setManual(true);
              else if (r) close();
            })
          }
        >
          <Trash2 size={14} strokeWidth={1.75} /> Delete key
        </button>
      </div>
    </Modal>
  );
}

/** Fills in the secret of a key the provider listed without it, or replaces a custom key's values. */
function KeyForm({
  snapshot,
  busy,
  perform,
  credential,
  close,
}: Controller & { credential: CredentialView; close: () => void }) {
  const manifest = snapshot.plugins.find(
    (p) =>
      p.manifest.id ===
      snapshot.connections.find((c) => c.id === credential.connection_id)?.plugin_id,
  )?.manifest;
  if (credential.credential_type === CUSTOM || !manifest)
    return <CustomKeyForm {...{ snapshot, busy, perform, credential, close }} />;
  const fields = inputs(manifest.credential_fields);
  const prefill = (key: string) => credential.fields.find((f) => f.key === key)?.value ?? '';
  return (
    <Modal title={`Fill in ${credential.name}`} onClose={close}>
      <p>GetKeys checks the key with {manifest.name}, then keeps it encrypted on this device.</p>
      <form
        onSubmit={(e) => {
          e.preventDefault();
          const data = new FormData(e.currentTarget);
          void perform(
            () =>
              request<CredentialView>({
                kind: 'import',
                input: {
                  connection_id: credential.connection_id,
                  provider_id: credential.provider_id,
                  resource: credential.resource,
                  name: String(data.get('name')),
                  project: credential.project,
                  environment: credential.environment,
                  fields: Object.fromEntries(
                    fields
                      .map((f) => [f.key, String(data.get(f.key) ?? '').trim()])
                      .filter(([, value]) => value !== ''),
                  ),
                },
              }),
            'Key saved',
          ).then((result) => {
            if (result) close();
          });
        }}
      >
        <Field label="Name">
          <input name="name" defaultValue={credential.name} required autoFocus />
        </Field>
        {fields.map((f) => (
          <Field key={f.key} label={f.required ? f.label : `${f.label} (optional)`}>
            <input
              type={f.secret ? 'password' : 'text'}
              name={f.key}
              required={f.required}
              defaultValue={f.secret ? undefined : prefill(f.key)}
              placeholder={f.placeholder}
              autoComplete="new-password"
              spellCheck={false}
            />
          </Field>
        ))}
        <button className="button full" disabled={busy}>
          Save key
        </button>
      </form>
    </Modal>
  );
}
