import { execFileSync } from 'node:child_process';
import { copyFile, mkdir, readFile, rm } from 'node:fs/promises';
import path from 'node:path';
import { coreLibraryName } from './core-library.mjs';
import { writeCoreLicenses } from './core-licenses.mjs';

const manifest = JSON.parse(await readFile(new URL('./cores.json', import.meta.url), 'utf8'));
const module = await readFile('native/xray/go.mod', 'utf8');
if (!module.includes(`github.com/xtls/xray-core ${manifest.xray.moduleVersion}\n`)
    || !manifest.xray.moduleVersion.endsWith(manifest.xray.commit.slice(0, 12))) {
  throw new Error('Xray source pin does not match scripts/cores.json.');
}
const name = coreLibraryName();
const output = path.resolve('native/target/xray', name);
await mkdir(path.dirname(output), { recursive: true });
console.log(`Building embedded Xray ${manifest.xray.version} for ${process.platform}-${process.arch}…`);
execFileSync('go', ['build', '-mod=readonly', '-trimpath', '-buildmode=c-shared', '-ldflags=-s -w', '-o', output, '.'], {
  cwd: 'native/xray', stdio: 'inherit', env: { ...process.env, CGO_ENABLED: '1' },
});
await mkdir('resources/bin', { recursive: true });
await writeCoreLicenses('resources/bin', manifest.xray);
await copyFile(output, path.join('resources/bin', name));
// Old installations must not accidentally keep shipping the standalone core.
await rm(path.join('resources/bin', process.platform === 'win32' ? 'xray.exe' : 'xray'), { force: true });
console.log(`Embedded core: resources/bin/${name}`);
