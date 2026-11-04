import type { NetProtocol } from './ipc';
/**
 * Quick connect parser (pure). Turns what the user typed into an intent:
 *   "COM4", "com4 9600", "/dev/cu.usbserial-XYZ", "cu.usbserial-XYZ 115200" → serial open
 *   a saved session name                                                 → open that session
 *   "ssh admin@10.0.0.1", "telnet 10.0.0.5 2001", "10.0.0.1"             → not supported yet
 */

export const MIN_BAUD = 300;
export const MAX_BAUD = 921_600;

export interface NamedSession {
  id: number;
  name: string;
}

export type QuickConnect =
  | { kind: 'empty' }
  | { kind: 'session'; session: NamedSession }
  | { kind: 'serial'; port: string; baud: number | null }
  | {
      kind: 'network';
      protocol: NetProtocol;
      host: string;
      port: number | null;
      username: string | null;
    }
  | { kind: 'invalidPort'; value: string }
  | { kind: 'invalidBaud'; port: string; value: string }
  | { kind: 'noMatch'; text: string };

const WINDOWS_COM = /^com(\d{1,3})$/i;
// Bare macOS / Linux device names typed without /dev/.
const BARE_DEVICE = /^(cu|tty)\.[\w.-]+$|^tty(USB|ACM|S|AMA)\d+$/;
const IPV4 = /^\d{1,3}(\.\d{1,3}){3}(:\d{1,5})?$/;
const FQDN = /^[a-z0-9-]+(\.[a-z0-9-]+)+$/i;

/** Valid serial speed in the supported range. */
export function isValidBaud(n: number): boolean {
  return Number.isInteger(n) && n >= MIN_BAUD && n <= MAX_BAUD;
}

/** Normalizes a serial port token, or returns null when it does not look like one. */
export function normalizePort(token: string): string | null {
  const com = WINDOWS_COM.exec(token);
  if (com) return `COM${Number(com[1])}`;
  if (token.startsWith('/dev/') && token.length > 5) return token;
  if (BARE_DEVICE.test(token)) return `/dev/${token}`;
  return null;
}

function parseBaud(token: string): number | 'auto' | null {
  if (/^auto$/i.test(token)) return 'auto';
  if (!/^\d+$/.test(token)) return null;
  return Number(token);
}

const SCHEMES: Record<string, NetProtocol> = {
  ssh: 'ssh',
  telnet: 'telnet',
  raw: 'raw',
  rlogin: 'rlogin',
};

interface HostPart {
  host: string;
  port: string | null;
  username: string | null;
}

/** `[user@]host[:port]`, `[v6]:port`, bare IPv6. Returns null if it does not look like a host. */
function splitHost(token: string): HostPart | null {
  let rest = token;
  let username: string | null = null;
  const at = rest.lastIndexOf('@');
  if (at > 0) {
    username = rest.slice(0, at);
    rest = rest.slice(at + 1);
  }
  let host = rest;
  let port: string | null = null;
  const v6 = /^\[([^\]]+)\](?::(\d+))?$/.exec(rest);
  if (v6) {
    host = v6[1] as string;
    port = v6[2] ?? null;
  } else if ((rest.match(/:/g) ?? []).length === 1) {
    const i = rest.indexOf(':');
    host = rest.slice(0, i);
    port = rest.slice(i + 1);
  }
  if (!host || /\s/.test(host)) return null;
  const looksLike =
    IPV4.test(`${host}`) || FQDN.test(host) || /^[a-z0-9][\w-]*$/i.test(host) || host.includes(':');
  return looksLike ? { host, port, username } : null;
}

function networkIntent(tokens: string[]): QuickConnect | null {
  let first = (tokens[0] ?? '').toLowerCase();
  let protocol: NetProtocol | null = null;
  let rest = tokens;
  const scheme = /^(ssh|telnet|raw|rlogin):\/\/(.+)$/i.exec(tokens[0] ?? '');
  if (scheme) {
    protocol = SCHEMES[(scheme[1] as string).toLowerCase()] ?? null;
    rest = [scheme[2] as string, ...tokens.slice(1)];
  } else if (first in SCHEMES && tokens.length > 1) {
    protocol = SCHEMES[first] ?? null;
    rest = tokens.slice(1);
  }
  if (rest.length === 0 || rest.length > 2) return null;
  first = rest[0] as string;
  const part = splitHost(first);
  if (!part) return null;
  let portText = part.port;
  if (rest.length === 2) {
    if (portText !== null || !/^\d{1,5}$/.test(rest[1] as string)) return null;
    portText = rest[1] as string;
  }
  if (!protocol) {
    // Bare forms: "10.0.0.5 2001" = telnet (reverse-telnet ports), host / user@host = SSH.
    const explicitHost = IPV4.test(part.host) || FQDN.test(part.host) || part.username !== null;
    if (!explicitHost && part.port === null) return null;
    protocol = rest.length === 2 ? 'telnet' : 'ssh';
  }
  let port: number | null = null;
  if (portText !== null) {
    const n = Number(portText);
    if (!/^\d{1,5}$/.test(portText) || n < 1 || n > 65535)
      return { kind: 'invalidPort', value: portText };
    port = n;
  } else if (protocol === 'raw') {
    return { kind: 'invalidPort', value: '' };
  }
  return { kind: 'network', protocol, host: part.host, port, username: part.username };
}

export function parseQuickConnect(input: string, sessions: readonly NamedSession[]): QuickConnect {
  const text = input.trim();
  if (text === '') return { kind: 'empty' };

  // An exact saved session name always wins (case-insensitive).
  const lower = text.toLowerCase();
  const session = sessions.find((s) => s.name.trim().toLowerCase() === lower);
  if (session) return { kind: 'session', session };

  let tokens = text.split(/\s+/);
  if (tokens[0]?.toLowerCase() === 'serial' && tokens.length > 1) tokens = tokens.slice(1);

  // "COM4@9600" / "COM4:9600" shorthand.
  if (tokens.length === 1) {
    const m = /^([^@\s]+?)[@,:](\w+)$/.exec(tokens[0] ?? '');
    if (m && normalizePort(m[1] ?? '')) tokens = [m[1] as string, m[2] as string];
  }

  const port = normalizePort(tokens[0] ?? '');
  if (port && tokens.length <= 2) {
    if (tokens.length === 1) return { kind: 'serial', port, baud: null };
    const raw = tokens[1] as string;
    const baud = parseBaud(raw);
    if (baud === 'auto') return { kind: 'serial', port, baud: null };
    if (baud !== null && isValidBaud(baud)) return { kind: 'serial', port, baud };
    return { kind: 'invalidBaud', port, value: raw };
  }

  const net = networkIntent(tokens);
  if (net) return net;

  return { kind: 'noMatch', text };
}
