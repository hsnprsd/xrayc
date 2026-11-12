import { build as bundle } from 'esbuild';
import { build as viteBuild } from 'vite';

await viteBuild();
for (const entry of ['main', 'preload']) {
  await bundle({ entryPoints: [`src/main/${entry}.ts`], outfile: `dist/${entry}.cjs`,
    bundle: true, platform: 'node', format: 'cjs', target: 'node22', external: ['electron'] });
}
