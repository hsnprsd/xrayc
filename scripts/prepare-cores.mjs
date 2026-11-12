import { createHash } from 'node:crypto';
import { mkdtemp, readFile, writeFile, mkdir, rm, readdir, copyFile, chmod } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { execFileSync } from 'node:child_process';

const manifest = JSON.parse(await readFile(new URL('./cores.json', import.meta.url), 'utf8'));
const target = `${process.platform}-${process.arch}`;
const output = path.resolve('resources/bin');
await mkdir(output, { recursive: true });
const temp = await mkdtemp(path.join(tmpdir(), 'xrayc-download-'));
try {
  for (const [name, core] of Object.entries(manifest)) {
    const asset = core.assets[target];
    if (!asset) throw new Error(`Unsupported platform: ${target}`);
    const [filename, checksum] = asset;
    console.log(`Downloading ${name} ${core.version} for ${target}…`);
    const response = await fetch(`https://github.com/${core.repository}/releases/download/${core.version}/${filename}`, { signal: AbortSignal.timeout(180_000) });
    if (!response.ok) throw new Error(`Download failed: ${response.status}`);
    const data = Buffer.from(await response.arrayBuffer());
    if (createHash('sha256').update(data).digest('hex') !== checksum) throw new Error(`SHA-256 mismatch for ${filename}`);
    const archive = path.join(temp, filename);
    await writeFile(archive, data);
    const extracted = path.join(temp, name);
    await mkdir(extracted);
    if (filename.endsWith('.zip')) {
      if (process.platform === 'win32') {
        const quote = (s) => `'${s.replaceAll("'", "''")}'`;
        execFileSync('powershell.exe', ['-NoProfile', '-Command', `Expand-Archive -LiteralPath ${quote(archive)} -DestinationPath ${quote(extracted)}`]);
      } else execFileSync('unzip', ['-q', archive, '-d', extracted]);
    } else execFileSync('tar', ['-xzf', archive, '-C', extracted]);
    for (const file of await readdir(extracted, { recursive: true })) {
      const base = path.basename(file);
      if ([name, `${name}.exe`, 'geoip.dat', 'geosite.dat', 'wintun.dll'].includes(base)) {
        await copyFile(path.join(extracted, file), path.join(output, base));
        if (process.platform !== 'win32' && base === name) await chmod(path.join(output, base), 0o755);
      }
      if (/^(licen[cs]e|copying)/i.test(base)) await copyFile(path.join(extracted, file), path.join(output, `${name}-${base}`));
    }
    console.log(`Verified and installed ${name}.`);
  }
} finally { await rm(temp, { recursive: true, force: true }); }
