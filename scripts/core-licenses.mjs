import { execFileSync } from 'node:child_process';
import { readFile, readdir, writeFile } from 'node:fs/promises';
import path from 'node:path';

export async function writeCoreLicenses(output, manifest) {
  // Include licenses for the modules actually linked on this target, including
  // nested notices. Go verifies their source archives against go.sum.
  const dependencies = execFileSync('go', ['list', '-mod=readonly', '-deps', '-f', '{{if .Module}}{{.Module.Path}}\t{{.Module.Version}}\t{{.Module.Dir}}{{end}}', '.'], {
    cwd: 'native/xray', encoding: 'utf8',
  });
  const modules = [...new Set(dependencies.split('\n').filter((line) => line.trim()))].sort();
  const notices = [`Embedded Xray-core ${manifest.version}\nSource commit: ${manifest.commit}\nModule: ${manifest.moduleVersion}\n`];
  for (const filename of ['LICENSE', 'PATENTS']) {
    notices.push(`Go ${filename}\n${await readFile(new URL(`../native/xray/licenses/go-${filename}`, import.meta.url), 'utf8')}`);
  }
  for (const module of modules) {
    const [name, version, dir] = module.split('\t');
    if (name === 'xrayc/core') continue; // Covered by the application's MIT license.
    const files = (await readdir(dir, { recursive: true, withFileTypes: true }))
      .filter((entry) => entry.isFile() && /^(licen[cs]e|copying|copyright|notice|patents)(?:$|[.-])/i.test(entry.name))
      .map((entry) => path.join(entry.parentPath, entry.name)).sort();
    if (!files.length) throw new Error(`No license found for linked module ${name} ${version}`);
    for (const file of files) {
      notices.push(`${name} ${version} — ${path.relative(dir, file)}\n${await readFile(file, 'utf8')}`);
    }
    if (name === 'github.com/xtls/xray-core') {
      await writeFile(path.join(output, 'xray-LICENSE'), await readFile(path.join(dir, 'LICENSE')));
    }
  }
  await writeFile(path.join(output, 'xray-dependency-licenses.txt'), notices.join('\n\n' + '='.repeat(72) + '\n\n'));
}
