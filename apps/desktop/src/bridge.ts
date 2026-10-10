import { invoke, isTauri } from '@tauri-apps/api/core';
import type { Command, CredentialView, EnvLine } from './types';
export async function request<T>(command: Command): Promise<T> {
  if (!isTauri())
    throw new Error(
      'Open GetKeys in the desktop app to use your encrypted vault. Run npm run desktop -- dev.',
    );
  return invoke<T>('request', { command });
}
export function copyCredential(credentialId: string, field: string) {
  return invoke<void>('copy_credential', { credentialId, field });
}
/** Saves the chosen keys as a .env file, or copies them. False when the save is cancelled. */
export function exportEnv(lines: EnvLine[], save: boolean) {
  return invoke<boolean>('export_env', { lines, save });
}
export interface Capture {
  pluginId: string;
  connectionId: string | null;
  resource: string;
  name: string;
  group: string | null;
  replaces: string | null;
}
/**
 * Opens the provider's page for making a key and waits until the person copies the new key
 * there; the key is saved without passing through the window. Null when cancelled.
 */
export function captureKey(capture: Capture) {
  return invoke<CredentialView | null>('capture_key', { ...capture });
}
export function cancelCapture() {
  return invoke<void>('cancel_capture');
}
