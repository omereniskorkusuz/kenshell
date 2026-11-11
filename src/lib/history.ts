/**
 * Connection history: every connection that reached "connected", saved session or not
 * (quick connect, detected console). Stored as the `history.connections` setting, newest
 * first, one entry per endpoint. Holds only what is needed to connect again: never a
 * password (targets carry none).
 */
import type { OpenTarget } from './ipc';

export interface HistoryEntry {
  /** One entry per endpoint: `s:<sessionId>` or the endpoint itself. */
  key: string;
  name: string;
  envTag: string | null;
  /** Saved session it came from (may have been deleted since). */
  sessionId: number | null;
  target: OpenTarget;
  /** Last connected, epoch ms. */
  at: number;
}

export const MAX_HISTORY = 30;

export function historyKey(target: OpenTarget, sessionId: number | null): string {
  if (sessionId !== null) return `s:${sessionId}`;
  if (target.type === 'serial') return `serial:${target.settings.port}`;
  const user = target.username ? `${target.username}@` : '';
  return `${target.protocol}:${user}${target.host.toLowerCase()}:${target.port ?? ''}`;
}

/** `entry` first, any older entry of the same endpoint removed, capped at `max`. */
export function addHistory(
  list: readonly HistoryEntry[],
  entry: Omit<HistoryEntry, 'key'>,
  max = MAX_HISTORY,
): HistoryEntry[] {
  const key = historyKey(entry.target, entry.sessionId);
  return [{ ...entry, key }, ...list.filter((e) => e.key !== key)].slice(0, max);
}

export function removeHistory(list: readonly HistoryEntry[], key: string): HistoryEntry[] {
  return list.filter((e) => e.key !== key);
}

const isObj = (v: unknown): v is Record<string, unknown> => typeof v === 'object' && v !== null;

function validTarget(v: unknown): v is OpenTarget {
  if (!isObj(v)) return false;
  if (v.type === 'serial')
    return isObj(v.settings) && typeof v.settings.port === 'string' && v.settings.port !== '';
  return (
    v.type === 'network' &&
    typeof v.protocol === 'string' &&
    ['ssh', 'telnet', 'raw', 'rlogin'].includes(v.protocol) &&
    typeof v.host === 'string' &&
    v.host !== '' &&
    (v.port === null || typeof v.port === 'number') &&
    (v.username === null || typeof v.username === 'string') &&
    isObj(v.ssh)
  );
}

/** Stored value → valid entries (anything malformed is dropped). */
export function sanitizeHistory(value: unknown): HistoryEntry[] {
  if (!Array.isArray(value)) return [];
  const out: HistoryEntry[] = [];
  const seen = new Set<string>();
  for (const raw of value as unknown[]) {
    if (!isObj(raw) || !validTarget(raw.target)) continue;
    if (typeof raw.name !== 'string' || typeof raw.at !== 'number') continue;
    const sessionId = typeof raw.sessionId === 'number' ? raw.sessionId : null;
    const key = historyKey(raw.target, sessionId);
    if (seen.has(key)) continue;
    seen.add(key);
    out.push({
      key,
      name: raw.name.slice(0, 200),
      envTag: typeof raw.envTag === 'string' ? raw.envTag : null,
      sessionId,
      target: raw.target,
      at: raw.at,
    });
    if (out.length >= MAX_HISTORY) break;
  }
  return out;
}

/** A row of the "Recent" list: a saved session (still existing) or a stored endpoint. */
export interface RecentItem<S> {
  key: string;
  name: string;
  envTag: string | null;
  at: number;
  session: S | null;
  /** Null only for a saved session known from its own "last connected" time. */
  target: OpenTarget | null;
  /** False when the row does not come from the history. */
  removable: boolean;
}

interface SessionLike {
  id: number;
  name: string;
  envTag: string | null;
  lastConnectedAt: number | null;
}

/**
 * History entries (a saved session shows its current name and tag) plus saved sessions
 * that connected before the history existed, newest first.
 */
export function recentItems<S extends SessionLike>(
  history: readonly HistoryEntry[],
  sessions: readonly S[],
  limit = 8,
): RecentItem<S>[] {
  const byId = new Map(sessions.map((s) => [s.id, s]));
  const items: RecentItem<S>[] = history.map((e) => {
    const session = e.sessionId !== null ? (byId.get(e.sessionId) ?? null) : null;
    return {
      key: e.key,
      name: session?.name ?? e.name,
      envTag: session ? session.envTag : e.envTag,
      at: e.at,
      session,
      target: e.target,
      removable: true,
    };
  });
  return items.sort((a, b) => b.at - a.at).slice(0, limit);
}
