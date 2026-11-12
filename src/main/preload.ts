import { contextBridge, ipcRenderer } from 'electron';
import type { DesktopAPI, Status } from '../shared/types';

const api: DesktopAPI = {
  load: () => ipcRenderer.invoke('app:load'),
  validate: (text) => ipcRenderer.invoke('config:validate', text),
  save: (text, remember) => ipcRenderer.invoke('config:save', text, remember),
  connect: (text) => ipcRenderer.invoke('vpn:connect', text),
  disconnect: () => ipcRenderer.invoke('vpn:disconnect'),
  onStatus: (callback) => {
    const listener = (_event: unknown, status: Status) => callback(status);
    ipcRenderer.on('vpn:status', listener);
    return () => ipcRenderer.removeListener('vpn:status', listener);
  },
};
contextBridge.exposeInMainWorld('xrayc', api);
