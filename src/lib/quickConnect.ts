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
  | { kind: 'unsupported'; protocol: 'ssh' | 'telnet' }
  | { kind: 'invalidBaud'; port: string; value: string }
  | { kind: 'noMatch'; text: string };

const WINDOWS_COM = /^com(\d{1,3})$/i;
// Bare macOS / Linux device names typed without /dev/.
const BARE_DEVICE = /^(cu|tty)\.[\w.\-]+$|^tty(USB|ACM|S|AMA)\d+$/;
const IPV4 = /^\d{1,3}(\.\d{1,3}){3}(:\d{1,5})?$/;
const HOST_PORT = /^[\w.-]+:\d{1,5}$/;
const USER_AT_HOST = /^[^@\s]+@[\w.\-:[\]]+$/;
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

function networkProtocol(tokens: string[]): 'ssh' | 'telnet' | null {
  const first = (tokens[0] ?? '').toLowerCase();
  if (first === 'ssh' || first.startsWith('ssh://')) return 'ssh';
  if (first === 'telnet' || first.startsWith('telnet://')) return 'telnet';
  if (tokens.length === 2 && /^\d{1,5}$/.test(tokens[1] ?? '') && looksLikeHost(first)) {
    // "10.0.0.5 2001" — host + port is the telnet form.
    return 'telnet';
  }
  if (tokens.length === 1 && looksLikeHost(first)) return 'ssh';
  return null;
}

function looksLikeHost(token: string): boolean {
  return IPV4.test(token) || HOST_PORT.test(token) || USER_AT_HOST.test(token) || FQDN.test(token);
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

  const net = networkProtocol(tokens);
  if (net) return { kind: 'unsupported', protocol: net };

  return { kind: 'noMatch', text };
}
