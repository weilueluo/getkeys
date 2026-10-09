import { useEffect, useId, useState, type ReactNode } from 'react';
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
  Lock,
  LockOpen,
  PenLine,
  Plus,
  RefreshCw,
  RotateCw,
  SquareTerminal,
  Trash2,
  X,
} from 'lucide-react';
import type {
  Binding,
  ConnectionView,
  CredentialView,
  Deletion,
  KeyField,
  Manifest,
  Resource,
  Rotation,
} from './types';
import type { Controller } from './controller';
import { copyCredential, exportEnv, request } from './bridge';
import {
  Checkbox,
  Empty,
  Field,
  inputs,
  Modal,
  PageHeader,
  ProviderMark,
  sentence,
  Tabs,
} from './ui';
import { formatDay, isFinished, RotationProgress, rotationsFor } from './RotationProgress';
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
const resourceLabel = (kind?: string) =>
  kind ? sentence(kind).replace(/^Iam\b/, 'IAM') : 'Resource';
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
/** Which keys Refresh covers. */
export type RefreshScope = { connections?: string[]; label?: string };

type Props = Controller & {
  search: string;
  searchField: ReactNode;
  browse: () => void;
  refreshKeys: (scope?: RefreshScope) => void;
};
export function VaultPanel({
  snapshot,
  busy,
  perform,
  search,
  searchField,
  browse,
  refreshKeys,
}: Props) {
  const [view, setView] = useState<View>('ready');
  const [created, setCreated] = useState(storedGroups);
  const [naming, setNaming] = useState(false);
  const [picking, setPicking] = useState<string>();
  const [open, setOpen] = useState('');
  const [form, setForm] = useState<{ credential?: CredentialView }>();
  const [exporting, setExporting] = useState(false);
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
          },
        ];
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
  const items = searched.filter((i) => inView(i, shown));
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
        label: activeGroup,
      }
    : {};
  const canRefresh = scope.connections ? scope.connections.length > 0 : active.length > 0;
  const newKey = (
    <button className="button" disabled={busy} onClick={() => setForm({})}>
      <Plus size={14} /> New key
    </button>
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
              <button className="button outline" disabled={busy} onClick={() => setForm({})}>
                <Plus size={14} /> Add a key by hand
              </button>
            </div>
          }
        >
          Add GitHub, AWS, Vercel, or Supabase. GetKeys brings in the keys already on this device
          and in your accounts.
        </Empty>
        {form && <KeyForm {...{ snapshot, busy, perform }} close={() => setForm(undefined)} />}
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
        {activeGroup && (
          <div className="group-actions">
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
          </div>
        )}
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
              edit={() => setForm({ credential: i.c })}
              refreshKeys={refreshKeys}
              groups={groups}
            />
          ))}
        </ul>
      ) : (
        <Empty title={search ? 'No matching keys' : 'No keys yet'}>
          {search
            ? 'Try a different name, provider, or project.'
            : activeGroup
              ? `Nothing in ${activeGroup} yet. Choose keys to add them.`
              : 'Refresh to look for keys again, or add one with New key.'}
        </Empty>
      )}
      {form && (
        <KeyForm
          {...{ snapshot, busy, perform }}
          credential={form.credential}
          close={() => setForm(undefined)}
        />
      )}
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
}) {
  const { c, connection, manifest, provider, place, secretless, local, custom } = item;
  const { busy, perform, snapshot } = control;
  const [secret, reveal] = useReveal(c.id, perform);
  const [dialog, setDialog] = useState<'rotate' | 'guided' | 'delete'>();
  const primary = primaryField(c);
  const value = primary && shown(primary, secret);
  const hidden = c.fields.some((f) => f.secret && f.available);
  const rotation = rotationsFor(snapshot, c.id).find((r) => !isFinished(r));
  const guided = !!manifest?.operations.includes('guide');
  const rotate = rotation
    ? expand
    : custom
      ? edit
      : c.can_rotate || local
        ? () => setDialog('rotate')
        : guided
          ? () => setDialog('guided')
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
      {open && <KeyDetails {...control} item={item} secret={secret} copy={copy} groups={groups} />}
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
  ...control
}: Controller & {
  item: Item;
  secret?: Record<string, string>;
  copy: (f: KeyField) => void;
  groups: string[];
}) {
  const { c: credential, connection, custom } = item;
  const { busy, perform, snapshot } = control;
  const [binding, setBinding] = useState(false);
  const bindings = snapshot.bindings.filter((b) => b.credential_id === credential.id);
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
          <dt>{custom ? 'Provider' : 'Account'}</dt>
          <dd>{connection?.name ?? 'None, added by hand'}</dd>
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
        <GroupEditor {...control} credential={credential} groups={groups} />
        {!custom && (
          <div className="detail-section">
            <div className="detail-section-heading">
              <h3>
                Used by <span className="quiet">{bindings.length || ''}</span>
              </h3>
              <button
                className="icon-button"
                title={credential.value_available ? 'Add an app' : 'Fill in the secret first'}
                aria-label="Add consumer"
                disabled={!credential.value_available}
                onClick={() => setBinding(true)}
              >
                <Plus size={15} />
              </button>
            </div>
            {bindings.length ? (
              bindings.map((b) => (
                <div className="binding-item" key={b.id}>
                  <div>
                    <strong className="mono">{b.variable}</strong>
                    <small>
                      {b.project} · {b.environment}
                      {b.destination.kind === 'local' ? ' · local' : ''}
                    </small>
                  </div>
                  <button
                    className="icon-button"
                    disabled={busy}
                    aria-label={`Remove ${b.variable}`}
                    onClick={() =>
                      void perform(
                        () => request({ kind: 'remove_binding', binding_id: b.id }),
                        'Consumer removed',
                      )
                    }
                  >
                    <X size={13} />
                  </button>
                </div>
              ))
            ) : (
              <p className="quiet">
                Add the apps that read this key so a rotation can update them.
              </p>
            )}
          </div>
        )}
      </div>
      {binding && (
        <BindingForm {...control} credential={credential} close={() => setBinding(false)} />
      )}
    </div>
  );
}

/** Files a key under groups, which scope Refresh and Export. */
function GroupEditor({
  credential,
  groups,
  busy,
  perform,
}: Controller & { credential: CredentialView; groups: string[] }) {
  const [draft, setDraft] = useState('');
  const listId = useId();
  const save = (next: string[]) =>
    void perform(() =>
      request<CredentialView>({ kind: 'set_groups', credential_id: credential.id, groups: next }),
    );
  const add = () => {
    const name = draft.trim();
    if (!name) return;
    setDraft('');
    save([...credential.groups, name]);
  };
  return (
    <div className="detail-section">
      <div className="detail-section-heading">
        <h3>Groups</h3>
      </div>
      <div className="group-editor">
        {credential.groups.map((g) => (
          <span className="chip" key={g}>
            {g}
            <button
              type="button"
              aria-label={`Remove from ${g}`}
              disabled={busy}
              onClick={() => save(credential.groups.filter((x) => x !== g))}
            >
              <X size={11} strokeWidth={2.25} />
            </button>
          </span>
        ))}
        <input
          className="chip-input"
          aria-label={`Add ${credential.name} to a group`}
          placeholder={credential.groups.length ? 'Add…' : 'Add to a group'}
          list={listId}
          value={draft}
          maxLength={40}
          disabled={busy}
          onChange={(e) => setDraft(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter') {
              e.preventDefault();
              add();
            }
          }}
          onBlur={add}
        />
        <datalist id={listId}>
          {groups
            .filter((g) => !credential.groups.includes(g))
            .map((g) => (
              <option key={g} value={g} />
            ))}
        </datalist>
      </div>
    </div>
  );
}

type DialogProps = Controller & { item: Item; close: () => void; done?: () => void };

function RotateDialog({ item, close, done, snapshot, busy, perform }: DialogProps) {
  const { c } = item;
  const bindings = snapshot.bindings.filter((b) => b.credential_id === c.id).length;
  return (
    <Modal title={`Rotate ${c.name}`} onClose={close}>
      <p>
        GetKeys creates a replacement, updates {plural(bindings, 'registered app')}, and waits for
        your go-ahead before retiring this key.
      </p>
      {bindings === 0 && (
        <p className="callout">
          No apps are registered for this key. Update anything that uses it yourself before retiring
          it.
        </p>
      )}
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
        Create a replacement in {manifest.name} with the access your apps need, add it with New key,
        then choose it here so GetKeys can update your apps.
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

/** Revokes a key with its provider, or walks the person through deleting it there. */
function DeleteDialog({ item, close, snapshot, busy, perform }: DialogProps) {
  const { c } = item;
  const manifest = item.manifest;
  const [manual, setManual] = useState(false);
  const revokes =
    !!manifest &&
    !c.provider_id.startsWith('import-') &&
    manifest.operations.includes('revoke_credential');
  const bindings = snapshot.bindings.filter((b) => b.credential_id === c.id).length;
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
        {!manifest
          ? 'GetKeys removes it from this device. You added it by hand, so nothing changes anywhere else.'
          : revokes
            ? `GetKeys revokes it in ${manifest.name} and removes it from this device. Anything still using it stops working.`
            : `GetKeys removes it from this device and opens ${manifest.name} so you can delete it there too.`}
      </p>
      {bindings > 0 && (
        <p className="callout">
          {plural(bindings, 'registered app')} use this key and will lose access.
        </p>
      )}
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

/** Adds a key by hand, or fills in the secret of a key the provider listed without it. */
function KeyForm({
  snapshot,
  busy,
  perform,
  credential,
  close,
}: Controller & { credential?: CredentialView; close: () => void }) {
  const manifestOf = (id: string) =>
    snapshot.plugins.find(
      (p) => p.manifest.id === snapshot.connections.find((c) => c.id === id)?.plugin_id,
    )?.manifest;
  const accounts = snapshot.connections.filter((c) => {
    const m = manifestOf(c.id);
    return c.state !== 'disconnected' && m && inputs(m.credential_fields).length > 0;
  });
  const [account, setAccount] = useState(
    credential ? credential.connection_id || CUSTOM : (accounts[0]?.id ?? CUSTOM),
  );
  const [resources, setResources] = useState<Resource[] | undefined>(credential && []);
  const [resource, setResource] = useState(credential?.resource ?? '');
  const manifest = manifestOf(account);
  const accountToken = !!manifest?.account_credentials;
  useEffect(() => {
    if (credential || !account || account === CUSTOM) return;
    let current = true;
    setResources(undefined);
    void perform(() => request<Resource[]>({ kind: 'resources', connection_id: account })).then(
      (r) => {
        if (!current) return;
        const all = [
          ...(accountToken ? [{ id: '', name: 'Account token', kind: '' }] : []),
          ...(r ?? []),
        ];
        setResources(all);
        setResource(all[0]?.id ?? '');
      },
    );
    return () => {
      current = false;
    };
  }, [account, accountToken, credential, perform]);
  const prefill = (key: string) => credential?.fields.find((f) => f.key === key)?.value ?? '';
  const providerSelect = !credential && (
    <Field label="Provider">
      <select value={account} onChange={(e) => setAccount(e.target.value)} required>
        {accounts.map((c) => (
          <option key={c.id} value={c.id}>
            {manifestOf(c.id)?.name} · {c.name}
          </option>
        ))}
        <option value={CUSTOM}>None · any other service</option>
      </select>
    </Field>
  );
  if (account === CUSTOM)
    return (
      <CustomKeyForm {...{ snapshot, busy, perform, credential, close }} header={providerSelect} />
    );
  return (
    <Modal title={credential ? `Fill in ${credential.name}` : 'New key'} onClose={close}>
      <p>
        {manifest
          ? `GetKeys checks the key with ${manifest.name}, then keeps it encrypted on this device.`
          : 'Add a provider first.'}
      </p>
      <form
        onSubmit={(e) => {
          e.preventDefault();
          if (!manifest) return;
          const data = new FormData(e.currentTarget);
          void perform(
            () =>
              request<CredentialView>({
                kind: 'import',
                input: {
                  connection_id: account,
                  provider_id: credential?.provider_id ?? null,
                  resource,
                  name: String(data.get('name')),
                  project: credential?.project ?? '',
                  environment: credential?.environment ?? '',
                  fields: Object.fromEntries(
                    inputs(manifest.credential_fields)
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
        {!credential && (
          <>
            {providerSelect}
            {!!resources?.length && (
              <Field label={accountToken ? 'Belongs to' : resourceLabel(resources[0].kind)}>
                <select value={resource} onChange={(e) => setResource(e.target.value)} required>
                  {resources.map((r) => (
                    <option key={r.id} value={r.id}>
                      {r.name}
                    </option>
                  ))}
                </select>
              </Field>
            )}
          </>
        )}
        <Field label="Name">
          <input
            name="name"
            defaultValue={credential?.name ?? ''}
            placeholder="Production API key"
            required
            autoFocus
          />
        </Field>
        {manifest &&
          inputs(manifest.credential_fields).map((f) => (
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
        <button className="button full" disabled={busy || !manifest || !resources}>
          Save key
        </button>
      </form>
    </Modal>
  );
}

type Row = { variable: string; value: string; secret: boolean };
const variableName = (s: string) => s.toUpperCase().replace(/[^A-Z0-9_]/g, '_');

/** A key from any service, saved as named values. Editing keeps secrets left blank. */
function CustomKeyForm({
  credential,
  header,
  close,
  busy,
  perform,
}: Controller & { credential?: CredentialView; header: ReactNode; close: () => void }) {
  const [rows, setRows] = useState<Row[]>(
    credential?.fields.map((f) => ({
      variable: f.key,
      value: f.secret ? '' : (f.value ?? ''),
      secret: f.secret,
    })) ?? [{ variable: '', value: '', secret: true }],
  );
  const kept = (r: Row) =>
    r.secret && !!credential?.fields.some((f) => f.secret && f.key === r.variable);
  const update = (i: number, patch: Partial<Row>) =>
    setRows((all) => all.map((r, j) => (i === j ? { ...r, ...patch } : r)));
  /** Pasting `NAME=value` lines fills in a row for each. */
  const paste = (i: number, text: string) => {
    const parsed = text
      .split(/\r?\n/)
      .map((l) => /^\s*(?:export\s+)?([A-Za-z_][A-Za-z0-9_]*)\s*=\s*(.*?)\s*$/.exec(l))
      .flatMap((m) =>
        m
          ? [
              {
                variable: m[1].toUpperCase(),
                value: m[2].replace(/^(['"])(.*)\1$/, '$2'),
                secret: true,
              },
            ]
          : [],
      );
    if (!parsed.length) return false;
    setRows((all) => [...all.slice(0, i), ...parsed, ...all.slice(i + 1)]);
    return true;
  };
  return (
    <Modal title={credential ? `Replace ${credential.name}` : 'New key'} onClose={close}>
      <p>
        {credential
          ? 'Paste the new values. Secret values you leave blank stay as they are.'
          : 'Any key from any service. Each value is exported under its variable name. Paste NAME=value lines to fill in several at once.'}
      </p>
      <form
        onSubmit={(e) => {
          e.preventDefault();
          const name = String(new FormData(e.currentTarget).get('name'));
          void perform(
            () =>
              request<CredentialView>({
                kind: 'save_custom',
                credential_id: credential?.id ?? null,
                name,
                fields: rows,
              }),
            'Key saved',
          ).then((result) => {
            if (result) close();
          });
        }}
      >
        {header}
        <Field label="Name">
          <input
            name="name"
            defaultValue={credential?.name ?? ''}
            placeholder="OpenAI"
            required
            autoFocus
          />
        </Field>
        <div className="field">
          <span>Values</span>
          <div className="value-rows">
            {rows.map((r, i) => (
              <div className="value-row" key={i}>
                <input
                  aria-label={`Variable ${i + 1}`}
                  className="mono"
                  placeholder="API_KEY"
                  value={r.variable}
                  required
                  spellCheck={false}
                  onChange={(e) => update(i, { variable: variableName(e.target.value) })}
                  onPaste={(e) => {
                    if (paste(i, e.clipboardData.getData('text'))) e.preventDefault();
                  }}
                />
                <input
                  aria-label={`Value of ${r.variable || `variable ${i + 1}`}`}
                  type={r.secret ? 'password' : 'text'}
                  placeholder={kept(r) ? 'Unchanged' : 'Value'}
                  value={r.value}
                  required={!kept(r)}
                  autoComplete="new-password"
                  spellCheck={false}
                  onChange={(e) => update(i, { value: e.target.value })}
                  onPaste={(e) => {
                    if (paste(i, e.clipboardData.getData('text'))) e.preventDefault();
                  }}
                />
                <button
                  type="button"
                  className="icon-button outline"
                  aria-pressed={r.secret}
                  aria-label={`${r.variable || 'Value'} is secret`}
                  title={r.secret ? 'Secret: hidden until revealed' : 'Not secret: always shown'}
                  onClick={() => update(i, { secret: !r.secret })}
                >
                  {r.secret ? <Lock size={14} /> : <LockOpen size={14} />}
                </button>
                <button
                  type="button"
                  className="icon-button"
                  aria-label={`Remove ${r.variable || `variable ${i + 1}`}`}
                  disabled={rows.length === 1}
                  onClick={() => setRows((all) => all.filter((_, j) => j !== i))}
                >
                  <X size={14} />
                </button>
              </div>
            ))}
          </div>
          <button
            type="button"
            className="button ghost small add-row"
            onClick={() => setRows((all) => [...all, { variable: '', value: '', secret: true }])}
          >
            <Plus size={13} /> Add value
          </button>
        </div>
        <button className="button full" disabled={busy}>
          Save key
        </button>
      </form>
    </Modal>
  );
}

function BindingForm({
  credential,
  close,
  snapshot,
  busy,
  perform,
}: Controller & { credential: CredentialView; close: () => void }) {
  const fields = credential.fields.filter((f) => f.available);
  const [field, setField] = useState(fields[0]?.key ?? '');
  const [variable, setVariable] = useState(fields[0]?.env ?? '');
  const [destination, setDestination] = useState('local');
  const destinations = snapshot.connections.filter(
    (c) =>
      c.state === 'connected' &&
      snapshot.plugins
        .find((p) => p.manifest.id === c.plugin_id)
        ?.manifest.operations.includes('write_secret'),
  );
  return (
    <Modal title="Add a consumer" onClose={close}>
      <p>Map a credential field to the environment variable your app expects.</p>
      <form
        onSubmit={(e) => {
          e.preventDefault();
          const d = new FormData(e.currentTarget);
          const value: Binding = {
            id: '',
            credential_id: credential.id,
            field: String(d.get('field')),
            variable: String(d.get('variable')),
            project: String(d.get('project')),
            environment: String(d.get('environment')),
            destination:
              destination === 'local'
                ? { kind: 'local' }
                : {
                    kind: 'provider',
                    connection_id: destination,
                    resource: String(d.get('resource')),
                  },
          };
          void perform(
            () => request<Binding>({ kind: 'save_binding', binding: value }),
            'Consumer added',
          ).then((b) => {
            if (b) close();
          });
        }}
      >
        <Field label="Credential field">
          <select
            name="field"
            value={field}
            onChange={(e) => {
              const next = fields.find((f) => f.key === e.target.value);
              setField(e.target.value);
              if (next?.env) setVariable(next.env);
            }}
          >
            {fields.map((f) => (
              <option key={f.key} value={f.key}>
                {f.label}
              </option>
            ))}
          </select>
        </Field>
        <Field label="Environment variable">
          <input
            name="variable"
            value={variable}
            onChange={(e) => setVariable(e.target.value)}
            placeholder="API_TOKEN"
            pattern="[A-Za-z_][A-Za-z0-9_]*"
            required
          />
        </Field>
        <div className="form-row">
          <Field label="Project name">
            <input name="project" placeholder="my-app" defaultValue={credential.project} required />
          </Field>
          <Field label="Environment">
            <input name="environment" defaultValue={credential.environment} required />
          </Field>
        </div>
        <Field label="Destination">
          <select value={destination} onChange={(e) => setDestination(e.target.value)}>
            <option value="local">Local process · getkeys run</option>
            {destinations.map((c) => (
              <option value={c.id} key={c.id}>
                {c.name} ·{' '}
                {snapshot.plugins.find((p) => p.manifest.id === c.plugin_id)?.manifest.name}
              </option>
            ))}
          </select>
        </Field>
        {destination !== 'local' && (
          <Field label="Destination resource ID">
            <input name="resource" placeholder="Project ID or owner/repository" required />
          </Field>
        )}
        <p className="small-text quiet">
          Saving a consumer records the mapping. Remote values are delivered during rotation. GitHub
          destinations use repository Actions secrets; Vercel needs a deployment after delivery.
        </p>
        <button className="button full" disabled={busy}>
          <Check size={15} /> Save consumer
        </button>
      </form>
    </Modal>
  );
}
