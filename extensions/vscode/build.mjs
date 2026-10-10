import { build } from 'esbuild';
import { spawnSync } from 'node:child_process';
import { mkdirSync, rmSync, writeFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { renderKeyIcon } from '../scripts/icons.mjs';

const root = dirname(fileURLToPath(import.meta.url));
const dist = join(root, 'dist');

rmSync(dist, { recursive: true, force: true });
mkdirSync(dist, { recursive: true });

await build({
  entryPoints: [join(root, 'src', 'extension.ts')],
  outfile: join(dist, 'extension.js'),
  bundle: true,
  platform: 'node',
  format: 'cjs',
  target: 'node20',
  external: ['vscode'],
  minify: true,
  legalComments: 'none',
  logLevel: 'warning',
});

writeFileSync(join(dist, 'icon.png'), renderKeyIcon(128));

const vsix = join(root, 'getkeys.vsix');
const require = createRequire(import.meta.url);
const vsceBin = join(dirname(require.resolve('@vscode/vsce/package.json')), 'vsce');
const vsce = spawnSync(process.execPath, [vsceBin, 'package', '--no-dependencies', '-o', vsix], {
  cwd: root,
  stdio: 'inherit',
});
if (vsce.status !== 0) process.exit(vsce.status ?? 1);
console.log(`vscode: ${vsix}`);
