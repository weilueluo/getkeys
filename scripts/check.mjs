import { run } from './run.mjs';

// `npm run check` runs every stage; CI runs each stage as its own job, in parallel.
let built = false;
/** The desktop crate embeds the built frontend, so it compiles only once that exists. */
function frontend() {
  if (!built) run('npm', ['run', 'build']);
  built = true;
}
const stages = {
  lint() {
    run('cargo', ['fmt', '--all', '--check']);
    run('npm', ['run', 'format:check']);
    run('node', ['scripts/verify-plugins.mjs']);
    run('cargo', ['run', '-p', 'xtask', '--', 'verify']);
    run('cargo', ['run', '-p', 'xtask', '--', 'types']);
  },
  clippy() {
    frontend();
    run('cargo', ['clippy', '--workspace', '--all-targets', '--', '-D', 'warnings']);
  },
  rust() {
    frontend();
    if (process.platform === 'darwin') {
      // The CLI journeys open each vault from two programs, the test and getkeys. macOS asks the
      // user before another unsigned program reads a keychain item, which blocks unattended runs.
      run('cargo', ['test', '--workspace', '--exclude', 'getkeys']);
      run('cargo', ['test', '-p', 'getkeys', '--bins']);
    } else {
      run('cargo', ['test', '--workspace']);
    }
  },
  web() {
    frontend();
    run('npm', ['test']);
    run('npx', ['playwright', 'test', '--project=browser']);
    run('npm', ['audit', '--audit-level=high']);
  },
  native() {
    // Native tests drive the app's WebView2 over CDP, which only Windows offers.
    if (process.platform !== 'win32') return;
    frontend();
    run('cargo', ['build', '-p', 'getkeys-desktop', '-p', 'getkeys']);
    run('npx', ['playwright', 'test', '--project=native']);
  },
};

const asked = process.argv.slice(2);
for (const name of asked) if (!stages[name]) throw new Error(`Unknown check stage: ${name}`);
for (const name of asked.length ? asked : Object.keys(stages)) stages[name]();
