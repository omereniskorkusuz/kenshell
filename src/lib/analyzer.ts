/**
 * Analyzer table helpers (P4-T01): command matching (mirror of Rust
 * `analyzer::command_matches`), type-aware sorting, filtering, hiding empty columns.
 */
import type { AnalyzerColumn, ColumnKind } from './ipc';
import { parseIface } from './iface';

/** `sh ip int br` is an abbreviation of `show ip interface brief` (pipes ignored). */
export function commandMatches(typed: string, canonical: string): boolean {
  const t = (typed.split('|')[0] ?? '').trim().split(/\s+/).filter(Boolean);
  const c = canonical.split(/\s+/);
  return (
    t.length > 0 &&
    t.length === c.length &&
    t.every((w, i) => (c[i] as string).toLowerCase().startsWith(w.toLowerCase()))
  );
}

const collator = new Intl.Collator(undefined, { numeric: true, sensitivity: 'base' });

function ipValue(s: string): number[] | null {
  const m = /^(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.(\d{1,3})(?:\/(\d{1,2}))?$/.exec(s.trim());
  return m ? m.slice(1).map((x) => Number(x ?? 0)) : null;
}

/** Compares two cells of a column kind; empty cells sort last. */
export function compareCells(a: string, b: string, kind: ColumnKind): number {
  if (a === b) return 0;
  if (!a) return 1;
  if (!b) return -1;
  if (kind === 'number') {
    const x = Number(a.replace(/,/g, ''));
    const y = Number(b.replace(/,/g, ''));
    if (Number.isFinite(x) && Number.isFinite(y)) return x - y;
  }
  if (kind === 'ip') {
    const x = ipValue(a);
    const y = ipValue(b);
    if (x && y) {
      for (let i = 0; i < x.length; i++) if (x[i] !== y[i]) return (x[i] ?? 0) - (y[i] ?? 0);
      return 0;
    }
    if (x) return -1;
    if (y) return 1;
  }
  if (kind === 'iface') {
    const x = parseIface(a)?.key ?? a;
    const y = parseIface(b)?.key ?? b;
    return collator.compare(x, y);
  }
  return collator.compare(a, b);
}

export interface SortState {
  col: number;
  dir: 1 | -1;
}

export function sortRows(
  rows: readonly string[][],
  columns: readonly AnalyzerColumn[],
  sort: SortState | null,
): string[][] {
  if (!sort) return [...rows];
  const kind = columns[sort.col]?.kind ?? 'text';
  return [...rows].sort(
    (r1, r2) => sort.dir * compareCells(r1[sort.col] ?? '', r2[sort.col] ?? '', kind),
  );
}

/** Rows where every word of `query` appears in some cell (case-insensitive). */
export function filterRows(rows: readonly string[][], query: string): string[][] {
  const words = query.toLocaleLowerCase().split(/\s+/).filter(Boolean);
  if (!words.length) return [...rows];
  return rows.filter((r) => {
    const hay = r.join('\u0001').toLocaleLowerCase();
    return words.every((w) => hay.includes(w));
  });
}

/** Indexes of columns that have at least one value. */
export function shownColumns(rows: readonly string[][], width: number): number[] {
  const out: number[] = [];
  for (let c = 0; c < width; c++) if (rows.some((r) => (r[c] ?? '') !== '')) out.push(c);
  return out;
}

/** Tone of an up/down style cell. */
export function statusTone(cell: string): 'success' | 'danger' | 'warning' | null {
  const c = cell.toLowerCase();
  if (/err-?disabled|errdisable/.test(c)) return 'warning';
  if (/\b(down|notconnect|disabled|inactive|suspended)\b/.test(c)) return 'danger';
  if (/\b(up|connected|active)\b/.test(c)) return 'success';
  return null;
}
