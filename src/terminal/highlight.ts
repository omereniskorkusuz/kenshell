/**
 * Output colouring rules (P3-T10): pure line matcher. Rules come from
 * `src/data/highlight.json` (vendor-specific ones carry `families`); interface names use the
 * shared normaliser (`scanIfaces`), so Ruijie `GigabitEthernet 0/24` is found as well.
 * The terminal applies the spans to visible rows only (`highlighter.ts`).
 */
import data from '../data/highlight.json';
import { scanIfaces } from '../lib/iface';

export const HIGHLIGHT_CATEGORIES = ['status', 'errors', 'addresses', 'interfaces'] as const;
export type HighlightCategory = (typeof HIGHLIGHT_CATEGORIES)[number];
export type HighlightStyle = 'up' | 'down' | 'error' | 'warning' | 'ip' | 'mac' | 'iface';

export interface Span {
  start: number;
  end: number;
  style: HighlightStyle;
}

interface RuleDef {
  category: HighlightCategory;
  style: HighlightStyle;
  pattern: string;
  families?: string[];
}

export interface CompiledRules {
  regexes: { re: RegExp; style: HighlightStyle }[];
  interfaces: boolean;
}

const RULES = data.rules as RuleDef[];
const cache = new Map<string, CompiledRules>();

/** Rules for one vendor family (null = not identified), without the disabled categories. */
export function compileRules(family: string | null, off: readonly string[] = []): CompiledRules {
  const key = `${family ?? ''}|${[...off].sort().join(',')}`;
  const hit = cache.get(key);
  if (hit) return hit;
  const rules: CompiledRules = {
    regexes: RULES.filter(
      (r) =>
        !off.includes(r.category) &&
        (!r.families || (family !== null && r.families.includes(family))),
    ).map((r) => ({ re: new RegExp(r.pattern, 'g'), style: r.style })),
    interfaces: !off.includes('interfaces'),
  };
  cache.set(key, rules);
  return rules;
}

/** Longer lines are not coloured (keeps a pathological line cheap). */
const MAX_LINE = 1000;

/** Non-overlapping coloured spans of one line, left to right. */
export function highlightLine(text: string, rules: CompiledRules): Span[] {
  if (!text.trim() || text.length > MAX_LINE) return [];
  const spans: Span[] = [];
  const taken = (s: number, e: number) => spans.some((x) => s < x.end && e > x.start);
  for (const { re, style } of rules.regexes) {
    re.lastIndex = 0;
    for (let m = re.exec(text); m; m = re.exec(text)) {
      if (m[0].length === 0) {
        re.lastIndex++;
        continue;
      }
      // A leading space consumed by a `(?:^| )` alternative is not coloured.
      const lead = m[0].length - m[0].trimStart().length;
      const s = m.index + lead;
      const e = m.index + m[0].trimEnd().length;
      if (e > s && !taken(s, e)) spans.push({ start: s, end: e, style });
    }
  }
  if (rules.interfaces) {
    for (const h of scanIfaces(text)) {
      if (!taken(h.start, h.end)) spans.push({ start: h.start, end: h.end, style: 'iface' });
    }
  }
  return spans.sort((a, b) => a.start - b.start);
}
