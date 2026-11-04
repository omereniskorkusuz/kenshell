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

// ------------------------------------------------------------------ transport (Tauri or browser mock)

/**
 * Outside Tauri (plain `vite` dev server in a browser) every command is answered by an
 * in-memory mock so the UI can be demoed. The mock is a separate, dynamically imported chunk
 * that the Tauri app never loads.
 */
let mockModule: Promise<typeof import('./mock')> | null = null;
function loadMock(): Promise<typeof import('./mock')> {
  mockModule ??= import('./mock');
  return mockModule;
}

function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  if (inTauri()) return invoke<T>(cmd, args);
  return loadMock().then((m) => m.mockInvoke(cmd, args) as Promise<T>);
}

/**
 * Opens an http(s) URL in the system browser. Other schemes are refused.
 * TODO(lead): route through the Tauri opener plugin once it is wired.
 */
export function openExternal(url: string): boolean {
  let parsed: URL;
  try {
    parsed = new URL(url);
  } catch {
    return false;
  }
  if (parsed.protocol !== 'https:' && parsed.protocol !== 'http:') return false;
  window.open(parsed.href, '_blank', 'noopener,noreferrer');
  return true;
}

// ------------------------------------------------------------------ storage API

export const api = {
  folderList: () => call<Folder[]>('folder_list'),
  folderCreate: (name: string, parentId: number | null) =>
    call<Folder>('folder_create', { name, parentId }),
  folderRename: (id: number, name: string) => call<void>('folder_rename', { id, name }),
  folderMove: (id: number, parentId: number | null) => call<void>('folder_move', { id, parentId }),
  folderDelete: (id: number) => call<void>('folder_delete', { id }),

  sessionList: () => call<SessionRecord[]>('session_list'),
  sessionCreate: (input: SessionInput) => call<SessionRecord>('session_create', { input }),
  sessionUpdate: (id: number, input: SessionInput) =>
    call<SessionRecord>('session_update', { id, input }),
  sessionDelete: (id: number) => call<void>('session_delete', { id }),
  sessionDuplicate: (id: number) => call<SessionRecord>('session_duplicate', { id }),

  settingsAll: () => call<Settings>('settings_all'),
  settingSet: (key: string, value: unknown) => call<void>('setting_set', { key, value }),

  consoleList: () => call<ConsoleSnapshot>('console_list'),
  consoleRescan: () => call<ConsoleSnapshot>('console_rescan'),

  liveWrite: (id: RuntimeId, data: string) => call<void>('live_write', { id, data }),
  liveResize: (id: RuntimeId, cols: number, rows: number) =>
    call<void>('live_resize', { id, cols, rows }),
  liveBreak: (id: RuntimeId) => call<void>('live_break', { id }),
  liveClose: (id: RuntimeId) => call<void>('live_close', { id }),
  liveReconnect: (id: RuntimeId) => call<void>('live_reconnect', { id }),
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
  if (!inTauri()) return (await loadMock()).liveOpen(request, onData, onEvent);
  const data = new Channel<ArrayBuffer>();
  data.onmessage = (buf) => onData(new Uint8Array(buf));
  const events = new Channel<SessionEvent>();
  events.onmessage = onEvent;
  return invoke<RuntimeId>('live_open', { request, onData: data, onEvent: events });
}

export async function onConsoleChanged(cb: (change: ConsoleChange) => void): Promise<UnlistenFn> {
  if (!inTauri()) return (await loadMock()).onConsoleChanged(cb);
  return listen<ConsoleChange>(EVENTS.consoleChanged, (e) => cb(e.payload));
}
