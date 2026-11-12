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
  Note,
  Snippet,
  SnippetInput,
  RuntimeId,
  SessionEvent,
  SessionInput,
  SessionRecord,
  Settings,
} from '../ipc';
import { isMac } from '../ipc';
import { applyPatch, type SessionPatch } from '../bulk';
import { parseIface } from '../iface';
import { framingLabel } from '../format';
import { FakeCli } from './cli';
import { mockDiscovery, mockFlow, mockSnmp } from './topology';

const mockHistory = new Map<string, string[]>();
let mockScanCancel = false;
let mockSftp = [
  {
    name: 'bootflash',
    path: '/bootflash',
    isDir: true,
    isLink: false,
    size: 0,
    modified: 1_762_400_000,
  },
  {
    name: 'startup-config',
    path: '/startup-config',
    isDir: false,
    isLink: false,
    size: 4821,
    modified: 1_762_400_000,
  },
  {
    name: 'cat9k_iosxe.17.09.04a.SPA.bin',
    path: '/cat9k_iosxe.17.09.04a.SPA.bin',
    isDir: false,
    isLink: false,
    size: 1_234_567_890,
    modified: 1_762_300_000,
  },
];
let mockSnippets: Snippet[] = [];
let mockBackups: { id: number; text: string; [k: string]: unknown }[] = [];
let nextSnippetId = 1;
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
    vendor: 'Switch',
    model: 'USB Console',
    vid: 0x05a6,
    pid: 0x0009,
    driverName: null,
    driverUrl: null,
  },
  manufacturer: 'Switch',
  product: 'Switch USB Console',
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
    identify: true,
    deviceId: null,
    sortOrder: id,
    createdAt: now() - 30 * 24 * HOUR,
    updatedAt: now() - 2 * HOUR,
    lastConnectedAt,
  };
}

let notes: Note[] = [];
let nextNoteId = 1;
let sessions: SessionRecord[] = [
  record(1, 1, 'core-sw1', FTDI_PATH, 9600, 'prod', now() - 2 * HOUR),
  record(2, 1, 'fw-edge', MISSING_PATH, 115200, 'prod', now() - 26 * HOUR),
  record(3, 2, 'lab-r2', FTDI_PATH, null, 'lab', now() - 5 * HOUR),
  record(4, 3, 'access-sw3', FTDI_PATH, null, 'test', null),
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
/** The demo device (notes attach to it). */
const MOCK_DEVICE_ID = 1;

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
    if (this.last === '') {
      // What the backend's hidden `show version` would report (demo values).
      this.onEvent({
        type: 'identity',
        hostname: hit.host ?? null,
        vendor: { id: 'cisco-ios', label: 'Type A', family: 'cisco' },
        facts: {
          hostname: hit.host ?? null,
          model: 'WS-C2960X-48FPD-L',
          version: '15.2(7)E8',
          serial: 'FOC1928Z0AB',
          uptime: '2 weeks, 3 days, 4 hours',
        },
        deviceId: MOCK_DEVICE_ID,
      });
    }
    if (key !== this.last) {
      this.last = key;
      this.onEvent({ type: 'mode', mode: hit.mode, sub: hit.sub });
      this.onEvent({
        type: 'identity',
        hostname: hit.host ?? null,
        vendor: null,
        facts: null,
        deviceId: null,
      });
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

  get host(): string {
    return this.request.target.type === 'network' ? this.request.target.host : '';
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
                kex: 'curve25519-sha256',
                cipher: 'chacha20-poly1305@openssh.com',
                mac: 'hmac-sha2-256-etm@openssh.com',
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
let mockSnmpCommunity = false;
let mockFlowState = { running: false, sflow: 0, netflow: 0 };

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
          identify: input.identify ?? true,
          deviceId: null,
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
      case 'device_list':
        return [];
      case 'notes_list': {
        const sid = (a.sessionId as number | null) ?? null;
        const did = (a.deviceId as number | null) ?? null;
        return structuredClone(
          notes
            .filter(
              (n) =>
                (did !== null && n.deviceId === did) ||
                (sid !== null && n.sessionId === sid && n.deviceId === null),
            )
            .sort((x, y) => Number(y.pinned) - Number(x.pinned) || y.createdAt - x.createdAt),
        );
      }
      case 'note_create': {
        const body = String(a.body ?? '').trim();
        if (!body) fail('invalid', 'empty note');
        const sid = (a.sessionId as number | null) ?? null;
        const did = (a.deviceId as number | null) ?? null;
        if (sid === null && did === null) fail('invalid', 'no owner');
        const iface = parseIface(body.split('\n')[0]?.split(':')[0] ?? '');
        const t = now();
        const n: Note = {
          id: nextNoteId++,
          sessionId: sid,
          deviceId: did,
          interfaceKey: body.split('\n')[0]?.includes(':') ? (iface?.key ?? null) : null,
          interfaceShort: body.split('\n')[0]?.includes(':') ? (iface?.short ?? null) : null,
          body,
          pinned: false,
          createdAt: t,
          updatedAt: t,
        };
        notes.push(n);
        return structuredClone(n);
      }
      case 'note_update': {
        const n = need(
          notes.find((x) => x.id === a.id),
          'note',
        );
        n.body = String(a.body ?? '').trim();
        n.updatedAt = now();
        return structuredClone(n);
      }
      case 'note_pin': {
        const n = need(
          notes.find((x) => x.id === a.id),
          'note',
        );
        n.pinned = Boolean(a.pinned);
        return structuredClone(n);
      }
      case 'note_delete':
        need(
          notes.find((x) => x.id === a.id),
          'note',
        );
        notes = notes.filter((x) => x.id !== a.id);
        return undefined;
      case 'notes_search': {
        const q = String(a.text ?? '').toLowerCase();
        return notes
          .filter((n) => n.body.toLowerCase().includes(q))
          .map((n) => ({ sessionId: n.sessionId, deviceId: n.deviceId }));
      }
      case 'notes_export':
        return undefined;
      case 'live_capture':
        return mockCapture(String(a.command));
      case 'backup_take':
      case 'backup_import': {
        const text =
          a.text !== undefined
            ? String(a.text)
            : `hostname core-sw1\ninterface Gi1/0/1\n description ${mockBackups.length % 2 ? 'Uplink' : 'Uplink to core'}\nntp server 10.0.0.1\nend\n`;
        const b = {
          id: mockBackups.length + 1,
          sessionId: (a.sessionId as number | null) ?? null,
          deviceId: (a.deviceId as number | null) ?? null,
          hostname: (a.hostname as string | null) ?? null,
          source: (a.source as string | null) ?? 'file',
          createdAt: Date.now(),
          size: text.length,
          lines: text.split('\n').length - 1,
          sha256: String(mockBackups.length),
          text,
        };
        mockBackups = [b, ...mockBackups];
        return b;
      }
      case 'backups_list':
        return mockBackups;
      case 'backup_text':
        return need(
          mockBackups.find((b) => b.id === a.id),
          'backup',
        ).text;
      case 'backup_delete':
        mockBackups = mockBackups.filter((b) => b.id !== a.id);
        return undefined;
      case 'backup_export':
        return undefined;
      case 'config_file_read':
        return 'hostname from-file\nend\n';
      case 'config_diff':
        return mockDiff(String(a.left), String(a.right));
      case 'config_live_diff':
      case 'backup_diff_live':
        return mockDiff(
          'hostname core-sw1\nntp server 10.0.0.1\nend',
          'hostname core-sw1\nntp server 10.0.0.2\nlogging host 10.0.0.5\nend',
        );
      case 'diag_stop':
        mockDiagStopped.add(Number(a.job));
        return undefined;
      case 'server_stop':
        mockServers.delete(String(a.kind));
        return [...mockServers.values()];
      case 'server_status':
        return [...mockServers.values()];
      case 'sftp_open':
        return { sftp: 1, home: '/' };
      case 'sftp_list':
        return mockSftp
          .filter(
            (e) =>
              e.path.startsWith(String(a.path) === '/' ? '/' : `${String(a.path)}/`) &&
              e.path.split('/').length ===
                (String(a.path) === '/' ? 2 : String(a.path).split('/').length + 1),
          )
          .sort((x, y) => Number(y.isDir) - Number(x.isDir) || x.name.localeCompare(y.name));
      case 'sftp_mkdir':
        mockSftp.push({
          name: String(a.path).split('/').pop() ?? '',
          path: String(a.path),
          isDir: true,
          isLink: false,
          size: 0,
          modified: 1_762_400_000,
        });
        return undefined;
      case 'sftp_remove':
        mockSftp = mockSftp.filter((e) => e.path !== a.path);
        return undefined;
      case 'sftp_rename': {
        const e = mockSftp.find((x) => x.path === a.from);
        if (e) {
          e.path = String(a.to);
          e.name = e.path.split('/').pop() ?? e.name;
        }
        return undefined;
      }
      case 'sftp_close':
      case 'transfer_cancel':
        return undefined;
      case 'netscan_cancel':
        mockScanCancel = true;
        return undefined;
      case 'open_device_web':
        return undefined;
      case 'oui_lookup': {
        const hex = String(a.mac)
          .replace(/[^0-9a-fA-F]/g, '')
          .slice(0, 6)
          .toLowerCase();
        const demo: Record<string, string> = {
          '00000c': 'Cisco Systems',
          '58696c': 'Ruijie Networks',
        };
        return { vendor: demo[hex] ?? null, tableSize: 3 };
      }
      case 'netscan_networks':
        return {
          networks: [
            {
              iface: 'Wi-Fi',
              ip: '192.168.1.34',
              prefix: 24,
              network: '192.168.1.0',
              cidr: '192.168.1.0/24',
              defaultTarget: '192.168.1.0/24',
            },
            {
              iface: 'Ethernet',
              ip: '10.20.0.5',
              prefix: 16,
              network: '10.20.0.0',
              cidr: '10.20.0.0/16',
              defaultTarget: '10.20.0.0/24',
            },
          ],
          defaultTarget: '192.168.1.0/24',
          maxAddresses: 1024,
          concurrency: 64,
          ports: [22, 23, 80, 443, 3389],
          ouiSize: 3,
        };
      case 'netscan_check': {
        const target = String(a.target).trim();
        if (/^192\.168\.1\.(0\/24|\d+(-\d+)?)$/.test(target))
          return {
            range: { first: '192.168.1.1', last: '192.168.1.254' },
            count: 254,
            iface: 'Wi-Fi',
          };
        if (/^10\.20\.\d+\.0\/2[2-4]$/.test(target))
          return {
            range: { first: '10.20.0.1', last: '10.20.0.254' },
            count: 254,
            iface: 'Ethernet',
          };
        throw {
          reason: /^(10|172|192\.168|169\.254)\./.test(target)
            ? /\/(1\d|2[01])$/.test(target)
              ? 'tooLarge'
              : 'notConnected'
            : /^\d/.test(target)
              ? 'notPrivate'
              : 'invalid',
        };
      }
      case 'snmp_community_status':
        return mockSnmpCommunity;
      case 'snmp_community_set':
        mockSnmpCommunity = true;
        return undefined;
      case 'snmp_community_clear':
        mockSnmpCommunity = false;
        return undefined;
      case 'topology_snmp': {
        const d = mockSnmp(String(a.host));
        return new Promise((r, reject) =>
          setTimeout(
            () =>
              String(a.host).includes('down')
                ? reject(Object.assign(new Error('timeout'), { code: 'timeout', detail: null }))
                : r(d),
            400,
          ),
        );
      }
      case 'flow_start':
        mockFlowState = {
          running: true,
          sflow: Number(a.sflowPort),
          netflow: Number(a.netflowPort),
        };
        return mockFlow(true, mockFlowState.sflow, mockFlowState.netflow);
      case 'flow_stop':
        mockFlowState = { ...mockFlowState, running: false };
        return mockFlow(false, 0, 0);
      case 'flow_snapshot':
        return mockFlow(mockFlowState.running, mockFlowState.sflow, mockFlowState.netflow);
      case 'topology_discover': {
        const l = lives.get(a.id as RuntimeId);
        const d = mockDiscovery(l instanceof MockNetLive ? l.host : 'core-sw1', a.hosts === true);
        return new Promise((r) => setTimeout(() => r(d), 500));
      }
      case 'neighbors_discover':
        return [
          {
            name: 'core-sw1.lab.example.com',
            ip: '10.0.0.1',
            platform: 'CS-3850-48P',
            capability: 'Router Switch IGMP',
            localInterface: 'GigabitEthernet1/0/24',
            remoteInterface: 'GigabitEthernet1/0/1',
            cdp: true,
            lldp: true,
            chassis: null,
            vendor: null,
          },
          {
            name: 'ap-floor2',
            ip: '10.20.0.15',
            platform: 'AP-2802I',
            capability: 'Trans-Bridge',
            localInterface: 'GigabitEthernet1/0/5',
            remoteInterface: 'GigabitEthernet0',
            cdp: true,
            lldp: false,
            chassis: null,
            vendor: null,
          },
          {
            name: 'access-sw3',
            ip: '10.0.0.20',
            platform: 'S5750-48GT4XS',
            capability: 'Bridge',
            localInterface: 'GigabitEthernet1/0/47',
            remoteInterface: 'GigabitEthernet 0/49',
            cdp: false,
            lldp: true,
            chassis: null,
            vendor: null,
          },
          {
            name: 'printer-3f',
            ip: null,
            platform: null,
            capability: 'Station',
            localInterface: 'GigabitEthernet1/0/10',
            remoteInterface: '0011.2233.4455',
            cdp: false,
            lldp: true,
            chassis: null,
            vendor: null,
          },
        ];
      case 'logfile_analyze':
        return {
          kind: 'showTech',
          lines: 48211,
          bytes: 2_310_442,
          sections: [
            { title: 'show version', start: 1, end: 40 },
            { title: 'show logging', start: 41, end: 920 },
            { title: 'show interfaces', start: 921, end: 48211 },
          ],
          facts: [
            ['hostname', 'core-sw1'],
            ['model', 'C9300-48P'],
            ['version', '17.09.04a'],
          ],
          messages: [
            { mnemonic: 'SYS-2-MALLOCFAIL', severity: 2, count: 1, firstLine: 77 },
            { mnemonic: 'LINK-3-UPDOWN', severity: 3, count: 42, firstLine: 60 },
            { mnemonic: 'SYS-5-CONFIG_I', severity: 5, count: 9, firstLine: 44 },
          ],
          findings: [
            {
              severity: 'critical',
              code: 'fileTraceback',
              params: { count: '2', line: '78', sample: '-Traceback= 1A2B3C 4D5E6F' },
              command: '',
            },
            { severity: 'critical', code: 'fileMallocFail', params: { count: '1' }, command: '' },
            { severity: 'warning', code: 'fileLinkFlaps', params: { count: '42' }, command: '' },
          ],
        };
      case 'logfile_lines':
        return `------------------ show logging ------------------\n*Nov  6 10:00: %LINK-3-UPDOWN: Interface Gi1/0/2, changed state to down\n`;
      case 'config_audit':
        return [
          {
            severity: 'critical',
            code: 'telnetEnabled',
            params: { block: 'line vty 0 4' },
            command: 'show running-config',
          },
          { severity: 'warning', code: 'noNtp', params: {}, command: 'show running-config' },
          {
            severity: 'info',
            code: 'portsWithoutDescription',
            params: { count: '2', list: 'Gi1/0/2, Gi1/0/7' },
            command: 'show running-config',
          },
        ];
      case 'diagnostics_commands':
        return [
          'show version',
          'show environment all',
          'show processes cpu sorted',
          'show interfaces',
          'show interfaces status err-disabled',
          'show logging',
        ];
      case 'diagnostics_evaluate':
        return [
          {
            severity: 'critical',
            code: 'crcErrors',
            params: { iface: 'GigabitEthernet1/0/5', count: '12403' },
            command: 'show interfaces',
          },
          {
            severity: 'critical',
            code: 'errDisabled',
            params: { iface: 'Gi1/0/3', reason: 'bpduguard' },
            command: 'show interfaces status err-disabled',
          },
          {
            severity: 'warning',
            code: 'logErrors',
            params: {
              count: '3',
              sample: '%LINK-3-UPDOWN: Interface Gi1/0/2, changed state to down',
            },
            command: 'show logging',
          },
          {
            severity: 'info',
            code: 'commandUnsupported',
            params: { command: 'show environment all' },
            command: 'show environment all',
          },
        ];
      case 'analyzer_parsers':
        return [
          {
            id: 'cisco.show_ip_interface_brief',
            command: 'show ip interface brief',
            aliases: [],
            families: ['cisco', 'ruijie'],
          },
        ];
      case 'analyze_output':
        return mockAnalyze(String(a.text), (a.command as string | null) ?? null);
      case 'analyzer_export_csv':
        return undefined;
      case 'snippets_list':
        return mockSnippets;
      case 'snippet_create': {
        const i = a.input as SnippetInput;
        const s: Snippet = {
          ...i,
          id: nextSnippetId++,
          createdAt: Date.now(),
          updatedAt: Date.now(),
        };
        mockSnippets = [...mockSnippets, s];
        return s;
      }
      case 'snippet_update': {
        const i = a.input as SnippetInput;
        const cur = need(
          mockSnippets.find((x) => x.id === a.id),
          'snippet',
        );
        const s: Snippet = { ...cur, ...i, updatedAt: Date.now() };
        mockSnippets = mockSnippets.map((x) => (x.id === s.id ? s : x));
        return s;
      }
      case 'snippet_delete':
        mockSnippets = mockSnippets.filter((x) => x.id !== a.id);
        return undefined;
      case 'history_list':
        return mockHistory.get(`${a.sessionId ?? '-'}:${a.deviceId ?? '-'}`) ?? [];
      case 'history_add': {
        const k = `${a.sessionId ?? '-'}:${a.deviceId ?? '-'}`;
        const c = String(a.command);
        mockHistory.set(k, [c, ...(mockHistory.get(k) ?? []).filter((x) => x !== c)]);
        return undefined;
      }
      case 'history_clear':
        mockHistory.delete(`${a.sessionId ?? '-'}:${a.deviceId ?? '-'}`);
        return undefined;
      case 'notes_pinned':
        return notes
          .filter((n) => n.pinned)
          .map((n) => ({
            sessionId: n.sessionId,
            deviceId: n.deviceId,
            text: n.body.split('\n')[0] ?? '',
          }));
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
      case 'live_record_start':
        return `/demo/logs/${String(a.name)}_2026-11-06_101500.cast`;
      case 'live_record_stop':
        return `/demo/logs/recording.cast`;
      case 'recording_read':
        return mockCast();
      case 'serial_access':
        return { ok: true, group: null };
      case 'update_status':
        return { configured: false, current: '0.1.0' };
      case 'update_check':
        throw { code: 'unsupported', message: 'updates are not configured in this build' };
      case 'ai_key_status':
        return mockAiKey !== null;
      case 'ai_key_set':
        mockAiKey = String(a.key);
        return undefined;
      case 'ai_key_clear':
        mockAiKey = null;
        return undefined;
      case 'ai_key_get':
        return mockAiKey;
      case 'text_export':
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
        // Demo: the file-analysis picker (its list includes .tech) gets a sample show tech.
        if (a.kind === 'folder') return '/demo/tftp';
        return a.kind === 'file' && (a.extensions as string[] | null)?.includes('tech')
          ? '/demo/core-sw1-show-tech.txt'
          : null;
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

/** Browser demo of the Rust `show ip interface brief` parser (layout of the fake CLI). */
function mockAnalyze(text: string, command: string | null) {
  const lines = text.split(/\r?\n/);
  const cmd =
    command ??
    lines
      .find((l) => l.trim())
      ?.replace(/^.*?[#>]/, '')
      .trim() ??
    '';
  const words = cmd.split(/\s+/);
  const canon = ['show', 'ip', 'interface', 'brief'];
  if (
    words.length !== 4 ||
    !words.every((w, i) => (canon[i] as string).startsWith(w.toLowerCase()))
  )
    return null;
  const rows = lines
    .filter((l) => /^[A-Za-z]+\S*\d/.test(l) && !/[#>]\s*$/.test(l) && !/[#>]\S/.test(l))
    .map((l) => l.trim().split(/\s+/))
    .filter((w) => w.length >= 6)
    .map((w) => [
      w[0] ?? '',
      w[1] ?? '',
      '',
      w[2] ?? '',
      w[3] ?? '',
      w.slice(4, -1).join(' '),
      w[w.length - 1] ?? '',
    ]);
  return {
    parser: 'cisco.show_ip_interface_brief',
    command: 'show ip interface brief',
    columns: [
      { key: 'interface', kind: 'iface' },
      { key: 'ip', kind: 'ip' },
      { key: 'secondary', kind: 'ip' },
      { key: 'ok', kind: 'text' },
      { key: 'method', kind: 'text' },
      { key: 'status', kind: 'status' },
      { key: 'protocol', kind: 'status' },
    ],
    rows,
    truncated: false,
  };
}

/** Browser demo output for hidden captures (diagnostics). */
async function mockCapture(command: string): Promise<string> {
  await new Promise((r) => setTimeout(r, 250));
  if (command.startsWith('show interfaces status'))
    return `${command}\nPort      Name               Status       Reason\nGi1/0/3   Printer            err-disabled bpduguard\n`;
  if (command === 'show interfaces')
    return `${command}\nGigabitEthernet1/0/5 is up, line protocol is up\n     12403 input errors, 12403 CRC, 0 frame\n`;
  if (/^sh\S* ip int\S* br/i.test(command))
    return `${command}\nInterface              IP-Address      OK? Method Status                Protocol\nVlan1                  unassigned      YES NVRAM  administratively down down\nVlan10                 10.0.10.${Math.floor(Math.random() * 200) + 2}       YES NVRAM  up                    up\nGigabitEthernet1/0/1   unassigned      YES unset  up                    up\n`;
  if (command === 'show environment all')
    return `${command}\n% Invalid input detected at '^' marker.\n`;
  return `${command}\n(demo output)\n`;
}

/** Browser demo diff: same / changed by position (the real one is the Rust LCS diff). */
function mockDiff(left: string, right: string) {
  const a = left.split('\n');
  const b = right.split('\n');
  const rows = [];
  let added = 0;
  let removed = 0;
  let changed = 0;
  for (let i = 0; i < Math.max(a.length, b.length); i++) {
    const l = a[i] ?? null;
    const r = b[i] ?? null;
    const kind = l === r ? 'same' : l === null ? 'added' : r === null ? 'removed' : 'changed';
    if (kind === 'added') added++;
    else if (kind === 'removed') removed++;
    else if (kind === 'changed') changed++;
    rows.push({
      kind,
      leftNo: l === null ? null : i + 1,
      left: l,
      rightNo: r === null ? null : i + 1,
      right: r,
    });
  }
  return { rows, added, removed, changed, coarse: false };
}

/** Browser demo scan of 192.168.1.0/24: a handful of devices, progress over ~1.5 s. */
export async function netscanRun(
  target: string,
  onProgress: (p: { stage: 'sweep' | 'arp' | 'details'; done: number; total: number }) => void,
) {
  mockScanCancel = false;
  const base = target.startsWith('10.20.') ? '10.20.0.' : '192.168.1.';
  const total = 254;
  for (let done = 0; done <= total; done += 32) {
    if (mockScanCancel) break;
    onProgress({ stage: 'sweep', done: Math.min(done, total), total });
    await new Promise((r) => setTimeout(r, 120));
  }
  onProgress({ stage: 'details', done: 1, total: 1 });
  const devices: Record<number, { ports: number[]; name?: string; mac: string }> = {
    1: { ports: [80, 443], name: 'gateway.lan', mac: '58:69:6c:00:00:01' },
    2: { ports: [22, 443], name: 'core-sw1.lan', mac: '00:1a:2b:03:04:05' },
    3: { ports: [22, 23], name: 'access-sw3.lan', mac: '58:69:6c:00:11:11' },
    10: { ports: [80], name: 'printer-3f.lan', mac: '00:00:48:aa:bb:cc' },
    11: { ports: [], mac: 'f0:18:98:12:34:56' },
    34: { ports: [], name: 'my-laptop.lan', mac: 'a4:83:e7:00:00:34' },
    90: { ports: [3389], name: 'win-pc', mac: '00:15:5d:01:02:03' },
    200: { ports: [443], mac: 'b8:27:eb:11:22:33' },
  };
  return {
    hosts: Array.from({ length: total }, (_, i) => {
      const n = i + 1;
      const d = devices[n];
      return {
        vendor: d
          ? ((
              {
                1: 'Ruijie Networks',
                2: 'Cisco Systems',
                3: 'Ruijie Networks',
                11: 'Apple',
              } as Record<number, string>
            )[n] ?? null)
          : null,
        ip: `${base}${n}`,
        alive: Boolean(d),
        icmp: Boolean(d) && n !== 90,
        openPorts: d?.ports ?? [],
        mac: d?.mac ?? null,
        hostname: d?.name ?? null,
        own: n === 34,
        gateway: n === 1,
      };
    }),
    icmpAvailable: true,
    cancelled: mockScanCancel,
  };
}

/** Browser demo transfer: 1 MB over ~1.2 s. */
export async function mockTransfer(onProgress: (p: { done: number; total: number }) => void) {
  const total = 1_048_576;
  for (let done = 0; done <= total; done += 131_072) {
    onProgress({ done, total });
    await new Promise((r) => setTimeout(r, 150));
  }
  return total;
}

const mockServers = new Map<string, unknown>();

/** Browser demo file server: one switch fetches an image, another uploads its config. */
export async function mockServerStart(
  opts: {
    kind: string;
    root: string;
    bindIp: string;
    port: number;
    allowUpload: boolean;
    user: string | null;
    password: string | null;
  },
  onEvent: (e: unknown) => void,
) {
  setTimeout(
    () =>
      onEvent({
        kind: 'started',
        id: 1,
        peer: '192.168.1.2:50123',
        file: 'cat9k_iosxe.bin',
        upload: false,
        size: 4_000_000,
      }),
    600,
  );
  setTimeout(() => onEvent({ kind: 'progress', id: 1, bytes: 1_800_000 }), 1200);
  setTimeout(() => onEvent({ kind: 'done', id: 1, bytes: 4_000_000 }), 2000);
  setTimeout(
    () =>
      onEvent({
        kind: 'started',
        id: 2,
        peer: '192.168.1.3:61000',
        file: 'sw2-confg',
        upload: true,
        size: null,
      }),
    1500,
  );
  setTimeout(() => onEvent({ kind: 'done', id: 2, bytes: 6_210 }), 1900);
  const login = opts.kind !== 'tftp';
  mockServers.set(opts.kind, {
    kind: opts.kind,
    addr: `${opts.bindIp}:${opts.port}`,
    root: opts.root,
    allowUpload: opts.allowUpload,
    user: login ? opts.user || 'kenshell' : null,
    password: login ? opts.password || 'q7Hk2mPz9xRa' : null,
    fingerprints: opts.kind === 'scp' ? ['SHA256:Xq4m0demoFingerprintN2b6x0p3hW1sYtE9kQ'] : [],
  });
  const order = ['tftp', 'ftp', 'scp'];
  return [...mockServers.values()].sort(
    (x, y) =>
      order.indexOf((x as { kind: string }).kind) - order.indexOf((y as { kind: string }).kind),
  );
}

const mockDiagStopped = new Set<number>();

/** Browser demo ping / traceroute output. */
export async function mockDiagRun(
  job: number,
  kind: string,
  target: string,
  count: number | null,
  onLine: (line: string) => void,
): Promise<number | null> {
  const lines =
    kind === 'ping'
      ? [
          `PING ${target} (${target}): 56 data bytes`,
          ...Array.from(
            { length: count ?? 20 },
            (_, i) =>
              `64 bytes from ${target}: icmp_seq=${i} ttl=64 time=${(1.2 + ((i * 7) % 5) * 0.31).toFixed(3)} ms`,
          ),
        ]
      : [
          `traceroute to ${target} (${target}), 30 hops max, 40 byte packets`,
          ' 1  192.168.1.1  0.612 ms  0.455 ms  0.430 ms',
          ' 2  * * *',
          ' 3  100.64.0.1  8.101 ms  7.912 ms  8.004 ms',
          ` 4  ${target}  12.700 ms  12.311 ms  12.205 ms`,
        ];
  for (const l of lines) {
    if (mockDiagStopped.has(job)) return null;
    onLine(l);
    await new Promise((r) => setTimeout(r, kind === 'ping' ? 400 : 250));
  }
  return 0;
}

/** Browser demo recording (asciicast v2). */
function mockCast(): string {
  const ev: [number, string, string][] = [
    [0.3, 'o', 'core-sw1#'],
    [1.0, 'o', 'show ip interface brief\r\n'],
    [
      1.4,
      'o',
      'Interface              IP-Address      OK? Method Status                Protocol\r\n',
    ],
    [
      1.5,
      'o',
      'Vlan1                  10.0.0.1        YES NVRAM  \u001b[32mup\u001b[0m                    up\r\n',
    ],
    [
      1.6,
      'o',
      'GigabitEthernet1/0/1   unassigned      YES unset  \u001b[31mdown\u001b[0m                  down\r\n',
    ],
    [9.0, 'o', 'core-sw1#'],
  ];
  return [
    JSON.stringify({
      version: 2,
      width: 100,
      height: 24,
      timestamp: 1793923200,
      title: 'core-sw1',
    }),
    ...ev.map((e) => JSON.stringify(e)),
  ].join('\n');
}

let mockAiKey: string | null = null;
