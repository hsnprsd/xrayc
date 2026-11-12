import { spawn, type ChildProcessWithoutNullStreams } from 'node:child_process';
import { createInterface } from 'node:readline';
import type { Status } from '../shared/types';

export class ConnectionManager {
  private child?: ChildProcessWithoutNullStreams;
  private nextId = 0;
  private pending = new Map<number, { resolve: (value: any) => void; reject: (error: Error) => void; timer: NodeJS.Timeout }>();
  status: Status = { phase: 'disconnected', message: 'Xray is stopped.' };

  constructor(private binary: string, private onStatus: (status: Status) => void) {}

  private start() {
    if (this.child) return;
    const child = spawn(this.binary, ['--controller'], { windowsHide: true, stdio: 'pipe' });
    this.child = child;
    // Core diagnostics belong to the connection UI, never a persistent log file.
    child.stderr.resume();
    const lines = createInterface({ input: child.stdout });
    lines.on('line', (line) => {
      try {
        const value = JSON.parse(line);
        if (value.event === 'status') {
          this.status = value.status;
          this.onStatus(this.status);
        } else {
          const request = this.pending.get(value.id);
          if (request) { clearTimeout(request.timer); this.pending.delete(value.id); request.resolve(value.result); }
        }
      } catch { /* Ignore non-protocol output. */ }
    });
    const failed = () => {
      if (this.child !== child) return;
      this.child = undefined;
      for (const request of this.pending.values()) { clearTimeout(request.timer); request.reject(new Error('The network service is unavailable. Reinstall Xrayc or rebuild its network service.')); }
      this.pending.clear();
      if (['connected', 'connecting', 'disconnecting'].includes(this.status.phase)) {
        this.status = { phase: 'error', message: 'The network service stopped. The connection has been closed.' };
        this.onStatus(this.status);
      }
    };
    child.on('error', failed);
    child.on('exit', failed);
    child.stdin.on('error', failed);
  }

  call<T>(type: string, text?: string): Promise<T> {
    this.start();
    return new Promise((resolve, reject) => {
      const id = ++this.nextId;
      const timer = setTimeout(() => { this.pending.delete(id); reject(new Error('The network service did not respond.')); }, 10_000);
      this.pending.set(id, { resolve, reject, timer });
      this.child!.stdin.write(`${JSON.stringify({ id, type, text })}\n`);
    });
  }

  async close(): Promise<void> {
    const child = this.child;
    if (!child) return;
    try { await this.call('disconnect'); } catch { /* EOF also disconnects the network service. */ }
    child.stdin.end();
    await new Promise<void>((resolve) => {
      if (child.exitCode !== null) { resolve(); return; }
      const timer = setTimeout(() => { child.kill(); resolve(); }, 16_000);
      child.once('exit', () => { clearTimeout(timer); resolve(); });
    });
  }
}
