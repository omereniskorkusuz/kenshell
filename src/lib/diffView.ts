/**
 * Diff display (P4-T08): changed rows with a few lines of context; long unchanged stretches
 * fold into one "n unchanged lines" row.
 */
import type { DiffRow } from './ipc';

export type DisplayRow = { type: 'row'; row: DiffRow } | { type: 'fold'; count: number };

export function foldDiff(rows: readonly DiffRow[], context = 3): DisplayRow[] {
  const keep = new Array<boolean>(rows.length).fill(false);
  rows.forEach((r, i) => {
    if (r.kind === 'same') return;
    for (let k = Math.max(0, i - context); k <= Math.min(rows.length - 1, i + context); k++) {
      keep[k] = true;
    }
  });
  const out: DisplayRow[] = [];
  let folded = 0;
  rows.forEach((r, i) => {
    if (keep[i]) {
      if (folded) out.push({ type: 'fold', count: folded });
      folded = 0;
      out.push({ type: 'row', row: r });
    } else {
      folded++;
    }
  });
  if (folded) out.push({ type: 'fold', count: folded });
  return out;
}
