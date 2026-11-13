import { mkdtemp, rm, readlink } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { execFileSync } from 'node:child_process';

const binaries = path.resolve(process.env.XRAYC_TEST_BIN_DIR ?? 'resources/bin');
const temp = await mkdtemp(path.join(tmpdir(), 'xrayc-core-test-'));
try {
  execFileSync('cargo', ['run', '--quiet', '--locked', '--manifest-path', 'native/Cargo.toml', '--example', 'runtime-config', '--', temp], { stdio: 'inherit' });
  const service = path.join(binaries, `xrayc-service${process.platform === 'win32' ? '.exe' : ''}`);
  for (const config of ['xray.json', 'socks.json']) {
    execFileSync(service, ['--check-config', path.join(temp, config)], { stdio: 'inherit' });
  }
  if (process.platform === 'linux') {
    const namespaces = Object.fromEntries(await Promise.all(['net', 'mnt', 'user'].map(async (name) => [name, await readlink(`/proc/self/ns/${name}`)])));
    execFileSync('unshare', ['--user', '--map-root-user', '--mount', '--net', 'python3', 'tests/tun-session.py', path.resolve(temp), binaries], {
      stdio: 'inherit', timeout: 90_000, env: { ...process.env, XRAYC_TEST_NAMESPACES: JSON.stringify(namespaces) },
    });
  }
} finally { await rm(temp, { recursive: true, force: true }); }
