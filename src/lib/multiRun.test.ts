import { describe, expect, it } from 'vitest';
import type { AnalyzerTable } from './ipc';
import { isReadOnly, mergeTables } from './multiRun';

const table = (parser: string, rows: string[][]): AnalyzerTable => ({
  parser,
  command: 'show version',
  columns: [{ key: 'field', kind: 'text' }],
  rows,
  truncated: false,
});

describe('multi-device commands (P4-T11)', () => {
  it('only read-only commands (same rules as the backend)', () => {
    for (const ok of [
      'show version',
      'sh ip int br',
      'display interface brief',
      'show log | inc %LINK',
    ]) {
      expect(isReadOnly(ok), ok).toBe(true);
    }
    for (const bad of [
      'conf t',
      'reload',
      's ver',
      'show run | redirect flash:x',
      'show ver; reload',
      '',
    ]) {
      expect(isReadOnly(bad), bad).toBe(false);
    }
  });

  it('merges same-parser tables with a device column', () => {
    const m = mergeTables([
      { device: 'sw1', table: table('p', [['a']]) },
      { device: 'sw2', table: table('p', [['b'], ['c']]) },
    ]);
    expect(m?.columns.map((c) => c.key)).toEqual(['device', 'field']);
    expect(m?.rows).toEqual([
      ['sw1', 'a'],
      ['sw2', 'b'],
      ['sw2', 'c'],
    ]);
    expect(
      mergeTables([
        { device: 'a', table: table('p', []) },
        { device: 'b', table: null },
      ]),
    ).toBeNull();
    expect(
      mergeTables([
        { device: 'a', table: table('p', []) },
        { device: 'b', table: table('q', []) },
      ]),
    ).toBeNull();
    expect(mergeTables([])).toBeNull();
  });
});
