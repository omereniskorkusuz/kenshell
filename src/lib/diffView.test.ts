import { describe, expect, it } from 'vitest';
import { foldDiff } from './diffView';
import type { DiffRow } from './ipc';

const same = (n: number): DiffRow => ({
  kind: 'same',
  leftNo: n,
  left: `l${n}`,
  rightNo: n,
  right: `l${n}`,
});

describe('diff folding (P4-T08)', () => {
  it('keeps context around changes and folds the rest', () => {
    const rows: DiffRow[] = [
      ...Array.from({ length: 10 }, (_, i) => same(i + 1)),
      { kind: 'changed', leftNo: 11, left: 'a', rightNo: 11, right: 'b' },
      ...Array.from({ length: 10 }, (_, i) => same(i + 12)),
    ];
    const d = foldDiff(rows, 2);
    expect(d[0]).toEqual({ type: 'fold', count: 8 });
    expect(d.filter((x) => x.type === 'row')).toHaveLength(5);
    expect(d[d.length - 1]).toEqual({ type: 'fold', count: 8 });
    expect(foldDiff([same(1), same(2)])).toEqual([{ type: 'fold', count: 2 }]);
    expect(foldDiff([])).toEqual([]);
  });
});
