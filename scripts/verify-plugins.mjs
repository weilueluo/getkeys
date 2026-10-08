import { createHash } from 'node:crypto';
import { isDeepStrictEqual } from 'node:util';
import { readFileSync } from 'node:fs';
import { compilePlugins } from './compile-plugins.mjs';

const names = compilePlugins();
const catalog = JSON.parse(readFileSync('registry/dist/index.json', 'utf8'));
for (const name of names) {
  const manifest = JSON.parse(readFileSync(`plugins/${name}/manifest.json`, 'utf8'));
  const packageInfo = catalog.packages.find((p) => p.manifest.id === manifest.id);
  const bytes = readFileSync(
    `target/wasm32-unknown-unknown/release/getkeys_plugin_${name.replaceAll('-', '_')}.wasm`,
  );
  const digest = createHash('sha256').update(bytes).digest('hex');
  if (
    !packageInfo ||
    packageInfo.sha256 !== digest ||
    !isDeepStrictEqual(packageInfo.manifest, manifest)
  ) {
    throw new Error(
      `Signed bundle is stale for ${manifest.id}. Rebuild and sign with npm run plugins:build.`,
    );
  }
}
console.log('Signed provider bundles match the source build.');
