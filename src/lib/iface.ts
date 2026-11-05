/**
 * Interface name normalization (P3-T01), the frontend twin of `device/iface.rs`.
 *
 * Ruijie writes `GigabitEthernet 0/24`, Cisco `GigabitEthernet0/24`, people type `Gi0/24` or
 * `gi 0/24`: all are the same interface and get the same `key`. Types and abbreviations come
 * from `src/data/interfaces.json`; `src/data/iface-cases.json` tests both implementations.
 */
import data from '../data/interfaces.json';

interface TypeDef {
  canonical: string;
  short: string;
  names: string[];
  aliases: string[];
}

export interface Iface {
  /** Canonical type, e.g. `GigabitEthernet`. */
  kind: string;
  /** Numbering, e.g. `1/0/24`. */
  number: string;
  /** Lowercase match key (same for every spelling). */
  key: string;
  /** Display form, e.g. `Gi1/0/24`. */
  short: string;
}

const TYPES = data.types as TypeDef[];
/** `0/24`, `1/0/24`, `0/1.100` (sub-interface), `0/0/0:1` (channel). */
const NUMBER = String.raw`\d+(?:/\d+)*(?:\.\d+)?(?::\d+)?`;
const WHOLE = new RegExp(String.raw`^([A-Za-z][A-Za-z-]*) ?(${NUMBER})$`);
const escape = (s: string) => s.replace(/[.*+?^${}()|[\]\\-]/g, '\\$&');
const SPELLINGS = [...new Set(TYPES.flatMap((t) => [...t.names, ...t.aliases]))]
  .filter((s) => s.length >= 2)
  .sort((a, b) => b.length - a.length)
  .map(escape);
/** Mentions in free text: known spellings only, not inside a longer word or number. */
const SCAN = new RegExp(
  String.raw`(?:^|[^A-Za-z0-9])((?:${SPELLINGS.join('|')}) ?${NUMBER})(?=$|[^\w/])`,
  'gi',
);

/** Exact alias, full name, or a unique prefix (≥ 2 letters) of a full name. */
function resolve(word: string): TypeDef | null {
  const w = word.toLowerCase();
  const byAlias = TYPES.find((t) => t.aliases.some((a) => a.toLowerCase() === w));
  if (byAlias) return byAlias;
  const byName = TYPES.find((t) => t.names.some((n) => n.toLowerCase() === w));
  if (byName) return byName;
  if (w.length < 2) return null;
  const hits = TYPES.filter((t) => t.names.some((n) => n.toLowerCase().startsWith(w)));
  return hits.length === 1 ? (hits[0] as TypeDef) : null;
}

/** Normalizes one interface name (surrounding whitespace ignored); null if it is not one. */
export function parseIface(text: string): Iface | null {
  const m = WHOLE.exec(text.trim());
  if (!m) return null;
  const def = resolve(m[1] as string);
  if (!def) return null;
  const number = m[2] as string;
  return {
    kind: def.canonical,
    number,
    key: `${def.canonical.toLowerCase()}${number}`,
    short: `${def.short}${number}`,
  };
}

export const canonicalName = (i: Iface): string => `${i.kind}${i.number}`;

/** Interface mentions in `text`, in order, with their position. */
export function scanIfaces(text: string): { start: number; end: number; iface: Iface }[] {
  const out: { start: number; end: number; iface: Iface }[] = [];
  SCAN.lastIndex = 0;
  for (let m = SCAN.exec(text); m; m = SCAN.exec(text)) {
    const word = m[1] as string;
    const start = m.index + m[0].length - word.length;
    const iface = parseIface(word);
    if (iface) out.push({ start, end: start + word.length, iface });
    // The leading boundary character was consumed: continue right after the name.
    SCAN.lastIndex = start + word.length;
  }
  return out;
}

/** True when both names are the same interface (any spelling). */
export function sameIface(a: string, b: string): boolean {
  const x = parseIface(a);
  const y = parseIface(b);
  return x !== null && y !== null && x.key === y.key;
}
