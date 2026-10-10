import { execFile } from 'node:child_process';
import * as vscode from 'vscode';
import { parseKeyList, type KeyInfo } from './lib';

export function cliPath(): string {
  const configured = vscode.workspace.getConfiguration('getkeys').get<string>('cliPath', 'getkeys');
  return configured.trim() || 'getkeys';
}

export function workspaceCwd(): string | undefined {
  return vscode.workspace.workspaceFolders?.[0]?.uri.fsPath;
}

/** Runs the CLI without a shell. Rejects with the execFile error (has `code` and `stderr`). */
export function runCli(args: string[], timeoutMs = 30_000): Promise<string> {
  return new Promise((resolve, reject) => {
    execFile(
      cliPath(),
      args,
      {
        cwd: workspaceCwd(),
        timeout: timeoutMs,
        windowsHide: true,
        maxBuffer: 16 * 1024 * 1024,
        encoding: 'utf8',
      },
      (error, stdout, stderr) => {
        if (error) reject(Object.assign(error, { stderr }));
        else resolve(stdout);
      },
    );
  });
}

export async function listKeys(): Promise<KeyInfo[]> {
  return parseKeyList(await runCli(['ls', '--json']));
}
