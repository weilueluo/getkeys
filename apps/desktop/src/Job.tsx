import { useCallback, useRef, useState } from 'react';
import { Check, LoaderCircle, X } from 'lucide-react';
import type { AuthProgress, Discovery, Resource, Snapshot } from './types';
import { request } from './bridge';
import { Modal } from './ui';
import { FAILED, listed } from './secrets';

export type Step = { label: string; state: 'running' | 'done' | 'failed'; detail?: string };
export type Job = { title: string; steps: Step[]; running: boolean; stopping: boolean };
/** Which accounts a job covers: one account, a provider's, a list of them, or all of them. */
export type Scope = { plugin?: string; connection?: string; connections?: string[] };
export type Outcome = {
  snapshot: Snapshot;
  /** Keys that weren't in the vault before. */
  added: number;
  failed: Step[];
  /** Accounts connected from sign-ins found on this device. */
  connected: number;
  /** Keys newly put in the Failed group. */
  refused: number;
  cancelled: boolean;
};

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
        setJob({ title, steps: [...steps], running, stopping: running && stop.current });
      const step = async <T,>(
        label: string,
        task: (note: (detail: string) => void) => Promise<T>,
      ): Promise<T | undefined> => {
        const i = steps.push({ label, state: 'running' }) - 1;
        const update = (s: Partial<Step>) => {
          steps[i] = { ...steps[i], ...s };
          show();
        };
        show();
        try {
          const result = await task((detail) => update({ detail }));
          update({ state: 'done' });
          return result;
        } catch (e) {
          update({ state: 'failed', detail: String(e) });
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
            (await step('Looking for sign-ins on this device', () =>
              request<Discovery[]>({ kind: 'scan_sources', plugin_id: scope.plugin ?? null }),
            )) ?? [];
          await pool(sources, 6, stopped, async (d) => {
            const progress = await step(`Connecting ${d.account || d.source}`, () =>
              request<AuthProgress>({
                kind: 'connect_local',
                source_id: d.id,
                connection_id: null,
              }),
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
        // Keys behind a listing the provider refused, and the accounts whose listings went through.
        const refusedKeys = new Set<string>();
        const listedAccounts = new Set<string>();
        await pool(accounts, 8, stopped, async (c) => {
          const manifest = current.plugins.find((p) => p.manifest.id === c.plugin_id)?.manifest;
          if (!manifest) return;
          await step(`Listing keys in ${manifest.name} · ${c.name}`, async (note) => {
            const lists = manifest.operations.includes('list_credentials');
            // Account-level providers keep their keys on the account; the rest per project.
            const mine = current.credentials.filter(
              (k) => k.connection_id === c.id && k.credential_type !== 'sign_in' && !k.retired,
            );
            const refuse = (resource?: string) =>
              mine
                .filter((k) => resource === undefined || k.resource === resource)
                .forEach((k) => refusedKeys.add(k.id));
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
              if (!refusal(e)) throw e;
              refuse();
              listedAccounts.add(c.id);
              note('No permission to list these keys. They’re in Failed.');
              return;
            }
            const errors: string[] = [];
            let refused = 0;
            let done = 0;
            await pool(places, 4, stopped, async (resource) => {
              await request({ kind: 'sync', connection_id: c.id, resource }).catch((e) => {
                if (!refusal(e)) return errors.push(String(e));
                refused++;
                refuse(manifest.account_credentials ? undefined : resource);
              });
              if (places.length > 1) note(`${++done} of ${places.length}`);
            });
            if (places.length && errors.length && errors.length + refused === places.length)
              throw errors[0];
            listedAccounts.add(c.id);
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
              request({ kind: 'check', credential_id: k.id })
                .then(() => {})
                .catch((e) => {
                  if (refusal(e)) refusedKeys.add(k.id);
                }),
            );
            note(
              errors.length
                ? `${errors.length} of ${places.length} couldn’t be listed: ${errors[0]}`
                : refused
                  ? `No permission to list ${refused} of ${places.length}. Their keys are in Failed.`
                  : '',
            );
          });
        });
        snapshot = await refresh();
        // Keys the provider refused or rejected go in Failed; keys that came through leave it.
        let refused = 0;
        let moved = false;
        for (const k of stop.current ? [] : snapshot.credentials) {
          if (!listedAccounts.has(k.connection_id) || k.credential_type === 'sign_in') continue;
          const failing = refusedKeys.has(k.id) || !!k.problem;
          if (failing === k.groups.includes(FAILED)) continue;
          if (failing) refused++;
          const groups = failing ? [...k.groups, FAILED] : k.groups.filter((g) => g !== FAILED);
          await request({ kind: 'set_groups', credential_id: k.id, groups }).catch(() => {});
          moved = true;
        }
        if (moved) snapshot = await refresh();
        const added = [...keys(snapshot)].filter((id) => !before.has(id)).length;
        const failed = steps.filter((s) => s.state === 'failed');
        const outcome = { snapshot, added, failed, connected, refused, cancelled: stop.current };
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
