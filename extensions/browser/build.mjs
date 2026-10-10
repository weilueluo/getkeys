import { build } from 'esbuild';
import { copyFileSync, mkdirSync, rmSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { writeIcons } from '../scripts/icons.mjs';
import { zipDirectory } from '../scripts/zip.mjs';

const root = dirname(fileURLToPath(import.meta.url));
const dist = join(root, 'dist');

rmSync(dist, { recursive: true, force: true });
mkdirSync(dist, { recursive: true });

await build({
  entryPoints: ['background', 'content', 'popup'].map((name) => join(root, 'src', `${name}.ts`)),
  outdir: dist,
  bundle: true,
  format: 'iife',
  target: 'chrome121',
  minify: true,
  legalComments: 'none',
  loader: { '.css': 'text' },
  logLevel: 'warning',
});

copyFileSync(join(root, 'manifest.json'), join(dist, 'manifest.json'));
copyFileSync(join(root, 'src', 'popup.html'), join(dist, 'popup.html'));
copyFileSync(join(root, 'src', 'popup.css'), join(dist, 'popup.css'));
writeIcons(join(dist, 'icons'), [16, 32, 48, 128]);

const zip = join(root, 'getkeys-browser.zip');
zipDirectory(dist, zip);
console.log(`browser: ${dist}\nbrowser: ${zip}`);
