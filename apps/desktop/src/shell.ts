import { invoke, isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';

/** Requests from the tray and `getkeys://` links; the payload is a key or provider id. */
export type ShellEvent = 'tray-refresh' | 'open-key' | 'open-provider';
const events: ShellEvent[] = ['tray-refresh', 'open-key', 'open-provider'];

/** Listens for shell requests, then collects the ones that arrived first. Returns the cleanup. */
export function onShell(handle: (event: ShellEvent, payload: string) => void) {
  if (!isTauri()) return () => {};
  const listening = Promise.all(
    events.map((event) => listen<string>(event, (e) => handle(event, e.payload))),
  );
  listening.then(() => invoke('shell_ready')).catch(() => {});
  return () => {
    listening.then((stops) => stops.forEach((stop) => stop())).catch(() => {});
  };
}

export const hidePalette = () => getCurrentWindow().hide();
