import { mkdtemp, rm, readlink } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { execFileSync } from 'node:child_process';

const temp = await mkdtemp(path.join(tmpdir(), 'xrayc-core-test-'));
try {
  execFileSync('cargo', ['run', '--quiet', '--locked', '--manifest-path', 'native/Cargo.toml', '--example', 'runtime-config', '--', temp], { stdio: 'inherit' });
  for (const [name, args] of [
    ['xray', ['run', '-test', '-config', path.join(temp, 'xray.json')]],
  ]) {
    execFileSync(path.resolve(`resources/bin/${name}${process.platform === 'win32' ? '.exe' : ''}`), args, { stdio: 'inherit' });
    console.log(`${name}: generated configuration accepted`);
  }
  if (process.platform === 'linux') {
    const namespaces = Object.fromEntries(await Promise.all(['net', 'mnt', 'user'].map(async (name) => [name, await readlink(`/proc/self/ns/${name}`)])));
    execFileSync('unshare', ['--user', '--map-root-user', '--mount', '--net', 'python3', 'tests/tun-session.py', path.resolve(temp), path.resolve('resources/bin')], {
      stdio: 'inherit', timeout: 90_000, env: { ...process.env, XRAYC_TEST_NAMESPACES: JSON.stringify(namespaces) },
    });
  }
} finally { await rm(temp, { recursive: true, force: true }); }
