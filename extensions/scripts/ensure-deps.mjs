// Installs extension dev dependencies on first use so root scripts work after a fresh clone.
import { spawnSync } from 'node:child_process';
import { existsSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = dirname(dirname(fileURLToPath(import.meta.url)));
if (!existsSync(join(root, 'node_modules', '.package-lock.json'))) {
  const result = spawnSync('npm', ['ci', '--no-audit', '--no-fund'], {
    cwd: root,
    stdio: 'inherit',
    shell: process.platform === 'win32',
  });
  process.exit(result.status ?? 1);
}
