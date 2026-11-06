/**
 * Ping / traceroute output (P5-T05): the OS tools' text is shown as is; these parsers
 * pull the round-trip times and hops out of it for a summary. They work on macOS,
 * Linux (traceroute and tracepath) and Windows output, in any UI language (they key
 * on numbers, "ttl" and "ms", not on words).
 */

const RTT = /<?\s*(\d+(?:[.,]\d+)?)\s*ms\b/gi;

/** Round-trip time of a ping reply line, or null (headers, timeouts, summaries). */
export function pingReply(line: string): number | null {
  if (!/ttl[=\s]/i.test(line)) return null;
  const m = /[=<]\s*(\d+(?:[.,]\d+)?)\s*ms/i.exec(line);
  return m?.[1] ? Number(m[1].replace(',', '.')) : null;
}

export interface PingStats {
  replies: number;
  min: number;
  avg: number;
  max: number;
  last: number;
}

export function pingStats(lines: readonly string[]): PingStats | null {
  const rtts = lines.map(pingReply).filter((r): r is number => r !== null);
  if (rtts.length === 0) return null;
  return {
    replies: rtts.length,
    min: Math.min(...rtts),
    avg: rtts.reduce((a, b) => a + b, 0) / rtts.length,
    max: Math.max(...rtts),
    last: rtts[rtts.length - 1] ?? 0,
  };
}

export interface Hop {
  hop: number;
  /** Address / name text with the times and `*`s taken out. */
  text: string;
  rtts: number[];
  timeouts: number;
}

/** Hops of traceroute / tracert / tracepath output, in order. */
export function traceHops(lines: readonly string[]): Hop[] {
  const hops: Hop[] = [];
  for (const line of lines) {
    const m = /^\s*(\d{1,3})\??:?\s+(.*)$/.exec(line);
    // A line without a hop number continues the previous hop (another responder).
    const rest = m ? (m[2] ?? '') : /^\s+\S/.test(line) && hops.length ? line : null;
    if (rest === null) continue;
    const rtts = [...rest.matchAll(RTT)].map((x) => Number((x[1] ?? '').replace(',', '.')));
    const timeouts = (rest.match(/(^|\s)\*(?=\s|$)/g) ?? []).length;
    const text = rest
      .replace(RTT, ' ')
      .replace(/(^|\s)\*(?=\s|$)/g, ' ')
      .replace(/\s+/g, ' ')
      .trim();
    // tracepath's own header lines ("1?: [LOCALHOST] pmtu 1500") are not hops.
    if (/\[LOCALHOST\]|^Resume:/i.test(text)) continue;
    const prev = hops[hops.length - 1];
    if (m && (!prev || prev.hop !== Number(m[1]))) {
      hops.push({ hop: Number(m[1]), text, rtts, timeouts });
    } else if (prev) {
      prev.rtts.push(...rtts);
      prev.timeouts += timeouts;
      if (text && !prev.text.includes(text))
        prev.text = [prev.text, text].filter(Boolean).join(' · ');
    }
  }
  return hops;
}
