import { createHash } from 'node:crypto';
import { isDeepStrictEqual } from 'node:util';
import { readdirSync, readFileSync } from 'node:fs';
import { compilePlugins } from './compile-plugins.mjs';

const compiled = new Set(compilePlugins());
const catalog = JSON.parse(readFileSync('registry/dist/index.json', 'utf8'));
const names = readdirSync('plugins', { withFileTypes: true })
  .filter((p) => p.isDirectory())
  .map((p) => p.name);
const stale = (id) =>
  new Error(`Signed bundle is stale for ${id}. Rebuild and sign with npm run plugins:build.`);
// Serde fills defaults, so compare the source manifest with the signed one field by field.
const same = (source, signed) =>
  Object.entries(source).every(([key, value]) =>
    Array.isArray(value)
      ? Array.isArray(signed[key]) &&
        value.length === signed[key].length &&
        value.every((v, i) =>
          v && typeof v === 'object'
            ? same(v, signed[key][i])
            : isDeepStrictEqual(v, signed[key][i]),
        )
      : value && typeof value === 'object'
        ? same(value, signed[key] ?? {})
        : isDeepStrictEqual(value, signed[key]),
  );
for (const name of names) {
  const manifest = JSON.parse(readFileSync(`plugins/${name}/manifest.json`, 'utf8'));
  const signed = catalog.packages.find((p) => p.manifest.id === manifest.id);
  if (!signed || !same(manifest, signed.manifest)) throw stale(manifest.id);
  // Bundles are built and signed on Windows: the wasm embeds host source paths, so only a Windows
  // build reproduces their bytes. Other platforms still compile them and check the manifests.
  if (compiled.has(name) && process.platform === 'win32') {
    const bytes = readFileSync(
      `target/wasm32-unknown-unknown/release/getkeys_plugin_${name.replaceAll('-', '_')}.wasm`,
    );
    if (createHash('sha256').update(bytes).digest('hex') !== signed.sha256)
      throw stale(manifest.id);
  }
}
if (catalog.packages.length !== names.length)
  throw new Error('The signed catalog lists removed providers.');
console.log(
  process.platform === 'win32'
    ? 'Signed provider bundles match the source build.'
    : 'Signed provider manifests match the source; bundle bytes are verified on Windows.',
);
