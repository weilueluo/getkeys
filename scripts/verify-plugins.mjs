import { createHash } from 'node:crypto';
import { isDeepStrictEqual } from 'node:util';
import { readFileSync } from 'node:fs';
import { compilePlugins } from './compile-plugins.mjs';

// Fill the serde defaults of ConfigField so a terse source manifest equals the signed one.
const field = (f) => ({ placeholder: '', required: false, secret: true, env: '', ...f });
const normalize = (m) => ({
  ...m,
  fields: m.fields.map(field),
  credential_fields: m.credential_fields.map(field),
  account_fields: (m.account_fields ?? []).map(field),
  token_page: m.token_page ?? '',
  account_credentials: m.account_credentials ?? false,
});
const names = compilePlugins();
const catalog = JSON.parse(readFileSync('registry/dist/index.json', 'utf8'));
for (const name of names) {
  const manifest = normalize(JSON.parse(readFileSync(`plugins/${name}/manifest.json`, 'utf8')));
  const packageInfo = catalog.packages.find((p) => p.manifest.id === manifest.id);
  const bytes = readFileSync(
    `target/wasm32-unknown-unknown/release/getkeys_plugin_${name.replaceAll('-', '_')}.wasm`,
  );
  const digest = createHash('sha256').update(bytes).digest('hex');
  if (
    !packageInfo ||
    packageInfo.sha256 !== digest ||
    !isDeepStrictEqual(normalize(packageInfo.manifest), manifest)
  ) {
    throw new Error(
      `Signed bundle is stale for ${manifest.id}. Rebuild and sign with npm run plugins:build.`,
    );
  }
}
console.log('Signed provider bundles match the source build.');
