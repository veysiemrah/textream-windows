// The release build: the installer, built without this machine's paths, then scanned.
//
// A binary embeds the absolute paths of what it was compiled from: Rust panic locations, and
// `__FILE__` in the sherpa-onnx library this build compiles with MSVC. On a developer's machine
// those paths hold the account name. wayBetter's `build-env` runs the build with them remapped or
// trimmed, and its `privacy-scan` opens the finished installer and looks for what must not be in
// it. Nothing is published from here: a clean scan is what makes the installer publishable.
//
//   bun run release        (WAYBETTER_CLI: path to waybetter's bin/waybetter.mjs, else `waybetter`)
import { spawnSync } from 'node:child_process';
import { existsSync, readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = dirname(dirname(fileURLToPath(import.meta.url)));
const { version } = JSON.parse(readFileSync(join(root, 'package.json'), 'utf8'));
const installer = join(root, 'target', 'release', 'bundle', 'nsis', `Textream_${version}_x64-setup.exe`);
const waybetter = process.env.WAYBETTER_CLI ? ['node', process.env.WAYBETTER_CLI] : ['waybetter'];

function run(step, [command, ...args]) {
  console.log(`\n== ${step}`);
  const quoted = [command, ...args].map(arg => (/\s/.test(arg) ? `"${arg}"` : arg)).join(' ');
  const { status } = spawnSync(quoted, { cwd: root, stdio: 'inherit', shell: true });
  if (status !== 0) {
    console.error(`\nrelease: "${step}" failed (exit ${status ?? 'none'}). Nothing here is publishable.`);
    process.exit(status || 1);
  }
}

// CMake does not recompile objects when only the environment changed, so a sherpa-onnx left over
// from an ordinary build would keep its paths.
run('clean the release profile', ['cargo', 'clean', '--release']);
run('build the installer, path-free', [...waybetter, 'build-env', '--root', root, '--', 'bun', 'run', 'app:build']);
if (!existsSync(installer)) {
  console.error(`release: the build left no ${installer}`);
  process.exit(1);
}
run('scan the installer', [...waybetter, 'privacy-scan', installer, '--allow', join(root, 'privacy-allowlist.txt')]);
console.log(`\nClean. Publishable: ${installer}`);
