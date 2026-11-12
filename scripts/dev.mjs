import { createServer } from 'vite';
import { build } from 'esbuild';
import { spawn } from 'node:child_process';
import electron from 'electron';

for (const entry of ['main', 'preload']) {
  await build({ entryPoints: [`src/main/${entry}.ts`], outfile: `dist/${entry}.cjs`, bundle: true,
    platform: 'node', format: 'cjs', external: ['electron'] });
}
const server = await createServer();
await server.listen();
const env = { ...process.env, XRAYC_DEV_URL: 'http://127.0.0.1:5173' };
delete env.ELECTRON_RUN_AS_NODE;
const app = spawn(electron, ['.'], { stdio: 'inherit', env });
const cleanup = async () => { app.kill(); await server.close(); };
process.on('SIGINT', cleanup);
process.on('SIGTERM', cleanup);
app.on('exit', async (code) => { await server.close(); process.exit(code ?? 0); });
