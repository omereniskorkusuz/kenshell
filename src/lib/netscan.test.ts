import { describe, expect, it } from 'vitest';
import {
  blockAddresses,
  blockLabel,
  csvRows,
  freeBlocks,
  gridCells,
  ipToNum,
  numToIp,
  type ScanHost,
} from './netscan';

const host = (last: number, alive: boolean, extra: Partial<ScanHost> = {}): ScanHost => ({
  ip: `192.168.1.${last}`,
  alive,
  icmp: false,
  openPorts: [],
  mac: null,
  hostname: null,
  own: false,
  gateway: false,
  ...extra,
});

describe('network scanner helpers (P4-T14)', () => {
  it('ip arithmetic', () => {
    expect(numToIp(ipToNum('10.20.3.255') + 1)).toBe('10.20.4.0');
  });

  it('free blocks are consecutive silent addresses', () => {
    const hosts = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10].map((n) => host(n, [1, 5, 6].includes(n)));
    expect(freeBlocks(hosts)).toEqual([
      { first: '192.168.1.2', last: '192.168.1.4', count: 3 },
      { first: '192.168.1.7', last: '192.168.1.10', count: 4 },
    ]);
    expect(blockLabel({ first: '192.168.1.50', last: '192.168.1.89', count: 40 })).toBe(
      '192.168.1.50 – .89',
    );
    expect(blockLabel({ first: '10.0.0.250', last: '10.0.1.5', count: 12 })).toBe(
      '10.0.0.250 – .1.5',
    );
    expect(blockAddresses({ first: '192.168.1.2', last: '192.168.1.4', count: 3 })).toBe(
      '192.168.1.2\n192.168.1.3\n192.168.1.4',
    );
    // A gap in the scanned addresses splits a block.
    expect(freeBlocks([host(2, false), host(4, false)])).toHaveLength(2);
  });

  it('grid cells per /24 and CSV', () => {
    const g = gridCells([
      host(1, true, { gateway: true }),
      host(2, true),
      host(3, false),
      host(34, true, { own: true }),
    ]);
    expect(g).toHaveLength(1);
    const kinds = g[0]?.cells.map((c) => c.kind) ?? [];
    expect(kinds.slice(0, 4)).toEqual(['outside', 'gateway', 'used', 'free']);
    expect(kinds[34]).toBe('own');
    expect(kinds).toHaveLength(256);
    expect(
      csvRows([host(2, true, { openPorts: [22, 443] }), host(1, false)], (h) =>
        h.alive ? 'up' : 'free',
      ),
    ).toEqual([
      ['192.168.1.1', 'free', '', '', '', ''],
      ['192.168.1.2', 'up', '', '', '', '22 443'],
    ]);
  });
});
