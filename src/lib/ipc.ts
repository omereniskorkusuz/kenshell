/**
 * Typed bridge to the Rust backend. Keep EVERY `invoke` / `listen` call in this module.
 * Types mirror `src-tauri/src/model.rs` and `error.rs` (camelCase) — keep them in sync.
 */
import { Channel, invoke } from '@tauri-apps/api/core';
import type { ScanHost } from './netscan';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import type { SessionPatch } from './bulk';
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
  | 'badPassword'
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
  /** Read the device identity in the background after login (`show version`, hidden). */
  identify: boolean;
  /** Device this session last identified. */
  deviceId: number | null;
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
  | 'identify'
>;

/** What the identity command reported (P3-T04). */
export interface DeviceFacts {
  hostname: string | null;
  model: string | null;
  version: string | null;
  serial: string | null;
  uptime: string | null;
}

/** A device / session note (P3-T08). */
export interface Note {
  id: number;
  sessionId: number | null;
  deviceId: number | null;
  /** Normalized interface key for interface notes (`gigabitethernet0/24`). */
  interfaceKey: string | null;
  /** Display form, e.g. `Gi0/24`. */
  interfaceShort: string | null;
  body: string;
  pinned: boolean;
  createdAt: number;
  updatedAt: number;
}

export type ColumnKind = 'text' | 'number' | 'iface' | 'ip' | 'status';

export interface AnalyzerColumn {
  /** Stable id; label = `analyzer.col.<key>` (falls back to the key). */
  key: string;
  kind: ColumnKind;
}

export interface AnalyzerTable {
  parser: string;
  command: string;
  columns: AnalyzerColumn[];
  rows: string[][];
  truncated: boolean;
}

export type BackupSource = 'running' | 'startup' | 'file';

export interface BackupInfo {
  id: number;
  sessionId: number | null;
  deviceId: number | null;
  hostname: string | null;
  source: BackupSource;
  createdAt: number;
  size: number;
  lines: number;
  sha256: string;
  /** On a save: the newest backup already had this content. */
  unchanged?: boolean;
}

export type DiffKind = 'same' | 'changed' | 'removed' | 'added';

export interface DiffRow {
  kind: DiffKind;
  leftNo: number | null;
  left: string | null;
  rightNo: number | null;
  right: string | null;
}

export interface ConfigDiff {
  rows: DiffRow[];
  added: number;
  removed: number;
  changed: number;
  coarse: boolean;
}

export interface ScanNetwork {
  iface: string;
  ip: string;
  prefix: number;
  network: string;
  cidr: string;
  defaultTarget: string;
}

export interface ScanNetworks {
  networks: ScanNetwork[];
  defaultTarget: string | null;
  maxAddresses: number;
  concurrency: number;
  ports: number[];
  /** Prefixes in the embedded MAC vendor table. */
  ouiSize: number;
}

/** Why a target may not be scanned (P4-T12 scope rules). */
export type ScopeReason = 'invalid' | 'notPrivate' | 'notConnected' | 'tooLarge';

export interface ScanProgress {
  stage: 'sweep' | 'arp' | 'details';
  done: number;
  total: number;
}

export interface ScanResult {
  hosts: ScanHost[];
  icmpAvailable: boolean;
  cancelled: boolean;
}

export interface ScanCheck {
  range: { first: string; last: string };
  count: number;
  iface: string;
}

export interface Neighbor {
  name: string;
  ip: string | null;
  platform: string | null;
  capability: string | null;
  localInterface: string;
  remoteInterface: string | null;
  cdp: boolean;
  lldp: boolean;
  /** LLDP chassis id when it is a MAC. */
  chassis: string | null;
  /** Manufacturer of the chassis MAC (IEEE OUI table). */
  vendor: string | null;
}

/** What one live session contributes to the topology map. */
export interface Discovery {
  neighbors: Neighbor[];
  /** Addresses learned on ports (12 hex digits). */
  macs: { mac: string; vlan: string | null; interface: string; vendor: string | null }[];
  arp: { ip: string; mac: string }[];
  ownMac: string | null;
  ownVendor: string | null;
}

/** A device read over SNMP for the topology map. */
export interface SnmpDevice {
  name: string | null;
  description: string | null;
  discovery: Discovery;
  /** ifIndex → interface name. */
  interfaces: [number, string][];
}

/** An address a flow exporter saw entering one of its ports. */
export interface FlowHost {
  agent: string;
  ifIndex: number;
  mac: string | null;
  ip: string | null;
  vendor: string | null;
}

export interface FlowSnapshot {
  running: boolean;
  sflowPort: number | null;
  netflowPort: number | null;
  sflowDatagrams: number;
  netflowDatagrams: number;
  hosts: FlowHost[];
}

export interface FileSummary {
  kind: 'showTech' | 'crashinfo' | 'log' | 'text';
  lines: number;
  bytes: number;
  sections: { title: string; start: number; end: number }[];
  facts: [string, string][];
  messages: { mnemonic: string; severity: number; count: number; firstLine: number }[];
  findings: Finding[];
}

export type Severity = 'critical' | 'warning' | 'info' | 'ok';

export interface Finding {
  severity: Severity;
  /** UI text: `diag.<code>` with `params`. */
  code: string;
  params: Record<string, string>;
  command: string;
}

export interface CommandOutput {
  command: string;
  text: string | null;
}

export interface ParserInfo {
  id: string;
  command: string;
  aliases: string[];
  families: string[];
}

export interface Snippet {
  id: number;
  /** `Ports/Access` style path; empty = top level. */
  folder: string;
  name: string;
  body: string;
  /** Vendor family, or null = any device. */
  vendor: string | null;
  createdAt: number;
  updatedAt: number;
}

export interface SnippetInput {
  folder: string;
  name: string;
  body: string;
  vendor: string | null;
}

export interface PinnedNote {
  sessionId: number | null;
  deviceId: number | null;
  /** First line of the note. */
  text: string;
}

export interface NoteHit {
  sessionId: number | null;
  deviceId: number | null;
}

export interface DeviceRecord {
  id: number;
  /** Vendor profile id, e.g. `ruijie-rgos`. */
  vendor: string;
  hostname: string | null;
  model: string | null;
  osVersion: string | null;
  serial: string | null;
  firstSeen: number;
  lastSeen: number;
}

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

export type KeyKind = 'ed25519' | 'rsa3072' | 'rsa4096';
export type KeyFormat = 'openssh' | 'ppk';

/** Public information of a key (never private material). */
export interface KeyInfo {
  algorithm: string;
  bits: number | null;
  comment: string;
  fingerprint: string;
  publicLine: string;
  encrypted: boolean;
}

/** Result of restoring an encrypted backup. */
export interface RestoreReport {
  sessions: ImportReport;
  profilesCreated: number;
  passwords: number;
  knownHosts: number;
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
  /** Forward X11 to the local X server (P5-T09). */
  x11: boolean;
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
  x11: false,
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
  /** Negotiated algorithms (null on older backends). */
  kex: string | null;
  cipher: string | null;
  mac: string | null;
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
  | { type: 'health'; kind: HealthKind }
  /** Last event before the session moves to another window (P2-T19). */
  | { type: 'parked' }
  /** CLI mode from the current prompt (P3-T03). */
  | { type: 'mode'; mode: DeviceMode; sub: string | null }
  /** Host name (prompt) and/or vendor (banner, show version); unknown parts are null. */
  | {
      type: 'identity';
      hostname: string | null;
      vendor: VendorInfo | null;
      /** From the background identity command. */
      facts: DeviceFacts | null;
      /** Stored device row (once the identity command ran). */
      deviceId: number | null;
    }
  /** The device is restarting; the next `mode` event means it is back. */
  | { type: 'reload' };

export type DeviceMode = 'user' | 'privileged' | 'config' | 'boot' | 'shell';

export interface VendorInfo {
  /** Profile id, e.g. `ruijie-rgos`. */
  id: string;
  label: string;
  /** `cisco`, `ruijie`, `generic` … */
  family: string;
}

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

/** Bench runs only: tabs to open at start-up (see bench/measure-app.mjs --sessions). */
export interface BenchOpen {
  target: string;
  count: number;
  /** Show each tab once so its terminal renders the output. */
  visit: boolean;
  /** Self-check scenario (JSON) to play instead (see bench/check-topology.mjs). */
  check: string | null;
}

/** Self-check runs only: hands the result to the script that started the app. */
export async function checkReport(report: string): Promise<void> {
  if (inTauri()) await invoke('check_report', { report });
}

/** Shows the window; resolves with benchmark tabs to open (null outside bench runs). */
export async function appReady(): Promise<BenchOpen | null> {
  return inTauri() ? invoke<BenchOpen | null>('app_ready') : null;
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

export function isLinux(): boolean {
  return /Linux/.test(navigator.platform) && !/Android/.test(navigator.userAgent);
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
  /** Moves a folder under `parentId`, before sibling `beforeId` (null = at the end). */
  folderReorder: (id: number, parentId: number | null, beforeId: number | null) =>
    call<undefined>('folder_reorder', { id, parentId, beforeId }),

  sessionList: () => call<SessionRecord[]>('session_list'),
  sessionCreate: (input: SessionInput) => call<SessionRecord>('session_create', { input }),
  sessionUpdate: (id: number, input: SessionInput) =>
    call<SessionRecord>('session_update', { id, input }),
  sessionDelete: (id: number) => call<undefined>('session_delete', { id }),
  sessionDuplicate: (id: number) => call<SessionRecord>('session_duplicate', { id }),
  deviceList: () => call<DeviceRecord[]>('device_list'),
  notesList: (sessionId: number | null, deviceId: number | null) =>
    call<Note[]>('notes_list', { sessionId, deviceId }),
  noteCreate: (sessionId: number | null, deviceId: number | null, body: string) =>
    call<Note>('note_create', { sessionId, deviceId, body }),
  noteUpdate: (id: number, body: string) => call<Note>('note_update', { id, body }),
  notePin: (id: number, pinned: boolean) => call<Note>('note_pin', { id, pinned }),
  noteDelete: (id: number) => call<undefined>('note_delete', { id }),
  notesSearch: (text: string) => call<NoteHit[]>('notes_search', { text }),
  notesExport: (path: string) => call<undefined>('notes_export', { path }),
  notesPinned: () => call<PinnedNote[]>('notes_pinned'),
  analyzeOutput: (text: string, family: string | null, command: string | null) =>
    call<AnalyzerTable | null>('analyze_output', { text, family, command }),
  /** Read-only `show` command run hidden from the terminal (diagnostics, backups). */
  liveCapture: (id: number, command: string, timeoutSecs?: number) =>
    call<string>('live_capture', { id, command, timeoutSecs: timeoutSecs ?? null }),
  backupTake: (
    id: number,
    sessionId: number | null,
    deviceId: number | null,
    hostname: string | null,
    source: BackupSource,
  ) => call<BackupInfo>('backup_take', { id, sessionId, deviceId, hostname, source }),
  backupImport: (
    sessionId: number | null,
    deviceId: number | null,
    hostname: string | null,
    text: string,
    source?: BackupSource,
  ) =>
    call<BackupInfo>('backup_import', {
      sessionId,
      deviceId,
      hostname,
      text,
      source: source ?? null,
    }),
  backupsList: (sessionId: number | null, deviceId: number | null) =>
    call<BackupInfo[]>('backups_list', { sessionId, deviceId }),
  backupText: (id: number) => call<string>('backup_text', { id }),
  backupDelete: (id: number) => call<undefined>('backup_delete', { id }),
  backupExport: (id: number, path: string) => call<undefined>('backup_export', { id, path }),
  configDiff: (left: string, right: string) => call<ConfigDiff>('config_diff', { left, right }),
  configFileRead: (path: string) => call<string>('config_file_read', { path }),
  configLiveDiff: (id: number) => call<ConfigDiff>('config_live_diff', { id }),
  backupDiffLive: (backupId: number, id: number) =>
    call<ConfigDiff>('backup_diff_live', { backupId, id }),
  netscanNetworks: () => call<ScanNetworks>('netscan_networks'),
  ouiLookup: (mac: string) =>
    call<{ vendor: string | null; tableSize: number }>('oui_lookup', { mac }),
  netscanCancel: () => call<undefined>('netscan_cancel'),
  /** A scanned device's web UI (private IPv4 only; the URL is built by the backend). */
  openDeviceWeb: (ip: string, https: boolean) => call<undefined>('open_device_web', { ip, https }),
  /** Rejects with `{ reason: ScopeReason }` when the target is out of scope. */
  netscanCheck: (target: string) => call<ScanCheck>('netscan_check', { target }),
  neighborsDiscover: (id: number, family: string | null) =>
    call<Neighbor[]>('neighbors_discover', { id, family }),
  /** Neighbours plus (with `hosts`) the MAC and ARP tables, read hidden. */
  topologyDiscover: (id: number, family: string | null, hosts: boolean) =>
    call<Discovery>('topology_discover', { id, family, hosts }),
  /** SNMP community for the topology map: stored in the keychain, never read back. */
  snmpCommunityStatus: () => call<boolean>('snmp_community_status'),
  snmpCommunitySet: (community: string) => call<undefined>('snmp_community_set', { community }),
  snmpCommunityClear: () => call<undefined>('snmp_community_clear'),
  /** Reads `host` (or `host:port`) over SNMPv2c with the stored community. */
  topologySnmp: (host: string) => call<SnmpDevice>('topology_snmp', { host }),
  /** sFlow / NetFlow listeners (a port of 0 leaves that protocol off). */
  flowStart: (sflowPort: number, netflowPort: number) =>
    call<FlowSnapshot>('flow_start', { sflowPort, netflowPort }),
  flowStop: () => call<FlowSnapshot>('flow_stop'),
  flowSnapshot: () => call<FlowSnapshot>('flow_snapshot'),
  logfileAnalyze: (path: string) => call<FileSummary>('logfile_analyze', { path }),
  logfileLines: (path: string, start: number, end: number) =>
    call<string>('logfile_lines', { path, start, end }),
  configAudit: (family: string | null, config: string) =>
    call<Finding[]>('config_audit', { family, config }),
  diagnosticsCommands: (family: string | null) =>
    call<string[]>('diagnostics_commands', { family }),
  diagnosticsEvaluate: (family: string | null, outputs: CommandOutput[]) =>
    call<Finding[]>('diagnostics_evaluate', { family, outputs }),
  analyzerParsers: () => call<ParserInfo[]>('analyzer_parsers'),
  analyzerExportCsv: (path: string, headers: string[], rows: string[][]) =>
    call<undefined>('analyzer_export_csv', { path, headers, rows }),
  snippetsList: () => call<Snippet[]>('snippets_list'),
  snippetCreate: (input: SnippetInput) => call<Snippet>('snippet_create', { input }),
  snippetUpdate: (id: number, input: SnippetInput) =>
    call<Snippet>('snippet_update', { id, input }),
  snippetDelete: (id: number) => call<undefined>('snippet_delete', { id }),
  historyList: (sessionId: number | null, deviceId: number | null) =>
    call<string[]>('history_list', { sessionId, deviceId }),
  historyAdd: (sessionId: number | null, deviceId: number | null, command: string) =>
    call<undefined>('history_add', { sessionId, deviceId, command }),
  historyClear: (sessionId: number | null, deviceId: number | null) =>
    call<undefined>('history_clear', { sessionId, deviceId }),
  sessionReorder: (id: number, folderId: number | null, beforeId: number | null) =>
    call<undefined>('session_reorder', { id, folderId, beforeId }),
  /** One change set for many sessions; all or nothing. */
  sessionBulkUpdate: (ids: number[], patch: SessionPatch) =>
    call<SessionRecord[]>('session_bulk_update', { ids, patch }),
  sessionBulkDelete: (ids: number[]) => call<number>('session_bulk_delete', { ids }),

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
  /** Encrypted backup (library + saved passwords); Argon2id (64 MiB) takes a moment. */
  vaultExport: (path: string, password: string) =>
    call<undefined>('vault_export', { path, password }),
  vaultImport: (path: string, password: string) =>
    call<RestoreReport>('vault_import', { path, password }),
  /** Empty passphrase = unencrypted key. OpenSSH also writes `<path>.pub`. */
  keyGenerate: (
    kind: KeyKind,
    comment: string,
    format: KeyFormat,
    passphrase: string,
    path: string,
  ) => call<KeyInfo>('key_generate', { kind, comment, format, passphrase, path }),
  keyInspect: (path: string) => call<KeyInfo>('key_inspect', { path }),
  keyConvert: (
    path: string,
    passphrase: string,
    format: KeyFormat,
    newPassphrase: string,
    dest: string,
  ) => call<KeyInfo>('key_convert', { path, passphrase, format, newPassphrase, dest }),
  /** Native dialog; resolves null when cancelled. */
  pickPath: (kind: 'file' | 'folder' | 'save', extensions?: string[], name?: string) =>
    call<string | null>('pick_path', { kind, extensions: extensions ?? null, name: name ?? null }),

  consoleList: () => call<ConsoleSnapshot>('console_list'),
  consoleRescan: () => call<ConsoleSnapshot>('console_rescan'),

  liveWrite: (id: RuntimeId, data: string) => call<undefined>('live_write', { id, data }),
  liveResize: (id: RuntimeId, cols: number, rows: number) =>
    call<undefined>('live_resize', { id, cols, rows }),
  /** Session recording (P5-T06): asciicast v2 into the log folder. */
  liveRecordStart: (id: RuntimeId, name: string, cols: number, rows: number) =>
    call<string>('live_record_start', { id, name, cols, rows }),
  liveRecordStop: (id: RuntimeId) => call<string | null>('live_record_stop', { id }),
  recordingRead: (path: string) => call<string>('recording_read', { path }),
  textExport: (path: string, text: string) => call<undefined>('text_export', { path, text }),
  /** AI assistant key in the OS keychain (P5-T08). */
  /** Linux: may this user open serial devices (dialout / uucp group, P5-T12)? */
  serialAccess: () => call<{ ok: boolean; group: string | null }>('serial_access'),
  aiKeyStatus: () => call<boolean>('ai_key_status'),
  aiKeySet: (key: string) => call<undefined>('ai_key_set', { key }),
  aiKeyClear: () => call<undefined>('ai_key_clear'),
  aiKeyGet: () => call<string | null>('ai_key_get'),
  liveBreak: (id: RuntimeId) => call<undefined>('live_break', { id }),
  liveClose: (id: RuntimeId) => call<undefined>('live_close', { id }),
  liveReconnect: (id: RuntimeId) => call<undefined>('live_reconnect', { id }),
  /** Login answers (may contain passwords — never log). */
  liveAnswer: (id: RuntimeId, answers: string[]) => call<undefined>('live_answer', { id, answers }),
  knownHostForget: (host: string, port: number) =>
    call<undefined>('known_host_forget', { host, port }),

  /** Holds the session's output while its tab moves; the channels get end markers. */
  livePark: (id: RuntimeId) => invoke<undefined>('live_park', { id }),
  /** Opens a window for a parked session; resolves with its label. */
  windowDetach: (request: DetachRequest) => invoke<string>('window_detach', { request }),
  /** The tab handed to this window (detached windows only; null in the main window). */
  windowAdopt: () => (inTauri() ? invoke<Handoff | null>('window_adopt') : Promise.resolve(null)),
};

/** Moving tabs between windows needs the desktop app (no mock). */
export const canDetach = (): boolean => inTauri();

export interface DetachRequest {
  runtimeId: RuntimeId;
  title: string;
  /** Opaque tab snapshot (see stores/tabs.ts). */
  payload: string;
  x: number | null;
  y: number | null;
}

export interface Handoff {
  runtimeId: RuntimeId;
  payload: string;
}

function channels(onData: (bytes: Uint8Array) => void, onEvent: (event: SessionEvent) => void) {
  const data = new Channel<ArrayBuffer>();
  data.onmessage = (buf) => onData(new Uint8Array(buf));
  const events = new Channel<SessionEvent>();
  events.onmessage = onEvent;
  return { onData: data, onEvent: events };
}

/** Routes a live session's output (held output first) to this window. */
export function liveAttach(
  id: RuntimeId,
  onData: (bytes: Uint8Array) => void,
  onEvent: (event: SessionEvent) => void,
): Promise<undefined> {
  return invoke<undefined>('live_attach', { id, ...channels(onData, onEvent) });
}

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
  return invoke<RuntimeId>('live_open', { request, ...channels(onData, onEvent) });
}

export async function onConsoleChanged(cb: (change: ConsoleChange) => void): Promise<UnlistenFn> {
  if (!inTauri()) return (await loadMock()).onConsoleChanged(cb);
  return listen<ConsoleChange>(EVENTS.consoleChanged, (e) => cb(e.payload));
}

/**
 * Scans `target` (scope-checked again by the backend). Rejects with
 * `{ type: 'scope', reason }` or `{ type: 'busy' }`.
 */
export async function netscanRun(
  target: string,
  onProgress: (p: ScanProgress) => void,
): Promise<ScanResult> {
  if (!inTauri()) return (await loadMock()).netscanRun(target, onProgress);
  const ch = new Channel<ScanProgress>();
  ch.onmessage = onProgress;
  return invoke<ScanResult>('netscan_run', { target, onProgress: ch });
}

// ------------------------------------------------------------------ file transfer (P5-T01)

export interface TransferProgress {
  done: number;
  total: number;
}

export interface SftpEntry {
  name: string;
  path: string;
  isDir: boolean;
  isLink: boolean;
  size: number;
  /** Seconds since 1970 (0 = unknown). */
  modified: number;
}

function progressChannel(onProgress: (p: TransferProgress) => void) {
  const ch = new Channel<TransferProgress>();
  ch.onmessage = onProgress;
  return ch;
}

/** File transfer over a connected SSH tab (SCP, SFTP). `transfer` = a UI id for cancel. */
export const transfer = {
  async scpDownload(
    id: number,
    transfer: number,
    remote: string,
    local: string,
    onProgress: (p: TransferProgress) => void,
  ) {
    if (!inTauri()) return (await loadMock()).mockTransfer(onProgress);
    return invoke<number>('scp_download', {
      id,
      transfer,
      remote,
      local,
      onProgress: progressChannel(onProgress),
    });
  },
  async scpUpload(
    id: number,
    transfer: number,
    local: string,
    remote: string,
    onProgress: (p: TransferProgress) => void,
  ) {
    if (!inTauri()) return (await loadMock()).mockTransfer(onProgress);
    return invoke<undefined>('scp_upload', {
      id,
      transfer,
      local,
      remote,
      onProgress: progressChannel(onProgress),
    });
  },
  async sftpDownload(
    sftp: number,
    transfer: number,
    remote: string,
    local: string,
    onProgress: (p: TransferProgress) => void,
  ) {
    if (!inTauri()) return (await loadMock()).mockTransfer(onProgress);
    return invoke<number>('sftp_download', {
      sftp,
      transfer,
      remote,
      local,
      onProgress: progressChannel(onProgress),
    });
  },
  async sftpUpload(
    sftp: number,
    transfer: number,
    local: string,
    remoteDir: string,
    onProgress: (p: TransferProgress) => void,
  ) {
    if (!inTauri()) return (await loadMock()).mockTransfer(onProgress);
    return invoke<undefined>('sftp_upload', {
      sftp,
      transfer,
      local,
      remoteDir,
      onProgress: progressChannel(onProgress),
    });
  },
  cancel: (transfer: number) => call<undefined>('transfer_cancel', { transfer }),
  sftpOpen: (id: number) => call<{ sftp: number; home: string }>('sftp_open', { id }),
  sftpList: (sftp: number, path: string) => call<SftpEntry[]>('sftp_list', { sftp, path }),
  sftpMkdir: (sftp: number, path: string) => call<undefined>('sftp_mkdir', { sftp, path }),
  sftpRemove: (sftp: number, path: string, isDir: boolean) =>
    call<undefined>('sftp_remove', { sftp, path, isDir }),
  sftpRename: (sftp: number, from: string, to: string) =>
    call<undefined>('sftp_rename', { sftp, from, to }),
  sftpClose: (sftp: number) => call<undefined>('sftp_close', { sftp }),
};

// ------------------------------------------------- File servers (P5-T02 TFTP, P5-T03 FTP/SCP)

export type ServerKind = 'tftp' | 'ftp' | 'scp';

export type ServerEvent =
  | {
      kind: 'started';
      id: number;
      peer: string;
      file: string;
      upload: boolean;
      size: number | null;
    }
  | { kind: 'progress'; id: number; bytes: number }
  | { kind: 'done'; id: number; bytes: number }
  | { kind: 'failed'; id: number; error: string };

export interface ServerStatus {
  kind: ServerKind;
  addr: string;
  root: string;
  allowUpload: boolean;
  user: string | null;
  password: string | null;
  fingerprints: string[];
}

export interface ServerOptions {
  kind: ServerKind;
  root: string;
  bindIp: string;
  port: number;
  allowUpload: boolean;
  overwrite: boolean;
  user: string | null;
  password: string | null;
}

export const fileServers = {
  async start(opts: ServerOptions, onEvent: (e: ServerEvent) => void): Promise<ServerStatus[]> {
    if (!inTauri())
      return (await loadMock()).mockServerStart(opts, onEvent as (e: unknown) => void) as Promise<
        ServerStatus[]
      >;
    const ch = new Channel<ServerEvent>();
    ch.onmessage = onEvent;
    return invoke<ServerStatus[]>('server_start', { opts, onEvent: ch });
  },
  stop: (kind: ServerKind) => call<ServerStatus[]>('server_stop', { kind }),
  status: () => call<ServerStatus[]>('server_status'),
};

// ------------------------------------------------------------- Ping / traceroute (P5-T05)

export type DiagKind = 'ping' | 'trace';

export const diag = {
  /** Resolves with the tool's exit code (null when stopped). */
  async run(
    job: number,
    kind: DiagKind,
    target: string,
    count: number | null,
    resolve: boolean,
    onLine: (line: string) => void,
  ): Promise<number | null> {
    if (!inTauri()) return (await loadMock()).mockDiagRun(job, kind, target, count, onLine);
    const ch = new Channel<string>();
    ch.onmessage = onLine;
    return invoke<number | null>('diag_run', { job, kind, target, count, resolve, onLine: ch });
  },
  stop: (job: number) => call<undefined>('diag_stop', { job }),
};

// ------------------------------------------------------------------ Updates (P5-T10)

export interface UpdateInfo {
  version: string;
  current: string;
  notes: string | null;
  date: string | null;
}

export const updates = {
  status: () => call<{ configured: boolean; current: string }>('update_status'),
  check: () => call<UpdateInfo | null>('update_check'),
  async install(onProgress: (done: number, total: number | null) => void): Promise<void> {
    if (!inTauri()) return;
    const ch = new Channel<[number, number | null]>();
    ch.onmessage = ([done, total]) => onProgress(done, total);
    await invoke<undefined>('update_install', { onProgress: ch });
  },
};
