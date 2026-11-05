/**
 * In-memory backend used when the UI runs in a plain browser (`npm run dev`, screenshots).
 * Loaded only through a dynamic import from `ipc.ts` when `inTauri()` is false.
 *
 * Demo hooks (browser console):
 *   kenshellMock.plug()    — attach a Cisco USB console (toast + new row)
 *   kenshellMock.unplug()  — remove the FTDI console (open sessions on it disconnect)
 *   kenshellMock.replug()  — put the FTDI console back (sessions reconnect)
 * Query `?demo=hotplug` plugs the Cisco console 3 s after start.
 * Port names: "…noisy…" → autobaud suggestion, "…silent…" → no data, "COM3" → busy.
 */
import type {
  AppError,
  ConsoleChange,
  ConsolePort,
  ConsoleSnapshot,
  DriverHint,
  Folder,
  OpenRequest,
  RuntimeId,
  SessionEvent,
  SessionInput,
  SessionRecord,
  Settings,
} from '../ipc';
import { isMac } from '../ipc';
import { applyPatch, type SessionPatch } from '../bulk';
import { framingLabel } from '../format';
import { FakeCli } from './cli';

const enc = new TextEncoder();
const now = () => Date.now();
const HOUR = 3_600_000;

const MAC = typeof navigator !== 'undefined' && isMac();
const FTDI_PATH = MAC ? '/dev/cu.usbserial-A10K3XYZ' : 'COM4';
const FTDI_ALT = MAC ? '/dev/tty.usbserial-A10K3XYZ' : null;
const FTDI_NAME = MAC ? 'usbserial-A10K3XYZ' : 'COM4';
const CISCO_PATH = MAC ? '/dev/cu.usbmodem14201' : 'COM6';
const MISSING_PATH = MAC ? '/dev/cu.usbserial-FW01' : 'COM9';

const ftdi: ConsolePort = {
  key: 'A10K3XYZ',
  path: FTDI_PATH,
  altPath: FTDI_ALT,
  displayName: FTDI_NAME,
  chip: {
    vendor: 'FTDI',
    model: 'FT232R',
    vid: 0x0403,
    pid: 0x6001,
    driverName: null,
    driverUrl: null,
  },
  manufacturer: 'FTDI',
  product: 'FT232R USB UART',
  serialNumber: 'A10K3XYZ',
  rememberedBaud: 9600,
  lastDevice: 'core-sw1',
};

const ciscoConsole: ConsolePort = {
  key: 'FOC2301X0AB',
  path: CISCO_PATH,
  altPath: MAC ? '/dev/tty.usbmodem14201' : null,
  displayName: MAC ? 'usbmodem14201' : 'COM6',
  chip: {
    vendor: 'Cisco',
    model: 'USB Console',
    vid: 0x05a6,
    pid: 0x0009,
    driverName: null,
    driverUrl: null,
  },
  manufacturer: 'Cisco',
  product: 'Cisco USB Console',
  serialNumber: 'FOC2301X0AB',
  rememberedBaud: null,
  lastDevice: null,
};

const prolificHint: DriverHint = {
  chip: {
    vendor: 'Prolific',
    model: 'PL2303',
    vid: 0x067b,
    pid: 0x2303,
    driverName: 'PL23XX USB-to-Serial',
    driverUrl: 'https://www.prolific.com.tw/US/ShowProduct.aspx?p_id=225&pcid=41',
  },
};

// ------------------------------------------------------------------ storage

let folders: Folder[] = [
  { id: 1, parentId: null, name: 'PROD', sortOrder: 0 },
  { id: 2, parentId: null, name: 'LAB', sortOrder: 1 },
  { id: 3, parentId: 2, name: 'Rack B', sortOrder: 0 },
];

function serial(port: string, baud: number | null) {
  return {
    port,
    baud,
    dataBits: 8,
    parity: 'none' as const,
    stopBits: 1,
    flowControl: 'none' as const,
    dtr: null,
    rts: null,
  };
}

function record(
  id: number,
  folderId: number | null,
  name: string,
  port: string,
  baud: number | null,
  envTag: string | null,
  lastConnectedAt: number | null,
): SessionRecord {
  return {
    id,
    folderId,
    name,
    protocol: 'serial',
    host: null,
    port: null,
    username: null,
    serial: serial(port, baud),
    envTag,
    tags: [],
    favorite: false,
    ssh: null,
    credentialProfileId: null,
    autoCommands: [],
    sortOrder: id,
    createdAt: now() - 30 * 24 * HOUR,
    updatedAt: now() - 2 * HOUR,
    lastConnectedAt,
  };
}

let sessions: SessionRecord[] = [
  record(1, 1, 'core-sw1', FTDI_PATH, 9600, 'prod', now() - 2 * HOUR),
  record(2, 1, 'fw-edge', MISSING_PATH, 115200, 'prod', now() - 26 * HOUR),
  record(3, 2, 'lab-r2', FTDI_PATH, null, 'lab', now() - 5 * HOUR),
  record(4, 3, 'ruijie-s5750', FTDI_PATH, null, 'test', null),
  record(5, null, 'bench-console', FTDI_PATH, null, null, null),
];
let nextId = 100;

const SETTINGS_KEY = 'kenshell.mock.settings';
let settings: Settings = loadSettings();

function loadSettings(): Settings {
  try {
    const raw = localStorage.getItem(SETTINGS_KEY);
    return raw ? (JSON.parse(raw) as Settings) : {};
  } catch {
    return {};
  }
}

function saveSettings(): void {
  try {
    localStorage.setItem(SETTINGS_KEY, JSON.stringify(settings));
  } catch {
    // private mode / blocked storage: settings stay in memory
  }
}

const fail = (code: AppError['code'], detail: string | null = null): never => {
  const err: AppError = { code, detail };
  throw err;
};

function need<T>(v: T | undefined, what: string): T {
  if (v === undefined) fail('notFound', what);
  return v as T;
}

function wouldCycle(id: number, parentId: number | null): boolean {
  let p = parentId;
  while (p !== null) {
    if (p === id) return true;
    p = folders.find((f) => f.id === p)?.parentId ?? null;
  }
  return false;
}

function validName(name: unknown): string {
  const n = typeof name === 'string' ? name.trim() : '';
  if (!n) fail('invalid', 'empty name');
  return n;
}

function uniqueCopyName(name: string): string {
  const base = name.replace(/ \(\d+\)$/, '');
  for (let i = 2; ; i++) {
    const candidate = `${base} (${i})`;
    if (!sessions.some((s) => s.name === candidate)) return candidate;
  }
}

// ------------------------------------------------------------------ consoles

let ports: ConsolePort[] = [ftdi];
let hints: DriverHint[] = [prolificHint];
const consoleListeners = new Set<(c: ConsoleChange) => void>();

function snapshot(): ConsoleSnapshot {
  return { ports: ports.map((p) => ({ ...p })), driverHints: hints.map((h) => ({ ...h })) };
}

function emitConsoles(attached: string[], detached: string[]): void {
  const change: ConsoleChange = { snapshot: snapshot(), attached, detached };
  consoleListeners.forEach((cb) => cb(change));
  lives.forEach((l) => l.onConsoleChange(change));
}

function portExists(path: string): ConsolePort | undefined {
  return ports.find((p) => p.path === path || p.altPath === path);
}

// ------------------------------------------------------------------ live sessions

/**
 * Browser demo only: the backend DeviceObserver reports modes from prompts; the mock mirrors
 * its generic Cisco-like patterns so the mode colour changes in the demo too.
 */
class MockModeWatch {
  private last = '';
  constructor(private readonly onEvent: (e: SessionEvent) => void) {}
  see(text: string): void {
    const line = text.split(/\r?\n|\r/).pop() ?? '';
    const cfg = /^([\w.-]+)\((config[^)]*)\)#\s*$/.exec(line);
    const priv = /^([\w.-]+)#\s*$/.exec(line);
    const user = /^([\w.-]+)>\s*$/.exec(line);
    const hit = cfg
      ? { mode: 'config' as const, sub: cfg[2] ?? null, host: cfg[1] }
      : priv
        ? { mode: 'privileged' as const, sub: null, host: priv[1] }
        : user
          ? { mode: 'user' as const, sub: null, host: user[1] }
          : null;
    if (!hit) return;
    const key = `${hit.mode}|${hit.sub}`;
    if (key !== this.last) {
      this.last = key;
      this.onEvent({ type: 'mode', mode: hit.mode, sub: hit.sub });
      this.onEvent({ type: 'identity', hostname: hit.host ?? null, vendor: null });
    }
  }
}

class MockLive {
  private timers: ReturnType<typeof setTimeout>[] = [];
  private cli: FakeCli;
  private state: 'connecting' | 'detecting' | 'connected' | 'disconnected' = 'connecting';
  private baud: number | null;
  private waitingForCable = false;

  constructor(
    readonly id: RuntimeId,
    private readonly request: OpenRequest,
    private readonly onData: (bytes: Uint8Array) => void,
    private readonly onEvent: (e: SessionEvent) => void,
  ) {
    const s = sessions.find((x) => x.id === request.sessionId);
    this.cli = new FakeCli((s?.name ?? 'Switch').replace(/[^\w-]/g, '') || 'Switch');
    this.baud = this.serial.baud;
  }

  private get serial() {
    if (this.request.target.type !== 'serial') throw new Error('serial only');
    return this.request.target.settings;
  }

  get path(): string {
    return this.serial.port;
  }

  answer(): void {
    fail('invalid', 'no prompt pending');
  }

  start(): void {
    this.clear();
    const path = this.path;
    this.setState('connecting');
    this.later(250, () => {
      if (/COM3$/i.test(path)) {
        return this.setState('disconnected', {
          code: 'portBusy',
          detail: 'Access is denied. (os error 5)',
        });
      }
      const port = portExists(path);
      const synthetic = /noisy|silent/i.test(path);
      if (!port && !synthetic) {
        return this.setState('disconnected', {
          code: 'portNotFound',
          detail: 'No such file or directory (os error 2)',
        });
      }
      if (this.baud === null) this.autobaud(/noisy/i.test(path));
      else this.connect(false);
    });
  }

  private autobaud(noisy: boolean): void {
    this.setState('detecting');
    const plan = noisy ? [9600, 19200, 38400, 57600, 115200] : [9600, 115200];
    const trials: { baud: number; score: number; promptMatch: boolean; bytes: number }[] = [];
    plan.forEach((baud, i) => {
      this.later(450 * i + 50, () =>
        this.onEvent({
          type: 'autobaud',
          trials: [...trials],
          current: baud,
          result: null,
          suggestion: null,
        }),
      );
      this.later(450 * i + 400, () => {
        const ok = !noisy && baud === 115200;
        trials.push({
          baud,
          score: ok ? 0.97 : noisy && baud === 38400 ? 0.55 : 0.08,
          promptMatch: ok,
          bytes: 64,
        });
        const last = i === plan.length - 1;
        this.onEvent({
          type: 'autobaud',
          trials: [...trials],
          current: last || ok ? null : (plan[i + 1] ?? null),
          result: ok ? baud : null,
          suggestion: last && noisy ? 38400 : null,
        });
        if (ok) {
          this.baud = baud;
          this.later(150, () => this.connect(true));
        } else if (last && noisy) {
          this.baud = 38400;
          this.later(150, () => this.connect(true, true));
        }
      });
    });
  }

  private connect(auto: boolean, garbled = false): void {
    const s = this.serial;
    const port = portExists(this.path);
    this.onEvent({
      type: 'link',
      link: {
        path: this.path,
        displayName: port?.displayName ?? this.path.replace(/^\/dev\//, ''),
        baud: this.baud ?? 9600,
        framing: framingLabel(s.dataBits, s.parity, s.stopBits),
        chip: port?.chip ? `${port.chip.vendor} ${port.chip.model}` : null,
        autoDetected: auto,
      },
    });
    this.setState('connected');
    if (/silent/i.test(this.path)) {
      this.later(3000, () => this.onEvent({ type: 'health', kind: 'noData' }));
      return;
    }
    this.later(120, () => this.emit(garbled ? '\r\n\x1b[2m~xø§ÿ?#\x1b[0m\r\n' : this.cli.banner()));
  }

  write(data: string): void {
    if (this.state !== 'connected') return;
    const out = this.cli.input(data);
    if (out) this.later(8, () => this.emit(out));
  }

  sendBreak(): void {
    if (this.state !== 'connected') fail('invalid', 'not connected');
  }

  close(): void {
    this.clear();
    this.state = 'disconnected';
  }

  reconnect(): void {
    if (this.state === 'connected') return;
    this.waitingForCable = false;
    this.start();
  }

  onConsoleChange(change: ConsoleChange): void {
    const key = ports.find((p) => p.path === this.path)?.key;
    const gone = change.detached.length > 0 && !portExists(this.path);
    if (gone && this.state !== 'disconnected' && !/noisy|silent/i.test(this.path)) {
      this.clear();
      this.waitingForCable = true;
      this.onEvent({ type: 'health', kind: 'cableRemoved' });
      this.setState('disconnected', { code: 'deviceGone', detail: 'device disconnected' });
      return;
    }
    if (this.waitingForCable && key && change.attached.includes(key)) {
      this.waitingForCable = false;
      this.onEvent({ type: 'health', kind: 'cableReturned' });
      if (settings['console.autoReconnect'] !== false) this.later(600, () => this.start());
    }
  }

  private setState(state: MockLive['state'], error: AppError | null = null): void {
    this.state = state;
    this.onEvent({ type: 'state', state, error });
  }

  private modes: MockModeWatch | null = null;

  private emit(text: string): void {
    this.onData(enc.encode(text));
    (this.modes ??= new MockModeWatch(this.onEvent)).see(text);
  }

  private later(ms: number, fn: () => void): void {
    this.timers.push(setTimeout(fn, ms));
  }

  private clear(): void {
    this.timers.forEach(clearTimeout);
    this.timers = [];
  }
}

/** SSH/Telnet demo: password prompt in the terminal, then a fake Cisco CLI. */
class MockNetLive {
  private timers: ReturnType<typeof setTimeout>[] = [];
  private cli: FakeCli;
  private state: 'connecting' | 'connected' | 'disconnected' = 'connecting';
  private modes: MockModeWatch | null = null;

  private out(text: string): void {
    this.onData(enc.encode(text));
    (this.modes ??= new MockModeWatch(this.onEvent)).see(text);
  }

  constructor(
    readonly id: RuntimeId,
    private readonly request: OpenRequest,
    private readonly onData: (bytes: Uint8Array) => void,
    private readonly onEvent: (e: SessionEvent) => void,
  ) {
    const t = request.target;
    const host = t.type === 'network' ? t.host : 'device';
    this.cli = new FakeCli(host.replace(/[^\w-]/g, '-') || 'device');
  }

  private get net() {
    if (this.request.target.type !== 'network') throw new Error('network only');
    return this.request.target;
  }

  start(): void {
    this.state = 'connecting';
    this.onEvent({ type: 'state', state: 'connecting', error: null });
    const t = this.net;
    if (/fail|unreachable/i.test(t.host)) {
      this.later(900, () => {
        this.state = 'disconnected';
        this.onEvent({
          type: 'state',
          state: 'disconnected',
          error: { code: 'timeout', detail: `${t.host}: connect` },
        });
      });
      return;
    }
    if (t.protocol === 'ssh') {
      const items = [
        ...(t.username ? [] : [{ text: 'login as: ', echo: true }]),
        { text: `${t.username ?? 'admin'}@${t.host}'s password: `, echo: false },
      ];
      this.later(400, () => this.onEvent({ type: 'prompt', request: { instruction: '', items } }));
    } else {
      this.later(300, () => this.connected());
    }
  }

  answer(): void {
    if (this.state !== 'connecting') fail('invalid', 'no prompt pending');
    this.later(250, () => this.connected());
  }

  private connected(): void {
    const t = this.net;
    this.state = 'connected';
    this.onEvent({
      type: 'netLink',
      link: {
        protocol: t.protocol,
        host: t.host,
        port: t.port ?? 22,
        username: t.username,
        hostKey:
          t.protocol === 'ssh'
            ? {
                algorithm: 'ssh-ed25519',
                fingerprint: 'SHA256:mockmockmockmockmockmockmockmockmockmockmo',
                firstSeen: true,
              }
            : null,
      },
    });
    this.onEvent({ type: 'state', state: 'connected', error: null });
    this.later(100, () => this.out(this.cli.banner()));
  }

  write(data: string): void {
    if (this.state !== 'connected') return;
    const out = this.cli.input(data);
    if (out) this.later(8, () => this.out(out));
  }

  sendBreak(): void {
    if (this.net.protocol !== 'telnet') fail('unsupported', 'no BREAK');
  }

  close(): void {
    this.timers.forEach(clearTimeout);
    this.state = 'disconnected';
  }

  reconnect(): void {
    if (this.state !== 'disconnected') return;
    this.start();
  }

  onConsoleChange(): void {}

  private later(ms: number, fn: () => void): void {
    this.timers.push(setTimeout(fn, ms));
  }
}

const lives = new Map<RuntimeId, MockLive | MockNetLive>();

// Credential profiles + "keychain" (mock: only whether a password exists).
let profiles: { id: number; name: string; username: string | null; hasPassword: boolean }[] = [
  { id: 1, name: 'NOC admin', username: 'admin', hasPassword: true },
];
let nextProfile = 2;
const profilePw = new Set<number>([1]);
const sessionPw = new Set<number>();
let nextRuntime: RuntimeId = 1;

function live(id: unknown): MockLive | MockNetLive {
  return need(lives.get(id as RuntimeId), `runtime ${String(id)}`);
}

// ------------------------------------------------------------------ public mock surface

type Args = Record<string, unknown> | undefined;

function delay<T>(fn: () => T): Promise<T> {
  return new Promise((resolve, reject) =>
    setTimeout(() => {
      try {
        resolve(fn());
      } catch (e) {
        reject(e);
      }
    }, 15),
  );
}

export function mockInvoke(cmd: string, args: Args): Promise<unknown> {
  const a = args ?? {};
  return delay(() => {
    switch (cmd) {
      case 'folder_list':
        return folders.map((f) => ({ ...f }));
      case 'folder_create': {
        const parentId = (a.parentId as number | null) ?? null;
        if (parentId !== null)
          need(
            folders.find((f) => f.id === parentId),
            'parent',
          );
        const f: Folder = {
          id: nextId++,
          parentId,
          name: validName(a.name),
          sortOrder: folders.length,
        };
        folders.push(f);
        return { ...f };
      }
      case 'folder_rename': {
        const f = need(
          folders.find((x) => x.id === a.id),
          'folder',
        );
        f.name = validName(a.name);
        return undefined;
      }
      case 'folder_move': {
        const f = need(
          folders.find((x) => x.id === a.id),
          'folder',
        );
        const parentId = (a.parentId as number | null) ?? null;
        if (wouldCycle(f.id, parentId)) fail('invalid', 'cycle');
        f.parentId = parentId;
        return undefined;
      }
      case 'folder_reorder': {
        const f = need(
          folders.find((x) => x.id === a.id),
          'folder',
        );
        const parentId = (a.parentId as number | null) ?? null;
        if (wouldCycle(f.id, parentId)) fail('invalid', 'cycle');
        const rest = folders
          .filter((x) => x.parentId === parentId && x.id !== f.id)
          .sort((x, y) => x.sortOrder - y.sortOrder);
        const at = rest.findIndex((x) => x.id === a.beforeId);
        rest.splice(at < 0 ? rest.length : at, 0, f);
        rest.forEach((x, i) => {
          x.parentId = parentId;
          x.sortOrder = i;
        });
        return undefined;
      }
      case 'folder_delete': {
        const f = need(
          folders.find((x) => x.id === a.id),
          'folder',
        );
        folders = folders
          .filter((x) => x.id !== f.id)
          .map((x) => (x.parentId === f.id ? { ...x, parentId: f.parentId } : x));
        sessions = sessions.map((s) => (s.folderId === f.id ? { ...s, folderId: f.parentId } : s));
        return undefined;
      }
      case 'session_list':
        return sessions.map((s) => structuredClone(s));
      case 'session_create': {
        const input = a.input as SessionInput;
        const t = now();
        const s: SessionRecord = {
          ...structuredClone(input),
          ssh: input.ssh ?? null,
          credentialProfileId: input.credentialProfileId ?? null,
          autoCommands: input.autoCommands ?? [],
          name: validName(input.name),
          id: nextId++,
          sortOrder: sessions.length,
          createdAt: t,
          updatedAt: t,
          lastConnectedAt: null,
        };
        sessions.push(s);
        return structuredClone(s);
      }
      case 'session_update': {
        const i = sessions.findIndex((s) => s.id === a.id);
        const old = need(sessions[i], 'session');
        const input = a.input as SessionInput;
        const s = {
          ...old,
          ...structuredClone(input),
          name: validName(input.name),
          updatedAt: now(),
        };
        sessions[i] = s;
        return structuredClone(s);
      }
      case 'session_delete':
        need(
          sessions.find((s) => s.id === a.id),
          'session',
        );
        sessions = sessions.filter((s) => s.id !== a.id);
        return undefined;
      case 'session_bulk_update': {
        const ids = a.ids as number[];
        const patch = a.patch as SessionPatch;
        const found = ids.map((id) =>
          need(
            sessions.find((s) => s.id === id),
            'session',
          ),
        );
        const out = found.map((old) => {
          const next = { ...old, ...applyPatch(old, patch), updatedAt: now() };
          sessions[sessions.indexOf(old)] = next;
          return structuredClone(next);
        });
        return out;
      }
      case 'session_bulk_delete': {
        const ids = a.ids as number[];
        ids.forEach((id) =>
          need(
            sessions.find((s) => s.id === id),
            'session',
          ),
        );
        sessions = sessions.filter((s) => !ids.includes(s.id));
        return ids.length;
      }
      case 'session_duplicate': {
        const old = need(
          sessions.find((s) => s.id === a.id),
          'session',
        );
        const t = now();
        const s = {
          ...structuredClone(old),
          id: nextId++,
          name: uniqueCopyName(old.name),
          createdAt: t,
          updatedAt: t,
          lastConnectedAt: null,
        };
        sessions.push(s);
        return structuredClone(s);
      }
      case 'settings_all':
        return { ...settings };
      case 'setting_set':
        settings = { ...settings, [a.key as string]: a.value };
        saveSettings();
        return undefined;
      case 'console_list':
        return snapshot();
      case 'console_rescan':
        return snapshot();
      case 'live_write':
        live(a.id).write(a.data as string);
        return undefined;
      case 'live_resize':
        live(a.id);
        return undefined;
      case 'live_break':
        live(a.id).sendBreak();
        return undefined;
      case 'live_close':
        live(a.id).close();
        lives.delete(a.id as RuntimeId);
        return undefined;
      case 'live_reconnect':
        live(a.id).reconnect();
        return undefined;
      case 'live_answer':
        live(a.id).answer();
        return undefined;
      case 'known_host_forget':
        return undefined;
      case 'profile_list':
        return profiles.map((p) => ({ ...p, hasPassword: profilePw.has(p.id) }));
      case 'profile_create': {
        const p = {
          id: nextProfile++,
          name: validName(a.name as string),
          username: (a.username as string | null) ?? null,
          hasPassword: false,
        };
        profiles.push(p);
        if (a.password) profilePw.add(p.id);
        return { ...p, hasPassword: profilePw.has(p.id) };
      }
      case 'profile_update': {
        const p = need(
          profiles.find((x) => x.id === a.id),
          'profile',
        );
        p.name = validName(a.name as string);
        p.username = (a.username as string | null) ?? null;
        return { ...p, hasPassword: profilePw.has(p.id) };
      }
      case 'profile_set_password':
        if (a.password) profilePw.add(a.id as number);
        else profilePw.delete(a.id as number);
        return undefined;
      case 'profile_delete':
        profiles = profiles.filter((x) => x.id !== a.id);
        profilePw.delete(a.id as number);
        sessions = sessions.map((s) =>
          s.credentialProfileId === a.id ? { ...s, credentialProfileId: null } : s,
        );
        return undefined;
      case 'import_detect':
        return { puttyCount: 3, secureCrtDir: null, mobaxtermIni: null };
      case 'import_sessions':
        return { created: 3, skipped: 0, errors: [], folder: 'PuTTY' };
      case 'export_sessions':
      case 'vault_export':
        return undefined;
      case 'key_generate':
      case 'key_inspect':
      case 'key_convert':
        return {
          algorithm: 'ssh-ed25519',
          bits: 256,
          comment: 'admin@laptop',
          fingerprint: 'SHA256:mock0fingerprint0000000000000000000000000',
          publicLine: 'ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAImockmockmockmock admin@laptop',
          encrypted: a.passphrase ? true : false,
        };
      case 'vault_import':
        if (a.password !== 'mock-master') throw { code: 'badPassword', detail: null };
        return {
          sessions: { created: 2, skipped: 1, errors: [], folder: null },
          profilesCreated: 1,
          passwords: 2,
          knownHosts: 1,
        };
      case 'pick_path':
        return null;
      case 'session_reorder': {
        const id = a.id as number;
        const folderId = (a.folderId as number | null) ?? null;
        const moved = need(
          sessions.find((s) => s.id === id),
          'session',
        );
        const rest = sessions
          .filter((s) => s.folderId === folderId && s.id !== id)
          .sort((x, y) => x.sortOrder - y.sortOrder);
        const at = rest.findIndex((s) => s.id === a.beforeId);
        rest.splice(at < 0 ? rest.length : at, 0, moved);
        rest.forEach((s, i) => {
          s.folderId = folderId;
          s.sortOrder = i;
        });
        return undefined;
      }
      case 'session_password_set':
        if (a.password) sessionPw.add(a.id as number);
        else sessionPw.delete(a.id as number);
        return undefined;
      case 'session_password_exists':
        return sessionPw.has(a.id as number);
      case 'clipboard_read':
        return navigator.clipboard.readText().catch(() => '');
      case 'clipboard_write':
        return navigator.clipboard.writeText(a.text as string).catch(() => undefined);
      default:
        return fail('unsupported', cmd);
    }
  });
}

export async function liveOpen(
  request: OpenRequest,
  onData: (bytes: Uint8Array) => void,
  onEvent: (event: SessionEvent) => void,
): Promise<RuntimeId> {
  const target = request.target;
  if (target.type === 'serial' && !target.settings.port.trim()) fail('invalid', 'empty port');
  if (target.type === 'network' && !target.host.trim()) fail('invalid', 'empty host');
  const id = nextRuntime++;
  const l =
    target.type === 'serial'
      ? new MockLive(id, request, onData, onEvent)
      : new MockNetLive(id, request, onData, onEvent);
  lives.set(id, l);
  const s = sessions.find((x) => x.id === request.sessionId);
  if (s) s.lastConnectedAt = now();
  setTimeout(() => l.start(), 0);
  return id;
}

export async function onConsoleChanged(cb: (change: ConsoleChange) => void): Promise<() => void> {
  consoleListeners.add(cb);
  return () => consoleListeners.delete(cb);
}

// ------------------------------------------------------------------ demo hooks

const demo = {
  plug(): void {
    if (ports.some((p) => p.key === ciscoConsole.key)) return;
    ports = [...ports, ciscoConsole];
    emitConsoles([ciscoConsole.key], []);
  },
  unplug(): void {
    if (!ports.some((p) => p.key === ftdi.key)) return;
    ports = ports.filter((p) => p.key !== ftdi.key);
    emitConsoles([], [ftdi.key]);
  },
  replug(): void {
    if (ports.some((p) => p.key === ftdi.key)) return;
    ports = [ftdi, ...ports];
    emitConsoles([ftdi.key], []);
  },
  fixDriver(): void {
    hints = [];
    emitConsoles([], []);
  },
};

if (typeof window !== 'undefined') {
  (window as unknown as { kenshellMock: typeof demo }).kenshellMock = demo;
  if (/demo=hotplug/.test(window.location.search)) setTimeout(() => demo.plug(), 3000);
}
