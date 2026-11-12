export type Phase = 'disconnected' | 'connecting' | 'connected' | 'disconnecting' | 'error';

export interface Status {
  phase: Phase;
  message: string;
  connectedAt?: number;
}

export interface Validation {
  valid: boolean;
  message: string;
  protocols: string[];
  inbounds: string[];
  requiresElevation: boolean;
}

export interface Preferences {
  config: string;
  remember: boolean;
  storageAvailable: boolean;
}

export type Result = { ok: true } | { ok: false; error: string };

export interface DesktopAPI {
  load(): Promise<{ preferences: Preferences; status: Status; version: string }>;
  validate(text: string): Promise<Validation>;
  save(text: string, remember: boolean): Promise<Result>;
  connect(text: string): Promise<Result>;
  disconnect(): Promise<Result>;
  onStatus(callback: (status: Status) => void): () => void;
}
