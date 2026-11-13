import { app, BrowserWindow, ipcMain, Menu, nativeImage, safeStorage, Tray } from 'electron';
import { readFile, writeFile, mkdir, rename, unlink } from 'node:fs/promises';
import path from 'node:path';
import { ConnectionManager } from './connection-manager';
import type { Phase, Preferences, Result, Status, Validation } from '../shared/types';

let window: BrowserWindow | undefined;
let tray: Tray;
let quitting = false;
let readyToQuit = false;
let connection: ConnectionManager;
const MAX_CONFIG = 1024 * 1024;
const configPath = () => path.join(app.getPath('userData'), 'configuration.enc');
const storageAvailable = () => safeStorage.isEncryptionAvailable()
  && (process.platform !== 'linux' || safeStorage.getSelectedStorageBackend() !== 'basic_text');
const errorText = (error: unknown) => error instanceof Error ? error.message : String(error);

function checkedText(text: unknown): string {
  if (typeof text !== 'string' || Buffer.byteLength(text, 'utf8') > MAX_CONFIG) throw new Error('Configuration must be smaller than 1 MB.');
  return text;
}

async function loadPreferences(): Promise<Preferences> {
  const available = storageAvailable();
  try {
    const bytes = await readFile(configPath());
    if (!available) return { config: '', remember: true, storageAvailable: false };
    return { config: safeStorage.decryptString(bytes), remember: true, storageAvailable: true };
  } catch (error: any) {
    if (error.code === 'ENOENT') return { config: '', remember: false, storageAvailable: available };
    throw new Error('The saved configuration could not be decrypted. Unlock your system keyring, or paste your configuration again.');
  }
}

async function savePreferences(text: unknown, remember: unknown): Promise<Result> {
  try {
    const config = checkedText(text);
    if (typeof remember !== 'boolean') throw new Error('Invalid save preference.');
    if (!remember) {
      await unlink(configPath()).catch((error) => { if (error.code !== 'ENOENT') throw error; });
      return { ok: true };
    }
    if (!storageAvailable()) throw new Error('Secure storage is unavailable. Unlock your system keyring, or leave “Remember” off.');
    await mkdir(app.getPath('userData'), { recursive: true, mode: 0o700 });
    const temporary = `${configPath()}.tmp`;
    await writeFile(temporary, safeStorage.encryptString(config), { mode: 0o600 });
    await rename(temporary, configPath());
    return { ok: true };
  } catch (error) { return { ok: false, error: errorText(error) }; }
}

function connectionIcon(phase: Phase) {
  const name = phase === 'connected' ? 'icon' : ['connecting', 'disconnecting'].includes(phase) ? 'icon-connecting' : 'icon-disconnected';
  return nativeImage.createFromPath(path.join(__dirname, `../assets/${name}.png`));
}

function showWindow() {
  if (quitting) return;
  const current = window ?? createWindow();
  if (current.isMinimized()) current.restore();
  current.show();
  current.focus();
}

function updateStatus(status: Status) {
  const icon = connectionIcon(status.phase);
  tray.setImage(icon.resize({ width: 24, height: 24 }));
  tray.setToolTip(`Xrayc — ${status.message}`);
  tray.setContextMenu(Menu.buildFromTemplate([
    { label: status.message, enabled: false },
    { type: 'separator' },
    { label: 'Show Xrayc', click: showWindow },
    { label: 'Quit Xrayc', click: () => app.quit() },
  ]));
  app.dock?.setIcon(icon);
  if (window && !window.isDestroyed()) {
    window.setIcon(icon);
    window.webContents.send('vpn:status', status);
  }
}

function createWindow() {
  window = new BrowserWindow({
    title: 'Xrayc', width: 640, height: 720, minWidth: 440, minHeight: 540,
    backgroundColor: '#0e1118', icon: connectionIcon(connection.status.phase),
    webPreferences: { preload: path.join(__dirname, 'preload.cjs'), contextIsolation: true,
      nodeIntegration: false, sandbox: true, webSecurity: true },
  });
  window.webContents.setWindowOpenHandler(() => ({ action: 'deny' }));
  window.webContents.on('will-navigate', (event) => event.preventDefault());
  if (!app.isPackaged && process.env.XRAYC_DEV_URL === 'http://127.0.0.1:5173') {
    void window.loadURL(process.env.XRAYC_DEV_URL);
  } else void window.loadFile(path.join(__dirname, 'index.html'));
  window.on('close', (event) => {
    if (quitting) return;
    event.preventDefault();
    window!.hide();
  });
  window.on('closed', () => { window = undefined; });
  return window;
}

if (!app.requestSingleInstanceLock()) app.quit();
else {
  app.on('second-instance', () => { if (app.isReady()) showWindow(); });
  void app.whenReady().then(() => {
    Menu.setApplicationMenu(process.platform === 'darwin' ? Menu.buildFromTemplate([
      { role: 'appMenu' }, { role: 'editMenu' }, { role: 'windowMenu' },
    ]) : null);
    const dir = app.isPackaged ? path.join(process.resourcesPath, 'bin') : path.join(app.getAppPath(), 'resources/bin');
    connection = new ConnectionManager(path.join(dir, `xrayc-service${process.platform === 'win32' ? '.exe' : ''}`), updateStatus);
    tray = new Tray(connectionIcon(connection.status.phase).resize({ width: 24, height: 24 }));
    tray.on('click', showWindow);
    // Only the application window may use the deliberately small IPC API.
    for (const [channel, handler] of Object.entries({
      'app:load': async () => ({ preferences: await loadPreferences(), status: connection.status, version: app.getVersion() }),
      'config:validate': async (text: unknown) => connection.call<Validation>('validate', checkedText(text)),
      'config:save': savePreferences,
      'vpn:connect': async (text: unknown): Promise<Result> => {
        try { return await connection.call('connect', checkedText(text)); }
        catch (error) { return { ok: false, error: errorText(error) }; }
      },
      'vpn:disconnect': async (): Promise<Result> => {
        try { return await connection.call('disconnect'); }
        catch (error) { return { ok: false, error: errorText(error) }; }
      },
    })) {
      ipcMain.handle(channel, (event, ...args) => {
        if (!window || event.sender !== window.webContents || event.senderFrame !== window.webContents.mainFrame) throw new Error('Unauthorized IPC request.');
        return (handler as (...args: any[]) => unknown)(...args);
      });
    }
    createWindow();
    updateStatus(connection.status);
    app.on('activate', showWindow);
  });
  // Keep the tray available even if there are no windows.
  app.on('window-all-closed', () => {});
  app.on('before-quit', (event) => {
    if (readyToQuit) return;
    event.preventDefault();
    if (quitting) return;
    quitting = true;
    void (connection?.close() ?? Promise.resolve()).finally(() => { readyToQuit = true; app.quit(); });
  });
}
