/**
 * Typed bridge to the Rust backend. Keep EVERY `invoke` / `listen` call in this module.
 * Types mirror `src-tauri/src/model.rs` and `error.rs` (camelCase) — keep them in sync.
 */
import { Channel, invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import type * as MockModule from './mock';

// ------------------------------------------------------------------ errors

export type ErrorCode =
  | 'portBusy'
  | 'portNotFound'
  | 'permissionDenied'
  | 'deviceGone'
  | 'hostNotFound'
  | 'connectionRefused'
  | 'timeout'
  | 'authFailed'
  | 'hostKeyChanged'
  | 'noCommonAlgorithm'
  | 'connectionClosed'
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
  /** SSH options (null for other protocols). */
  ssh: SshOptions | null;
  /** Shared login profile (its password is in the OS keychain). */
  credentialProfileId: number | null;
  /** Sent once the device is idle after connecting. */
  autoCommands: string[];
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
  | 'ssh'
  | 'credentialProfileId'
  | 'autoCommands'
>;

export type Settings = Record<string, unknown>;

export interface ImportSources {
  puttyCount: number;
  secureCrtDir: string | null;
  mobaxtermIni: string | null;
}

export interface ImportReport {
  created: number;
  skipped: number;
  errors: string[];
  folder: string | null;
}

export type ImportSource = 'putty' | 'securecrt' | 'mobaxterm' | 'csv';

/** Reusable login; the password itself stays in the OS keychain. */
export interface CredentialProfile {
  id: number;
  name: string;
  username: string | null;
  hasPassword: boolean;
}

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

export interface SshOptions {
  /** Also offer diffie-hellman-group1/14-sha1, ssh-rsa, *-cbc, 3des, hmac-sha1. */
  legacyAlgorithms: boolean;
  keyPath: string | null;
  /** Seconds; null = 30, 0 = off. */
  keepaliveSecs: number | null;
  /** Try SSH agent keys first (off: devices often allow only ~3 attempts). */
  useAgent: boolean;
  /** Connect through this SSH server first (bastion). */
  jump: JumpHost | null;
  /** Started after login; failures are reported in the terminal, the session stays up. */
  forwards: Forward[];
  /** SOCKS5 / HTTP CONNECT proxy for the first TCP connection. */
  proxy: ProxyConfig | null;
}

export interface JumpHost {
  host: string;
  port: number | null;
  username: string | null;
}

/** local = -L, remote = -R, dynamic = -D (SOCKS5). */
export type ForwardKind = 'local' | 'remote' | 'dynamic';

export interface Forward {
  kind: ForwardKind;
  listenPort: number;
  targetHost: string | null;
  targetPort: number | null;
}

export type ProxyKind = 'socks5' | 'http';

export interface ProxyConfig {
  kind: ProxyKind;
  host: string;
  port: number;
}

export const DEFAULT_SSH: SshOptions = {
  legacyAlgorithms: false,
  keyPath: null,
  keepaliveSecs: null,
  useAgent: false,
  jump: null,
  forwards: [],
  proxy: null,
};

export type NetProtocol = Exclude<Protocol, 'serial'>;

export const DEFAULT_PORTS: Record<NetProtocol, number | null> = {
  ssh: 22,
  telnet: 23,
  rlogin: 513,
  raw: null,
};

export type OpenTarget =
  | { type: 'serial'; settings: SerialSettings }
  | {
      type: 'network';
      protocol: NetProtocol;
      host: string;
      port: number | null;
      username: string | null;
      ssh: SshOptions;
    };

export interface HostKeyInfo {
  algorithm: string;
  /** "SHA256:…" */
  fingerprint: string;
  /** First connection: the key was learned and stored just now. */
  firstSeen: boolean;
}

export interface NetLink {
  protocol: Protocol;
  host: string;
  port: number;
  username: string | null;
  hostKey: HostKeyInfo | null;
}

export interface PromptItem {
  text: string;
  /** false = secret (password): never echoed. */
  echo: boolean;
}

export interface PromptRequest {
  instruction: string;
  items: PromptItem[];
}

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
  | { type: 'netLink'; link: NetLink }
  | { type: 'prompt'; request: PromptRequest }
  | { type: 'logging'; path: string | null; error: AppError | null }
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
let mockModule: Promise<typeof MockModule> | null = null;
function loadMock(): Promise<typeof MockModule> {
  mockModule ??= import('./mock');
  return mockModule;
}

function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  if (inTauri()) return invoke<T>(cmd, args);
  return loadMock().then((m) => m.mockInvoke(cmd, args) as Promise<T>);
}

/**
 * Opens an official https link (e.g. a driver download page) in the system browser.
 * In Tauri this goes through the backend `open_external` command (https only, validated);
 * in a plain browser it falls back to `window.open`. Returns false for refused URLs.
 */
export function openExternal(url: string): boolean {
  let parsed: URL;
  try {
    parsed = new URL(url);
  } catch {
    return false;
  }
  if (parsed.protocol !== 'https:') return false;
  if (inTauri()) {
    void invoke('open_external', { url: parsed.href }).catch(() => undefined);
  } else {
    window.open(parsed.href, '_blank', 'noopener,noreferrer');
  }
  return true;
}

// ------------------------------------------------------------------ storage API

export const api = {
  folderList: () => call<Folder[]>('folder_list'),
  folderCreate: (name: string, parentId: number | null) =>
    call<Folder>('folder_create', { name, parentId }),
  folderRename: (id: number, name: string) => call<undefined>('folder_rename', { id, name }),
  folderMove: (id: number, parentId: number | null) =>
    call<undefined>('folder_move', { id, parentId }),
  folderDelete: (id: number) => call<undefined>('folder_delete', { id }),

  sessionList: () => call<SessionRecord[]>('session_list'),
  sessionCreate: (input: SessionInput) => call<SessionRecord>('session_create', { input }),
  sessionUpdate: (id: number, input: SessionInput) =>
    call<SessionRecord>('session_update', { id, input }),
  sessionDelete: (id: number) => call<undefined>('session_delete', { id }),
  sessionDuplicate: (id: number) => call<SessionRecord>('session_duplicate', { id }),
  sessionReorder: (id: number, folderId: number | null, beforeId: number | null) =>
    call<undefined>('session_reorder', { id, folderId, beforeId }),

  settingsAll: () => call<Settings>('settings_all'),
  settingSet: (key: string, value: unknown) => call<undefined>('setting_set', { key, value }),

  /** OS clipboard (no webview paste-permission bubble). */
  clipboardRead: () => call<string>('clipboard_read'),
  clipboardWrite: (text: string) => call<undefined>('clipboard_write', { text }),

  // Passwords are written to the OS keychain and never read back by the UI.
  profileList: () => call<CredentialProfile[]>('profile_list'),
  profileCreate: (name: string, username: string | null, password: string | null) =>
    call<CredentialProfile>('profile_create', { name, username, password }),
  profileUpdate: (id: number, name: string, username: string | null) =>
    call<CredentialProfile>('profile_update', { id, name, username }),
  profileSetPassword: (id: number, password: string | null) =>
    call<undefined>('profile_set_password', { id, password }),
  profileDelete: (id: number) => call<undefined>('profile_delete', { id }),
  sessionPasswordSet: (id: number, password: string | null) =>
    call<undefined>('session_password_set', { id, password }),
  sessionPasswordExists: (id: number) => call<boolean>('session_password_exists', { id }),

  importDetect: () => call<ImportSources>('import_detect'),
  importSessions: (source: ImportSource, path: string | null) =>
    call<ImportReport>('import_sessions', { source, path }),
  exportSessions: (path: string, format: 'json' | 'csv') =>
    call<undefined>('export_sessions', { path, format }),
  /** Native dialog; resolves null when cancelled. */
  pickPath: (kind: 'file' | 'folder' | 'save', extensions?: string[], name?: string) =>
    call<string | null>('pick_path', { kind, extensions: extensions ?? null, name: name ?? null }),

  consoleList: () => call<ConsoleSnapshot>('console_list'),
  consoleRescan: () => call<ConsoleSnapshot>('console_rescan'),

  liveWrite: (id: RuntimeId, data: string) => call<undefined>('live_write', { id, data }),
  liveResize: (id: RuntimeId, cols: number, rows: number) =>
    call<undefined>('live_resize', { id, cols, rows }),
  liveBreak: (id: RuntimeId) => call<undefined>('live_break', { id }),
  liveClose: (id: RuntimeId) => call<undefined>('live_close', { id }),
  liveReconnect: (id: RuntimeId) => call<undefined>('live_reconnect', { id }),
  /** Login answers (may contain passwords — never log). */
  liveAnswer: (id: RuntimeId, answers: string[]) => call<undefined>('live_answer', { id, answers }),
  knownHostForget: (host: string, port: number) =>
    call<undefined>('known_host_forget', { host, port }),
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
