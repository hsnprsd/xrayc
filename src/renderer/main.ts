import './style.css';
import type { DesktopAPI, Status, Validation } from '../shared/types';

declare global { interface Window { xrayc?: DesktopAPI } }
const api = window.xrayc;
const element = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;
const configuration = element<HTMLTextAreaElement>('configuration');
const connect = element<HTMLButtonElement>('connect');
const paste = element<HTMLButtonElement>('paste');
const remember = element<HTMLInputElement>('remember');
const saveMessage = element<HTMLParagraphElement>('save-message');
let status: Status = { phase: 'disconnected', message: 'Xray is stopped.' };
let validation: Validation = emptyValidation();
let editRevision = 0;
let validationTimer: ReturnType<typeof setTimeout>;
let saveTimer: ReturnType<typeof setTimeout>;
let busy = false;
let saveQueue = Promise.resolve();

function emptyValidation(): Validation {
  return { valid: false, message: '', protocols: [], inbounds: [], requiresElevation: false };
}
function renderStatus() {
  const active = ['connected', 'connecting', 'disconnecting'].includes(status.phase);
  const labels = { disconnected: 'Disconnected', connecting: 'Connecting', connected: 'Connected', disconnecting: 'Disconnecting', error: 'Connection failed' };
  element('status-label').textContent = labels[status.phase];
  element('status-dot').className = `status-dot ${status.phase}`;
  element('connection-message').textContent = ['error', 'connecting', 'disconnecting'].includes(status.phase) ? status.message : '';
  element('connection-message').className = `message ${status.phase === 'error' ? 'error' : ''}`;
  connect.classList.toggle('connected', status.phase === 'connected');
  const action = status.phase === 'connecting' ? 'Cancel' : status.phase === 'connected' ? 'Disconnect' : status.phase === 'disconnecting' ? 'Disconnecting…' : 'Connect';
  element('connect-label').textContent = action;
  connect.setAttribute('aria-label', `${action} Xray`);
  connect.disabled = !api || busy || status.phase === 'disconnecting' || (!active && !validation.valid);
  configuration.disabled = active || busy;
  paste.disabled = active || busy || !api;
  renderDuration();
}
function renderDuration() {
  if (status.phase !== 'connected' || !status.connectedAt) { element('duration').textContent = ''; return; }
  const seconds = Math.max(0, Math.floor((Date.now() - status.connectedAt) / 1000));
  element('duration').textContent = [Math.floor(seconds / 3600), Math.floor(seconds / 60) % 60, seconds % 60].map((n) => String(n).padStart(2, '0')).join(':');
}
setInterval(renderDuration, 1000);

async function validate() {
  const revision = ++editRevision;
  validation = emptyValidation();
  renderStatus();
  const empty = !configuration.value.trim();
  element('validation-text').textContent = empty ? 'Paste a configuration to begin.' : 'Checking…';
  element('validation').className = 'validation';
  element('protocols').textContent = '';
  element('permission-note').textContent = '';
  if (!api || empty) return;
  try {
    const result = await api.validate(configuration.value);
    if (revision !== editRevision) return;
    validation = result;
    element('validation-text').textContent = result.valid ? '' : result.message;
    element('validation').className = `validation ${result.valid ? 'valid' : 'invalid'}`;
    if (result.valid) {
      element('protocols').textContent = `${result.inbounds.join(' + ')} → ${result.protocols.join(' + ')}`;
      element('permission-note').textContent = result.requiresElevation ? 'Admin permission required' : '';
    }
  } catch (error) {
    if (revision !== editRevision) return;
    element('validation-text').textContent = error instanceof Error ? error.message : 'Unable to check configuration.';
    element('validation').className = 'validation invalid';
  }
  renderStatus();
}
function save(): Promise<void> {
  if (!api) return Promise.resolve();
  const text = configuration.value;
  const shouldRemember = remember.checked;
  saveQueue = saveQueue.catch(() => {}).then(async () => {
    try {
      const result = await api.save(text, shouldRemember);
      saveMessage.textContent = result.ok ? '' : result.error;
    } catch (error) { saveMessage.textContent = error instanceof Error ? error.message : 'Could not save configuration.'; }
  });
  return saveQueue;
}
function edited() {
  ++editRevision;
  validation.valid = false;
  renderStatus();
  clearTimeout(validationTimer);
  clearTimeout(saveTimer);
  validationTimer = setTimeout(() => void validate(), 250);
  saveTimer = setTimeout(() => void save(), 600);
}
configuration.addEventListener('input', edited);
configuration.addEventListener('blur', () => { clearTimeout(saveTimer); void save(); });
remember.addEventListener('change', () => { clearTimeout(saveTimer); void save(); });
paste.addEventListener('click', async () => {
  try { configuration.value = await navigator.clipboard.readText(); edited(); configuration.focus(); }
  catch { saveMessage.textContent = 'Use Ctrl+V (⌘V on Mac) to paste into the editor.'; configuration.focus(); }
});
connect.addEventListener('click', async () => {
  if (!api || busy) return;
  busy = true;
  renderStatus();
  try {
    const stop = status.phase === 'connected' || status.phase === 'connecting';
    if (!stop) { clearTimeout(saveTimer); await save(); }
    const result = stop ? await api.disconnect() : await api.connect(configuration.value);
    if (!result.ok) status = { phase: 'error', message: result.error };
  } catch (error) { status = { phase: 'error', message: error instanceof Error ? error.message : 'Connection failed.' }; }
  finally { busy = false; renderStatus(); }
});
if (api) {
  api.onStatus((next) => { status = next; renderStatus(); });
  void api.load().then(async ({ preferences, status: initial }) => {
    configuration.value = preferences.config;
    remember.checked = preferences.remember;
    remember.disabled = !preferences.storageAvailable && !preferences.remember;
    if (!preferences.storageAvailable) element('remember-label').title = 'Secure storage is unavailable. Unlock your system keyring to remember configurations.';
    status = initial;
    await validate();
    renderStatus();
  }).catch((error) => { saveMessage.textContent = error instanceof Error ? error.message : 'Could not load preferences.'; renderStatus(); });
} else {
  element('validation-text').textContent = 'Open the desktop application to connect.';
  remember.disabled = true;
  renderStatus();
}
