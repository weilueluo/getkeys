import type { Snapshot } from './types';
export type Controller = {
  snapshot: Snapshot;
  busy: boolean;
  /** Runs a task, refreshes the snapshot, and shows `message` (or the one it derives) on success. */
  perform: <T>(
    task: () => Promise<T>,
    message?: string | ((result: T) => string | undefined),
  ) => Promise<T | undefined>;
};
