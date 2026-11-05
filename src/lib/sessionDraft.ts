/** Session editor form model (pure): record ⇄ draft, validation. */
import {
  DEFAULT_PORTS,
  DEFAULT_SERIAL,
  DEFAULT_SSH,
  type FlowControl,
  type Parity,
  type Protocol,
  type SessionInput,
  type SshOptions,
} from './ipc';
import { formatForwards, formatJump, parseForwards, parseHostPort, parseJump } from './sshTunnel';
import { isValidBaud, MAX_BAUD, MIN_BAUD } from './quickConnect';
import { envKind, normalizeEnv } from './tree';

export const BAUD_PRESETS = [9600, 19200, 38400, 57600, 115200] as const;
export const DATA_BITS = [5, 6, 7, 8] as const;
export const STOP_BITS = [1, 2] as const;
export const ENV_CHOICES = ['none', 'prod', 'lab', 'test', 'custom'] as const;
export const MAX_NAME = 128;
export const MAX_ENV = 12;

export type BaudChoice = 'auto' | `${(typeof BAUD_PRESETS)[number]}` | 'custom';
export type EnvChoice = (typeof ENV_CHOICES)[number];
export type LineChoice = 'default' | 'on' | 'off';
export const MAX_KEEPALIVE = 3600;

const lineChoice = (v: boolean | null | undefined): LineChoice =>
  v === true ? 'on' : v === false ? 'off' : 'default';
const lineValue = (c: LineChoice): boolean | null => (c === 'default' ? null : c === 'on');

export interface Draft {
  name: string;
  folderId: number | null;
  protocol: Protocol;
  port: string;
  baudChoice: BaudChoice;
  customBaud: string;
  dataBits: number;
  parity: Parity;
  stopBits: number;
  flowControl: FlowControl;
  /** Line level after opening the port ('default' = leave as the driver sets it). */
  dtr: LineChoice;
  rts: LineChoice;
  envChoice: EnvChoice;
  envCustom: string;
  /** Network protocols. */
  host: string;
  netPort: string;
  username: string;
  legacyAlgorithms: boolean;
  keyPath: string;
  useAgent: boolean;
  /** Seconds as typed; empty = default (30), 0 = off. */
  keepalive: string;
  /** "user@host:port" (user and port optional). */
  jump: string;
  proxyKind: 'none' | 'socks5' | 'http';
  /** "host:port". */
  proxy: string;
  /** One per line: "L 8080:host:80", "R 9000:host:22", "D 1080". */
  forwards: string;
  credentialProfileId: number | null;
  /** Comma separated. */
  tags: string;
  /** One command per line. */
  autoCommands: string;
  /** Background identity command after login. */
  identify: boolean;
}

export function draftFrom(input: Partial<SessionInput> | null | undefined): Draft {
  const serial = { ...DEFAULT_SERIAL, port: '', ...(input?.serial ?? {}) };
  const baud = serial.baud;
  const preset = BAUD_PRESETS.find((b) => b === baud);
  const kind = envKind(input?.envTag);
  return {
    name: input?.name ?? '',
    folderId: input?.folderId ?? null,
    protocol: input?.protocol ?? 'serial',
    port: serial.port,
    baudChoice: baud === null ? 'auto' : preset ? (String(preset) as BaudChoice) : 'custom',
    customBaud: baud !== null && !preset ? String(baud) : '',
    dataBits: serial.dataBits,
    parity: serial.parity,
    stopBits: serial.stopBits,
    flowControl: serial.flowControl,
    dtr: lineChoice(serial.dtr),
    rts: lineChoice(serial.rts),
    envChoice: kind === null ? 'none' : kind,
    envCustom: kind === 'custom' ? (normalizeEnv(input?.envTag) ?? '') : '',
    host: input?.host ?? '',
    netPort: input?.port != null ? String(input.port) : '',
    username: input?.username ?? '',
    legacyAlgorithms: input?.ssh?.legacyAlgorithms ?? false,
    keyPath: input?.ssh?.keyPath ?? '',
    useAgent: input?.ssh?.useAgent ?? false,
    keepalive: input?.ssh?.keepaliveSecs != null ? String(input.ssh.keepaliveSecs) : '',
    jump: formatJump(input?.ssh?.jump),
    proxyKind: input?.ssh?.proxy?.kind ?? 'none',
    proxy: input?.ssh?.proxy ? formatHostPort(input.ssh.proxy.host, input.ssh.proxy.port) : '',
    forwards: formatForwards(input?.ssh?.forwards),
    credentialProfileId: input?.credentialProfileId ?? null,
    tags: (input?.tags ?? []).join(', '),
    autoCommands: (input?.autoCommands ?? []).join('\n'),
    identify: input?.identify ?? true,
  };
}

const formatHostPort = (host: string, port: number): string =>
  `${host.includes(':') ? `[${host}]` : host}:${port}`;

export type DraftErrors = Partial<
  Record<
    | 'name'
    | 'port'
    | 'baud'
    | 'env'
    | 'host'
    | 'netPort'
    | 'jump'
    | 'proxy'
    | 'forwards'
    | 'keepalive',
    'required' | 'range' | 'tooLong' | 'format'
  >
>;

/** 1-based bad forward lines (empty when the list is too long), or null when valid. */
export function forwardErrors(d: Draft): number[] | null {
  const r = parseForwards(d.forwards);
  return Array.isArray(r) ? null : r.badLines;
}

export function draftBaud(d: Draft): number | null {
  if (d.baudChoice === 'auto') return null;
  if (d.baudChoice === 'custom') return Number(d.customBaud.trim());
  return Number(d.baudChoice);
}

export function validateDraft(d: Draft): DraftErrors {
  const errors: DraftErrors = {};
  const name = d.name.trim();
  if (!name) errors.name = 'required';
  else if (name.length > MAX_NAME) errors.name = 'tooLong';
  if (d.protocol === 'serial' && !d.port.trim()) errors.port = 'required';
  if (d.protocol !== 'serial') {
    const host = d.host.trim();
    if (!host || /\s/.test(host)) errors.host = 'required';
    else if (host.length > 255) errors.host = 'tooLong';
    const raw = d.netPort.trim();
    if (raw === '') {
      if (DEFAULT_PORTS[d.protocol] === null) errors.netPort = 'required';
    } else if (!/^\d+$/.test(raw) || Number(raw) < 1 || Number(raw) > 65535) {
      errors.netPort = 'range';
    }
  }
  if (d.protocol === 'ssh') {
    if (parseJump(d.jump) === 'invalid') errors.jump = 'format';
    if (d.proxyKind !== 'none' && !parseHostPort(d.proxy, true)) errors.proxy = 'format';
    if (forwardErrors(d)) errors.forwards = 'format';
    const k = d.keepalive.trim();
    if (k !== '' && (!/^\d+$/.test(k) || Number(k) > MAX_KEEPALIVE)) errors.keepalive = 'range';
  }
  if (d.baudChoice === 'custom') {
    const raw = d.customBaud.trim();
    if (!/^\d+$/.test(raw) || !isValidBaud(Number(raw))) errors.baud = 'range';
  }
  if (d.envChoice === 'custom') {
    const env = d.envCustom.trim();
    if (!env) errors.env = 'required';
    else if (env.length > MAX_ENV) errors.env = 'tooLong';
  }
  return errors;
}

export const hasErrors = (e: DraftErrors): boolean => Object.keys(e).length > 0;

/** Builds the backend input; `base` keeps fields the form does not edit (tags, favorite …). */
export function draftToInput(d: Draft, base?: Partial<SessionInput>): SessionInput {
  const env =
    d.envChoice === 'none' ? null : d.envChoice === 'custom' ? d.envCustom.trim() : d.envChoice;
  return {
    folderId: d.folderId,
    name: d.name.trim(),
    protocol: d.protocol,
    host: d.protocol === 'serial' ? null : d.host.trim(),
    port: d.protocol === 'serial' || d.netPort.trim() === '' ? null : Number(d.netPort.trim()),
    username: d.protocol === 'serial' || !d.username.trim() ? null : d.username.trim(),
    serial:
      d.protocol === 'serial'
        ? {
            port: d.port.trim(),
            baud: draftBaud(d),
            dataBits: d.dataBits,
            parity: d.parity,
            stopBits: d.stopBits,
            flowControl: d.flowControl,
            dtr: lineValue(d.dtr),
            rts: lineValue(d.rts),
          }
        : null,
    envTag: env,
    tags: d.tags
      .split(',')
      .map((x) => x.trim())
      .filter(Boolean),
    favorite: base?.favorite ?? false,
    credentialProfileId: d.protocol === 'ssh' ? d.credentialProfileId : null,
    identify: d.identify,
    autoCommands: d.autoCommands
      .split(/\r?\n/)
      .map((c) => c.trim())
      .filter(Boolean),
    ssh:
      d.protocol === 'ssh'
        ? {
            ...DEFAULT_SSH,
            ...(base?.ssh ?? {}),
            legacyAlgorithms: d.legacyAlgorithms,
            keyPath: d.keyPath.trim() || null,
            useAgent: d.useAgent,
            keepaliveSecs: d.keepalive.trim() === '' ? null : Number(d.keepalive.trim()),
            ...tunnels(d),
          }
        : null,
  };
}

/** Jump / proxy / forwards of a valid draft (malformed parts are dropped). */
function tunnels(d: Draft): Pick<SshOptions, 'jump' | 'proxy' | 'forwards'> {
  const jump = parseJump(d.jump);
  const proxy = d.proxyKind === 'none' ? null : parseHostPort(d.proxy, true);
  const forwards = parseForwards(d.forwards);
  return {
    jump: jump === 'invalid' ? null : jump,
    proxy:
      proxy && proxy.port !== null && d.proxyKind !== 'none'
        ? { kind: d.proxyKind, host: proxy.host, port: proxy.port }
        : null,
    forwards: Array.isArray(forwards) ? forwards : [],
  };
}

export { MAX_BAUD, MIN_BAUD };
