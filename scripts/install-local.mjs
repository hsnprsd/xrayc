import { access, cp, mkdir, rename, rm, lstat, readlink, symlink, writeFile } from 'node:fs/promises';
import { homedir } from 'node:os';
import path from 'node:path';
import { execFileSync } from 'node:child_process';

if (process.platform !== 'linux') throw new Error('This installer is for Linux. Build on macOS/Windows and install the native package there.');
const home = homedir();
const install = path.join(home, '.local/opt/xrayc');
const source = path.resolve('release/linux-unpacked');
await access(path.join(source, 'xrayc'));
const command = path.join(home, '.local/bin/xrayc');
try {
  const stat = await lstat(command);
  if (!stat.isSymbolicLink() || await readlink(command) !== path.join(install, 'xrayc')) {
    throw new Error(`${command} already exists and is not managed by this installer.`);
  }
} catch (error) { if (error.code !== 'ENOENT') throw error; }
await mkdir(path.dirname(install), { recursive: true });
const staged = `${install}.staged-${process.pid}`;
const previous = `${install}.previous-${process.pid}`;
await cp(source, staged, { recursive: true, preserveTimestamps: true });
let replaced = false;
try {
  try { await rename(install, previous); replaced = true; } catch (error) { if (error.code !== 'ENOENT') throw error; }
  await rename(staged, install);
} catch (error) {
  if (replaced) await rename(previous, install);
  throw error;
} finally { await rm(staged, { recursive: true, force: true }); }
await rm(previous, { recursive: true, force: true });
await mkdir(path.dirname(command), { recursive: true });
await rm(command, { force: true });
await symlink(path.join(install, 'xrayc'), command);
const applications = path.join(home, '.local/share/applications');
const icons = path.join(home, '.local/share/icons/hicolor/256x256/apps');
await mkdir(applications, { recursive: true });
await mkdir(icons, { recursive: true });
await cp('assets/icon.png', path.join(icons, 'xrayc.png'));
// Desktop Exec quoting differs from shell quoting. Escape reserved characters
// and literal percent signs according to the desktop-entry specification.
const quote = (value) => `"${value.replaceAll('\\', '\\\\').replaceAll('"', '\\"').replaceAll('`', '\\`').replaceAll('$', '\\$').replaceAll('%', '%%')}"`;
const desktop = path.join(applications, 'xrayc.desktop');
await writeFile(desktop, `[Desktop Entry]\nType=Application\nName=Xrayc\nComment=Simple, system-wide Xray VPN client\nExec=${quote(path.join(install, 'xrayc'))}\nIcon=xrayc\nTerminal=false\nCategories=Network;\nStartupWMClass=xrayc\nKeywords=VPN;Xray;TUN;Proxy;\n`);
try { execFileSync('update-desktop-database', [applications], { stdio: 'ignore' }); } catch { /* Optional desktop integration. */ }
console.log(`Installed to ${install}\nLauncher: ${command}\nApplication menu: Xrayc`);
