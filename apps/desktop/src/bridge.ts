import { invoke, isTauri } from '@tauri-apps/api/core';
import type { Command, EnvLine } from './types';
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
