/**
 * Typed bridge to the Rust backend. Keep EVERY `invoke` / `listen` call in this module.
 * Types mirror `src-tauri/src/model.rs` and `error.rs` (camelCase) — keep them in sync.
 */
import { Channel, invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';

// ------------------------------------------------------------------ errors

export type ErrorCode =
  | 'portBusy'
  | 'portNotFound'
  | 'permissionDenied'
  | 'deviceGone'
  | 'notFound'
  | 'invalid'
  | 'unsupported'
  | 'io'
  | 'db'
  | 'internal';

/** Rejection value of every command. Show `errors.<code>` from i18n, never raw OS text. */
export interface AppError {
  code: ErrorCode;
  detail: string | null;
}

export function isAppError(e: unknown): e is AppError {
  return typeof e === 'object' && e !== null && 'code' in e;
}

// ------------------------------------------------------------------ storage

export interface Folder {
  id: number;
  parentId: number | null;
  name: string;
  sortOrder: number;
}

export type Protocol = 'serial' | 'ssh' | 'telnet' | 'rlogin' | 'raw';
export type Parity = 'none' | 'odd' | 'even';
export type FlowControl = 'none' | 'rtscts' | 'xonxoff';

export interface SerialSettings {
  port: string;
  /** null = automatic detection */
  baud: number | null;
  dataBits: number;
  parity: Parity;
  stopBits: number;
  flowControl: FlowControl;
  dtr: boolean | null;
  rts: boolean | null;
}

export const DEFAULT_SERIAL: Omit<SerialSettings, 'port'> = {
  baud: null,
  dataBits: 8,
  parity: 'none',
  stopBits: 1,
  flowControl: 'none',
  dtr: null,
  rts: null,
};

export interface SessionRecord {
  id: number;
  folderId: number | null;
  name: string;
  protocol: Protocol;
  host: string | null;
  port: number | null;
  username: string | null;
  serial: SerialSettings | null;
  /** 'prod' | 'lab' | 'test' | custom */
  envTag: string | null;
  tags: string[];
  favorite: boolean;
  sortOrder: number;
  createdAt: number;
  updatedAt: number;
  lastConnectedAt: number | null;
}

export type SessionInput = Pick<
  SessionRecord,
  | 'folderId'
  | 'name'
  | 'protocol'
  | 'host'
  | 'port'
  | 'username'
  | 'serial'
  | 'envTag'
  | 'tags'
  | 'favorite'
>;

export type Settings = Record<string, unknown>;

// ------------------------------------------------------------------ consoles

export interface ChipInfo {
  vendor: string;
  model: string;
  vid: number;
  pid: number;
  driverName: string | null;
  driverUrl: string | null;
}

export interface ConsolePort {
  key: string;
  path: string;
  altPath: string | null;
  displayName: string;
  chip: ChipInfo | null;
  manufacturer: string | null;
  product: string | null;
  serialNumber: string | null;
  rememberedBaud: number | null;
  lastDevice: string | null;
}

export interface DriverHint {
  chip: ChipInfo;
}

export interface ConsoleSnapshot {
  ports: ConsolePort[];
  driverHints: DriverHint[];
}

export interface ConsoleChange {
  snapshot: ConsoleSnapshot;
  attached: string[];
  detached: string[];
}

// ------------------------------------------------------------------ live sessions

export type RuntimeId = number;

export type OpenTarget = { type: 'serial'; settings: SerialSettings };

export interface OpenRequest {
  sessionId: number | null;
  target: OpenTarget;
  cols: number;
  rows: number;
}

export type ConnState = 'connecting' | 'detecting' | 'connected' | 'disconnected';

export interface BaudTrial {
  baud: number;
  score: number;
  promptMatch: boolean;
  bytes: number;
}

export type HealthKind = 'noData' | 'cableRemoved' | 'cableReturned';

export interface SerialLink {
  path: string;
  displayName: string;
  baud: number;
  framing: string;
  chip: string | null;
  autoDetected: boolean;
}

export type SessionEvent =
  | { type: 'state'; state: ConnState; error: AppError | null }
  | {
      type: 'autobaud';
      trials: BaudTrial[];
      current: number | null;
      result: number | null;
      suggestion: number | null;
    }
  | { type: 'link'; link: SerialLink }
  | { type: 'health'; kind: HealthKind };

// ------------------------------------------------------------------ plumbing

export const inTauri = (): boolean =>
  typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

export const EVENTS = { consoleChanged: 'console://changed' } as const;

// ------------------------------------------------------------------ app

export interface AppInfo {
  version: string;
  os: string;
  arch: string;
}

export async function appReady(): Promise<void> {
  if (inTauri()) await invoke('app_ready');
}

export async function appInfo(): Promise<AppInfo | null> {
  return inTauri() ? invoke<AppInfo>('app_info') : null;
}

/** Platform-aware modifier label used in shortcut hints. */
export function modKey(): string {
  return isMac() ? '⌘' : 'Ctrl';
}

export function isMac(): boolean {
  return /Mac|iPhone|iPad/.test(navigator.platform);
}

// ------------------------------------------------------------------ storage API

export const api = {
  folderList: () => invoke<Folder[]>('folder_list'),
  folderCreate: (name: string, parentId: number | null) =>
    invoke<Folder>('folder_create', { name, parentId }),
  folderRename: (id: number, name: string) => invoke<void>('folder_rename', { id, name }),
  folderMove: (id: number, parentId: number | null) =>
    invoke<void>('folder_move', { id, parentId }),
  folderDelete: (id: number) => invoke<void>('folder_delete', { id }),

  sessionList: () => invoke<SessionRecord[]>('session_list'),
  sessionCreate: (input: SessionInput) => invoke<SessionRecord>('session_create', { input }),
  sessionUpdate: (id: number, input: SessionInput) =>
    invoke<SessionRecord>('session_update', { id, input }),
  sessionDelete: (id: number) => invoke<void>('session_delete', { id }),
  sessionDuplicate: (id: number) => invoke<SessionRecord>('session_duplicate', { id }),

  settingsAll: () => invoke<Settings>('settings_all'),
  settingSet: (key: string, value: unknown) => invoke<void>('setting_set', { key, value }),

  consoleList: () => invoke<ConsoleSnapshot>('console_list'),
  consoleRescan: () => invoke<ConsoleSnapshot>('console_rescan'),

  liveWrite: (id: RuntimeId, data: string) => invoke<void>('live_write', { id, data }),
  liveResize: (id: RuntimeId, cols: number, rows: number) =>
    invoke<void>('live_resize', { id, cols, rows }),
  liveBreak: (id: RuntimeId) => invoke<void>('live_break', { id }),
  liveClose: (id: RuntimeId) => invoke<void>('live_close', { id }),
  liveReconnect: (id: RuntimeId) => invoke<void>('live_reconnect', { id }),
};

/**
 * Opens a live session. `onData` receives raw device bytes; `onEvent` state changes.
 * Resolves with the runtime id as soon as connecting started.
 */
export async function liveOpen(
  request: OpenRequest,
  onData: (bytes: Uint8Array) => void,
  onEvent: (event: SessionEvent) => void,
): Promise<RuntimeId> {
  const data = new Channel<ArrayBuffer>();
  data.onmessage = (buf) => onData(new Uint8Array(buf));
  const events = new Channel<SessionEvent>();
  events.onmessage = onEvent;
  return invoke<RuntimeId>('live_open', { request, onData: data, onEvent: events });
}

export function onConsoleChanged(cb: (change: ConsoleChange) => void): Promise<UnlistenFn> {
  return listen<ConsoleChange>(EVENTS.consoleChanged, (e) => cb(e.payload));
}
