import { test, expect, _electron as electron, type ElectronApplication, type Page } from '@playwright/test';
import { mkdtemp, rm } from 'node:fs/promises';
import { createServer, createConnection } from 'node:net';
import { tmpdir } from 'node:os';
import path from 'node:path';

let app: ElectronApplication;
let page: Page;
let profile: string;
const inbounds = [{ protocol: 'socks', listen: '127.0.0.1', port: 1080, settings: { udp: true } }];
const config = JSON.stringify({ inbounds, outbounds: [{ protocol: 'vless', settings: { vnext: [{ address: 'vpn.example', port: 443, users: [{ id: '00000000-0000-4000-8000-000000000001', encryption: 'none' }] }] } }] }, null, 2);

test.beforeEach(async () => {
  profile = await mkdtemp(path.join(tmpdir(), 'xrayc-desktop-test-'));
  const env = Object.fromEntries(Object.entries(process.env).filter((entry): entry is [string, string] => entry[1] !== undefined));
  delete env.ELECTRON_RUN_AS_NODE;
  delete env.XRAYC_DEV_URL;
  const executablePath = process.env.XRAYC_TEST_BINARY;
  app = await electron.launch({ executablePath, args: [...(executablePath ? [] : ['.']), `--user-data-dir=${profile}`], env });
  page = await app.firstWindow();
  await expect(page.locator('#configuration')).toBeVisible();
});
test.afterEach(async () => {
  await app?.close();
  if (profile) await rm(profile, { recursive: true, force: true });
});

test('validates pasted JSON and shows the actual inbound and outbound', async () => {
  await expect(page.getByRole('button', { name: 'Connect Xray', exact: true })).toBeDisabled();
  await page.getByRole('textbox', { name: 'Xray JSON configuration' }).fill(config);
  await expect(page.locator('#validation')).toHaveClass('validation valid');
  await expect(page.locator('#protocols')).toHaveText('socks → vless');
  await expect(page.getByRole('button', { name: 'Connect Xray', exact: true })).toBeEnabled();
  await page.locator('#configuration').fill('');
  await expect(page.getByRole('button', { name: 'Connect Xray', exact: true })).toBeDisabled();
  await expect(page.locator('#validation-text')).toHaveText('Paste a configuration to begin.');
});

test('requires configured inbounds and permits explicit direct outbounds', async () => {
  await page.locator('#configuration').fill('{oops');
  await expect(page.locator('#validation-text')).toContainText('Invalid JSON');
  await page.locator('#configuration').fill('{"outbounds":[{"protocol":"freedom"}]}');
  await expect(page.locator('#validation-text')).toContainText('“inbounds” array');
  await expect(page.getByRole('button', { name: 'Connect Xray', exact: true })).toBeDisabled();
  await page.locator('#configuration').fill(JSON.stringify({ inbounds, outbounds: [{ protocol: 'freedom' }] }));
  await expect(page.locator('#protocols')).toHaveText('socks → freedom');
  await expect(page.getByRole('button', { name: 'Connect Xray', exact: true })).toBeEnabled();
});

test('shows opt-in native TUN and its authorization requirement', async () => {
  await page.locator('#configuration').fill(JSON.stringify({ inbounds: [{ protocol: 'tun', settings: { name: 'user-tun' } }], outbounds: [{ protocol: 'freedom' }] }));
  await expect(page.locator('#protocols')).toHaveText('tun → freedom');
  await expect(page.locator('#permission-note')).toContainText('Admin permission required');
});

test('connects and disconnects with accessible status and locked configuration', async () => {
  // Simulate only OS authorization/network activation. The separate namespace
  // suite tests native TUN without touching the host's routes.
  await app.evaluate(({ ipcMain, BrowserWindow }) => {
    ipcMain.removeHandler('vpn:connect');
    ipcMain.handle('vpn:connect', () => {
      const window = BrowserWindow.getAllWindows()[0];
      window.webContents.send('vpn:status', { phase: 'connecting', message: 'Starting Xray…' });
      setTimeout(() => window.webContents.send('vpn:status', { phase: 'connected', message: 'Xray is running.', connectedAt: Date.now() }), 250);
      return { ok: true };
    });
    ipcMain.removeHandler('vpn:disconnect');
    ipcMain.handle('vpn:disconnect', () => {
      const window = BrowserWindow.getAllWindows()[0];
      window.webContents.send('vpn:status', { phase: 'disconnecting', message: 'Stopping Xray…' });
      setTimeout(() => window.webContents.send('vpn:status', { phase: 'disconnected', message: 'Xray is stopped.' }), 250);
      return { ok: true };
    });
  });
  await page.locator('#configuration').fill(config);
  await expect(page.locator('#validation')).toHaveClass('validation valid');
  await page.getByRole('button', { name: 'Connect Xray', exact: true }).click();
  await expect(page.locator('#status-label')).toHaveText('Connected');
  await expect(page.locator('#configuration')).toBeDisabled();
  await expect(page.locator('#duration')).toHaveText(/\d{2}:\d{2}:\d{2}/);
  await page.getByRole('button', { name: 'Disconnect Xray', exact: true }).click();
  await expect(page.locator('#status-label')).toHaveText('Disconnected');
  await expect(page.locator('#configuration')).toBeEnabled();
});

test('runs a real user-configured SOCKS inbound without administrator access', async () => {
  const reservation = createServer();
  await new Promise<void>((resolve) => reservation.listen(0, '127.0.0.1', resolve));
  const port = (reservation.address() as { port: number }).port;
  await new Promise<void>((resolve) => reservation.close(() => resolve()));
  await page.locator('#configuration').fill(JSON.stringify({
    inbounds: [{ protocol: 'socks', listen: '127.0.0.1', port, settings: { auth: 'noauth', udp: true } }],
    outbounds: [{ protocol: 'freedom' }],
  }));
  await expect(page.locator('#validation')).toHaveClass('validation valid');
  await expect(page.locator('#protocols')).toHaveText('socks → freedom');
  await page.getByRole('button', { name: 'Connect Xray', exact: true }).click();
  await expect(page.locator('#status-label')).toHaveText('Connected');
  const reply = await new Promise<Buffer>((resolve, reject) => {
    const socket = createConnection({ host: '127.0.0.1', port }, () => socket.write(Buffer.from([5, 1, 0])));
    socket.setTimeout(5000, () => { socket.destroy(); reject(new Error('SOCKS handshake timed out')); });
    socket.once('data', (data) => { socket.destroy(); resolve(typeof data === 'string' ? Buffer.from(data) : data); });
    socket.once('error', reject);
  });
  expect([...reply]).toEqual([5, 0]);
  await page.getByRole('button', { name: 'Disconnect Xray', exact: true }).click();
  await expect(page.locator('#status-label')).toHaveText('Disconnected');
});

test('has a sandboxed renderer and a readable desktop layout', async () => {
  const security = await app.evaluate(({ BrowserWindow }) => {
    const preferences = (BrowserWindow.getAllWindows()[0].webContents as any).getLastWebPreferences();
    return { sandbox: preferences?.sandbox, isolation: preferences?.contextIsolation, node: preferences?.nodeIntegration };
  });
  expect(security).toEqual({ sandbox: true, isolation: true, node: false });
  expect(await page.evaluate(() => typeof (window as any).require)).toBe('undefined');
  await page.screenshot({ path: 'test-results/desktop.png', fullPage: true });
  await app.evaluate(({ BrowserWindow }) => { BrowserWindow.getAllWindows()[0].setSize(640, 720); });
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
  await expect(page.getByRole('heading', { name: 'Xrayc' })).toBeVisible();
});
