import { describe, expect, it } from 'vitest';
import {
  commandMatches,
  compareCells,
  filterRows,
  shownColumns,
  sortRows,
  statusTone,
} from './analyzer';
import type { AnalyzerColumn } from './ipc';

describe('analyzer table helpers (P4-T01)', () => {
  it('command abbreviations', () => {
    expect(commandMatches('sh ip int br', 'show ip interface brief')).toBe(true);
    expect(commandMatches('sh ip int br | ex unass', 'show ip interface brief')).toBe(true);
    expect(commandMatches('sh ip int', 'show ip interface brief')).toBe(false);
    expect(commandMatches('', 'show version')).toBe(false);
  });

  it('sorts interfaces naturally (any spelling), IPs numerically, empties last', () => {
    const cols: AnalyzerColumn[] = [
      { key: 'interface', kind: 'iface' },
      { key: 'ip', kind: 'ip' },
    ];
    const rows = [
      ['Gi1/0/10', '10.0.0.10'],
      ['GigabitEthernet 1/0/2', '10.0.0.9'],
      ['Gi1/0/1', ''],
    ];
    expect(sortRows(rows, cols, { col: 0, dir: 1 }).map((r) => r[0])).toEqual([
      'Gi1/0/1',
      'GigabitEthernet 1/0/2',
      'Gi1/0/10',
    ]);
    expect(sortRows(rows, cols, { col: 1, dir: 1 }).map((r) => r[1])).toEqual([
      '10.0.0.9',
      '10.0.0.10',
      '',
    ]);
    expect(compareCells('9', '10', 'number')).toBeLessThan(0);
  });

  it('filter, empty columns and status tone', () => {
    const rows = [
      ['Vlan1', 'unassigned', '', 'administratively down'],
      ['Vlan10', '10.0.10.2', '', 'up'],
    ];
    expect(filterRows(rows, 'vlan 10.0')).toEqual([rows[1]]);
    expect(filterRows(rows, '  ')).toEqual(rows);
    expect(shownColumns(rows, 4)).toEqual([0, 1, 3]);
    expect(statusTone('administratively down')).toBe('danger');
    expect(statusTone('up')).toBe('success');
    expect(statusTone('err-disabled')).toBe('warning');
    expect(statusTone('YES')).toBeNull();
  });
});
