import { useCallback, useRef, useState } from 'react';
import { Check, LoaderCircle, X } from 'lucide-react';
import type { AuthProgress, Discovery, Resource, Snapshot } from './types';
import { request } from './bridge';
import { Modal } from './ui';
import { listed } from './secrets';

export type Step = { label: string; state: 'running' | 'done' | 'failed'; detail?: string };
export type Job = { title: string; steps: Step[]; running: boolean; stopping: boolean };
/**
 * What a job covers: one account, a provider's, a list of accounts and keys kept on their own,
 * or all of them.
 */
export type Scope = {
  plugin?: string;
  connection?: string;
  connections?: string[];
  keys?: string[];
};
export type Outcome = {
  snapshot: Snapshot;
  /** Keys that weren't in the vault before. */
  added: number;
  failed: Step[];
  /** Accounts connected from sign-ins found on this device. */
  connected: number;
  cancelled: boolean;
};

/** The group earlier versions put refused keys in; Others has taken its place. */
const OLD_FAILED = 'Failed';

const refusal = (e: unknown) => /\bforbidden:/.test(String(e));

const keys = (s: Snapshot) =>
  new Set(s.credentials.filter((c) => listed(c, s.connections)).map((c) => c.id));

/** Runs `task` over `items`, at most `limit` at a time, until `stopped` says to stop. */
async function pool<T>(
  items: T[],
  limit: number,
  stopped: () => boolean,
  task: (item: T) => Promise<void>,
) {
  let next = 0;
  const worker = async () => {
    while (next < items.length && !stopped()) await task(items[next++]);
  };
  await Promise.all(Array.from({ length: Math.min(limit, items.length) }, worker));
}

/**
 * Runs the slow work behind Add and Refresh with progress and a way to stop: connect the
 * sign-ins on this device, then list every account's keys side by side and check the keys no
 * listing vouches for.
 */
export function useJob(refresh: () => Promise<Snapshot>, setBusy: (busy: boolean) => void) {
  const [job, setJob] = useState<Job>();
  const stop = useRef(false);
  const run = useCallback(
    async (title: string, scope: Scope = {}): Promise<Outcome | undefined> => {
      stop.current = false;
      setBusy(true);
      const steps: Step[] = [];
      const show = (running = true) =>
        setJob({
          title,
          steps: steps.map((x) => ({ ...x })),
          running,
          stopping: running && stop.current,
        });
      /** Runs a step; a `quiet` step that fails leaves the list, and the log keeps why. */
      const step = async <T,>(
        label: string,
        task: (note: (detail: string) => void) => Promise<T>,
        quiet = false,
      ): Promise<T | undefined> => {
        const entry: Step = { label, state: 'running' };
        steps.push(entry);
        const update = (s: Partial<Step>) => {
          Object.assign(entry, s);
          show();
        };
        show();
        try {
          const result = await task((detail) => update({ detail }));
          update({ state: 'done' });
          return result;
        } catch (e) {
          if (quiet) {
            steps.splice(steps.indexOf(entry), 1);
            show();
          } else update({ state: 'failed', detail: String(e) });
          return undefined;
        }
      };
      try {
        let snapshot = await refresh();
        const before = keys(snapshot);
        let connected = 0;
        const stopped = () => stop.current;
        if (!scope.connection && !scope.connections) {
          const sources =
            (await step('Looking for sign-ins and keys on this device', () =>
              request<Discovery[]>({ kind: 'scan_sources', plugin_id: scope.plugin ?? null }),
            )) ?? [];
          await pool(sources, 6, stopped, async (d) => {
            const name = d.account || d.source;
            const progress = await step(
              d.key ? `Saving ${name}` : `Connecting ${name}`,
              () =>
                request<AuthProgress>({
                  kind: 'connect_local',
                  source_id: d.id,
                  connection_id: null,
                }),
              true,
            );
            if (progress?.kind === 'connected') connected++;
          });
          snapshot = await refresh();
        }
        const accounts = snapshot.connections.filter(
          (c) =>
            c.state !== 'disconnected' &&
            (scope.connection
              ? c.id === scope.connection
              : scope.connections
                ? scope.connections.includes(c.id)
                : !scope.plugin || c.plugin_id === scope.plugin),
        );
        const current = snapshot;
        await pool(accounts, 8, stopped, async (c) => {
          const manifest = current.plugins.find((p) => p.manifest.id === c.plugin_id)?.manifest;
          if (!manifest) return;
          await step(`Listing keys in ${manifest.name} · ${c.name}`, async (note) => {
            const lists = manifest.operations.includes('list_credentials');
            // Account-level providers keep their keys on the account; the rest per project.
            let places: string[] = [];
            try {
              places = manifest.account_credentials
                ? ['']
                : lists
                  ? (await request<Resource[]>({ kind: 'resources', connection_id: c.id })).map(
                      (r) => r.id,
                    )
                  : [];
            } catch (e) {
              // An account that may not list its projects has its keys marked as unchecked.
              if (refusal(e)) return;
              throw e;
            }
            const errors: string[] = [];
            let done = 0;
            await pool(places, 4, stopped, async (resource) => {
              await request({ kind: 'sync', connection_id: c.id, resource }).catch((e) =>
                errors.push(String(e)),
              );
              if (places.length > 1) note(`${++done} of ${places.length}`);
            });
            if (places.length && errors.length === places.length) throw errors[0];
            // Keys no listing confirms are asked about one by one.
            const unconfirmed = current.credentials.filter(
              (k) =>
                k.connection_id === c.id &&
                k.credential_type !== 'sign_in' &&
                !k.retired &&
                k.value_available &&
                (!lists || k.provider_id.startsWith('import-')),
            );
            await pool(unconfirmed, 4, stopped, (k) =>
              request({ kind: 'check', credential_id: k.id }).then(
                () => {},
                () => {},
              ),
            );
            note(errors.length ? `${errors.length} of ${places.length} couldn’t be listed` : '');
          });
        });
        // Keys kept on their own, outside any account, are checked with their provider directly.
        const own = scope.connection
          ? []
          : (await refresh()).credentials.filter(
              (k) =>
                k.plugin_id &&
                !k.connection_id &&
                !k.retired &&
                k.value_available &&
                (scope.connections
                  ? !!scope.keys?.includes(k.id)
                  : !scope.plugin || k.plugin_id === scope.plugin),
            );
        if (own.length && !stop.current)
          await step(
            `Checking ${own.length === 1 ? '1 key' : `${own.length} keys`}`,
            async (note) => {
              let done = 0;
              await pool(own, 6, stopped, async (k) => {
                await request({ kind: 'check', credential_id: k.id }).then(
                  () => {},
                  () => {},
                );
                if (own.length > 1) note(`${++done} of ${own.length}`);
              });
              note('');
            },
          );
        snapshot = await refresh();
        const stale = snapshot.credentials.filter((k) => k.groups.includes(OLD_FAILED));
        for (const k of stale)
          await request({
            kind: 'set_groups',
            credential_id: k.id,
            groups: k.groups.filter((g) => g !== OLD_FAILED),
          }).catch(() => {});
        if (stale.length) snapshot = await refresh();
        const added = [...keys(snapshot)].filter((id) => !before.has(id)).length;
        const failed = steps.filter((s) => s.state === 'failed');
        const outcome = { snapshot, added, failed, connected, cancelled: stop.current };
        if (failed.length && !stop.current) show(false);
        else setJob(undefined);
        return outcome;
      } catch (e) {
        steps.push({ label: 'Something went wrong', state: 'failed', detail: String(e) });
        show(false);
        return undefined;
      } finally {
        setBusy(false);
      }
    },
    [refresh, setBusy],
  );
  const cancel = useCallback(() => {
    stop.current = true;
    setJob((j) => (j?.running ? { ...j, stopping: true } : undefined));
  }, []);
  return { job, run, cancel, close: () => setJob(undefined) };
}

const icons = {
  running: <LoaderCircle className="spin" size={15} strokeWidth={2} />,
  done: <Check size={15} strokeWidth={2} />,
  failed: <X size={15} strokeWidth={2} />,
};

export function JobDialog({ job, cancel }: { job: Job; cancel: () => void }) {
  return (
    <Modal title={job.title} onClose={cancel}>
      <ol className="job-steps" aria-live="polite">
        {job.steps.map((s, i) => (
          <li key={i} className={`job-step ${s.state}`}>
            <span className="job-icon" role="img" aria-label={s.state}>
              {icons[s.state]}
            </span>
            <span className="job-text">
              <span>{s.label}</span>
              {s.detail && <small>{s.detail}</small>}
            </span>
          </li>
        ))}
      </ol>
      <div className="modal-footer">
        {job.running ? (
          <button className="button outline" disabled={job.stopping} onClick={cancel}>
            {job.stopping ? 'Stopping…' : 'Cancel'}
          </button>
        ) : (
          <button className="button" onClick={cancel}>
            Close
          </button>
        )}
      </div>
    </Modal>
  );
}
