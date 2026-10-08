import { spawnSync } from 'node:child_process';
import { existsSync } from 'node:fs';
import { resolve } from 'node:path';

export function run(command, args = [], options = {}) {
  const env = { ...process.env, ...options.env };
  const perl = resolve('.local/tools/perl/perl/bin/perl.exe');
  if (process.platform === 'win32' && !env.PERL && existsSync(perl)) env.PERL = perl;
  const result = spawnSync(command, args, {
    stdio: 'inherit',
    ...options,
    env,
    shell: process.platform === 'win32' && ['npm', 'npx'].includes(command),
  });
  if (result.error) throw result.error;
  if (result.status !== 0) process.exit(result.status ?? 1);
}
