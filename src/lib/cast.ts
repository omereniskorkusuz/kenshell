/**
 * asciicast v2 recordings (P5-T06): parsing and the replay timeline. Long pauses are
 * shortened to `idleLimit` seconds so a recording plays at a useful pace.
 */

export interface CastHeader {
  width: number;
  height: number;
  title: string | null;
  timestamp: number | null;
}

export interface CastEvent {
  /** Seconds on the (idle-limited) timeline. */
  t: number;
  kind: 'o' | 'r';
  data: string;
}

export interface Cast {
  header: CastHeader;
  events: CastEvent[];
  duration: number;
}

/** Parses a recording; null when it is not asciicast v2. Bad event lines are skipped. */
export function parseCast(text: string, idleLimit = 2): Cast | null {
  const lines = text.split('\n');
  let header: Record<string, unknown>;
  try {
    header = JSON.parse(lines[0] ?? '') as Record<string, unknown>;
  } catch {
    return null;
  }
  if (header.version !== 2) return null;
  const events: CastEvent[] = [];
  let prev = 0;
  let t = 0;
  for (const line of lines.slice(1)) {
    if (!line.trim()) continue;
    let e: unknown;
    try {
      e = JSON.parse(line);
    } catch {
      continue;
    }
    if (!Array.isArray(e) || typeof e[0] !== 'number' || typeof e[2] !== 'string') continue;
    const kind = e[1];
    if (kind !== 'o' && kind !== 'r') continue;
    const gap = Math.max(0, e[0] - prev);
    prev = Math.max(prev, e[0]);
    t += Math.min(gap, idleLimit);
    events.push({ t, kind, data: e[2] });
  }
  const num = (v: unknown, d: number) => (typeof v === 'number' && v > 0 ? v : d);
  return {
    header: {
      width: num(header.width, 80),
      height: num(header.height, 24),
      title: typeof header.title === 'string' ? header.title : null,
      timestamp: typeof header.timestamp === 'number' ? header.timestamp : null,
    },
    events,
    duration: events.length ? (events[events.length - 1]?.t ?? 0) : 0,
  };
}

/** Index of the first event after time `t` (events up to it have been played). */
export function indexAt(events: readonly CastEvent[], t: number): number {
  let lo = 0;
  let hi = events.length;
  while (lo < hi) {
    const mid = (lo + hi) >> 1;
    if ((events[mid]?.t ?? 0) <= t) lo = mid + 1;
    else hi = mid;
  }
  return lo;
}

/** `COLSxROWS` of a resize event. */
export function parseSize(data: string): { cols: number; rows: number } | null {
  const m = /^(\d+)x(\d+)$/.exec(data);
  return m ? { cols: Number(m[1]), rows: Number(m[2]) } : null;
}

export const formatClock = (secs: number) => {
  const s = Math.floor(secs);
  const mm = Math.floor(s / 60);
  return `${mm}:${String(s % 60).padStart(2, '0')}`;
};
