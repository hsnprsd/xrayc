import { spawnSync } from 'node:child_process';
import { copyFile, mkdir, chmod } from 'node:fs/promises';
import path from 'node:path';

const result = spawnSync('cargo', ['build', '--release', '--locked', '--manifest-path', 'native/Cargo.toml'], { stdio: 'inherit' });
if (result.status !== 0) process.exit(result.status ?? 1);
const name = `xrayc-service${process.platform === 'win32' ? '.exe' : ''}`;
await mkdir('resources/bin', { recursive: true });
await copyFile(path.join('native/target/release', name), path.join('resources/bin', name));
if (process.platform !== 'win32') await chmod(path.join('resources/bin', name), 0o755);
