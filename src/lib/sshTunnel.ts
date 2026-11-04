/**
 * Text forms for SSH jump host, proxy and port forwards (pure).
 *
 * The editor keeps them as short OpenSSH-like strings so a network engineer can type
 * them the way they would on the command line:
 *   jump      "admin@bastion:2222"  (user and port optional)
 *   proxy     "10.0.0.5:1080"
 *   forwards  one per line: "L 8080:10.1.1.1:80", "R 9000:127.0.0.1:22", "D 1080"
 */
import type { Forward, ForwardKind, JumpHost } from './ipc';

export const MAX_FORWARDS = 16;

const port = (raw: string): number | null =>
  /^\d{1,5}$/.test(raw) && Number(raw) >= 1 && Number(raw) <= 65535 ? Number(raw) : null;

/** "host" or "[v6]" plus optional ":port"; null when malformed. */
export function parseHostPort(
  text: string,
  requirePort: boolean,
): { host: string; port: number | null } | null {
  const t = text.trim();
  if (!t || /\s/.test(t)) return null;
  let host: string;
  let rest: string | null;
  if (t.startsWith('[')) {
    const end = t.indexOf(']');
    if (end < 2) return null;
    host = t.slice(1, end);
    const after = t.slice(end + 1);
    if (after && !after.startsWith(':')) return null;
    rest = after ? after.slice(1) : null;
  } else {
    const colons = t.split(':').length - 1;
    if (colons === 1) {
      const i = t.indexOf(':');
      host = t.slice(0, i);
      rest = t.slice(i + 1);
    } else {
      // Bare IPv6 without brackets cannot carry a port.
      host = t;
      rest = null;
    }
  }
  if (!host || host.length > 255) return null;
  if (rest === null) return requirePort ? null : { host, port: null };
  const p = port(rest);
  return p === null ? null : { host, port: p };
}

export function parseJump(text: string): JumpHost | null | 'invalid' {
  const t = text.trim();
  if (!t) return null;
  const at = t.lastIndexOf('@');
  const user = at >= 0 ? t.slice(0, at) : '';
  const hp = parseHostPort(at >= 0 ? t.slice(at + 1) : t, false);
  if (!hp || (at >= 0 && !user) || /\s/.test(user)) return 'invalid';
  return { host: hp.host, port: hp.port, username: user || null };
}

export function formatJump(j: JumpHost | null | undefined): string {
  if (!j) return '';
  const host = j.host.includes(':') ? `[${j.host}]` : j.host;
  return `${j.username ? `${j.username}@` : ''}${host}${j.port ? `:${j.port}` : ''}`;
}

const KIND: Record<string, ForwardKind> = { l: 'local', r: 'remote', d: 'dynamic' };
const LETTER: Record<ForwardKind, string> = { local: 'L', remote: 'R', dynamic: 'D' };

/**
 * Parses forward lines; returns the 1-based numbers of bad lines instead when any
 * (an empty list means "more than MAX_FORWARDS").
 */
export function parseForwards(text: string): Forward[] | { badLines: number[] } {
  const out: Forward[] = [];
  const bad: number[] = [];
  text.split(/\r?\n/).forEach((raw, i) => {
    const line = raw.trim();
    if (!line) return;
    const m = /^([LRDlrd])\s+(\S+)$/.exec(line);
    const kind = m ? KIND[(m[1] as string).toLowerCase()] : undefined;
    const spec = m?.[2] ?? '';
    if (!kind) return void bad.push(i + 1);
    if (kind === 'dynamic') {
      const p = port(spec);
      if (p === null) return void bad.push(i + 1);
      out.push({ kind, listenPort: p, targetHost: null, targetPort: null });
      return;
    }
    const first = spec.indexOf(':');
    const listen = first > 0 ? port(spec.slice(0, first)) : null;
    const target = first > 0 ? parseHostPort(spec.slice(first + 1), true) : null;
    if (listen === null || !target) return void bad.push(i + 1);
    out.push({ kind, listenPort: listen, targetHost: target.host, targetPort: target.port });
  });
  if (bad.length) return { badLines: bad };
  return out.length > MAX_FORWARDS ? { badLines: [] } : out;
}

export function formatForwards(list: readonly Forward[] | null | undefined): string {
  return (list ?? [])
    .map((f) => {
      if (f.kind === 'dynamic') return `D ${f.listenPort}`;
      const host = f.targetHost?.includes(':') ? `[${f.targetHost}]` : (f.targetHost ?? '');
      return `${LETTER[f.kind]} ${f.listenPort}:${host}:${f.targetPort ?? ''}`;
    })
    .join('\n');
}
