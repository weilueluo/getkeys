import type { Snapshot } from './types';
export type Controller = {
  snapshot: Snapshot;
  busy: boolean;
  perform: <T>(task: () => Promise<T>, message?: string) => Promise<T | undefined>;
};
