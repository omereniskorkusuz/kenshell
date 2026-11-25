import { describe, expect, it } from 'vitest';
import {
  blockAddresses,
  blockLabel,
  csvRows,
  filterHosts,
  freeBlocks,
  gridCells,
  ipToNum,
  numToIp,
  rttLabel,
  type ScanHost,
} from './netscan';

const host = (last: number, extra: Partial<ScanHost> = {}, base = '192.168.1.'): ScanHost => ({
  ip: `${base}${last}`,
  rttMs: 1.2,
  openPorts: [],
  mac: null,
  vendor: null,
  hostname: null,
  own: false,
  gateway: false,
  ...extra,
});

const r24 = { first: '192.168.1.1', last: '192.168.1.254' };

describe('network scanner helpers (P4-T14, R-T24)', () => {
  it('ip arithmetic', () => {
    expect(numToIp(ipToNum('10.20.3.255') + 1)).toBe('10.20.4.0');
  });

  it('free blocks are the range minus live hosts', () => {
    const range = { first: '192.168.1.1', last: '192.168.1.10' };
    expect(freeBlocks(range, [host(1), host(5), host(6)])).toEqual([
      { first: '192.168.1.2', last: '192.168.1.4', count: 3 },
      { first: '192.168.1.7', last: '192.168.1.10', count: 4 },
    ]);
    expect(freeBlocks(range, [])).toEqual([
      { first: '192.168.1.1', last: '192.168.1.10', count: 10 },
    ]);
    expect(freeBlocks(range, [host(10)]).pop()?.last).toBe('192.168.1.9');
    expect(blockLabel({ first: '192.168.1.50', last: '192.168.1.89', count: 40 })).toBe(
      '192.168.1.50 – .89',
    );
    expect(blockLabel({ first: '10.0.0.250', last: '10.0.1.5', count: 12 })).toBe(
      '10.0.0.250 – .1.5',
    );
    expect(blockAddresses({ first: '192.168.1.2', last: '192.168.1.4', count: 3 })).toBe(
      '192.168.1.2\n192.168.1.3\n192.168.1.4',
    );
  });

  it('grid cells per /24; a wide range draws only blocks with live hosts', () => {
    const g = gridCells(r24, [host(1, { gateway: true }), host(2), host(34, { own: true })]);
    expect(g).toHaveLength(1);
    const kinds = g[0]?.cells.map((c) => c.kind) ?? [];
    expect(kinds.slice(0, 4)).toEqual(['outside', 'gateway', 'used', 'free']);
    expect(kinds[34]).toBe('own');
    expect(kinds[255]).toBe('outside');
    expect(kinds).toHaveLength(256);

    const wide = { first: '10.0.0.1', last: '10.0.255.254' };
    const g2 = gridCells(wide, [host(7, {}, '10.0.40.'), host(9, {}, '10.0.3.')]);
    expect(g2.map((b) => b.base)).toEqual(['10.0.3.0', '10.0.40.0']);
    expect(gridCells({ first: '10.0.0.1', last: '10.0.3.254' }, [])).toHaveLength(4);
  });

  it('search, response time and CSV', () => {
    const hosts = [
      host(2, { hostname: 'core-sw1.lan', mac: '00:1a:2b:03:04:05', vendor: 'Example Corp' }),
      host(90, { hostname: 'PC-MUHASEBE', rttMs: null }),
    ];
    expect(filterHosts(hosts, 'core').map((h) => h.ip)).toEqual(['192.168.1.2']);
    expect(filterHosts(hosts, '001a.2b03')).toHaveLength(1);
    expect(filterHosts(hosts, 'muhasebe')).toHaveLength(1);
    expect(filterHosts(hosts, '.90')).toHaveLength(1);
    expect(filterHosts(hosts, ' ')).toHaveLength(2);
    expect(rttLabel(host(1, { rttMs: 0.42 }))).toBe('0.4 ms');
    expect(rttLabel(host(1, { rttMs: 23.6 }))).toBe('24 ms');
    expect(rttLabel(host(1, { rttMs: 0 }))).toBe('<1 ms');
    expect(rttLabel(host(1, { rttMs: null, mac: 'aa:bb:cc:dd:ee:ff' }))).toBe('ARP');
    expect(rttLabel(host(1, { rttMs: null }))).toBeNull();
    expect(
      csvRows([host(2, { openPorts: [22, 443] }), host(1, { rttMs: null })], () => 'up'),
    ).toEqual([
      ['192.168.1.1', 'up', '', '', '', '', ''],
      ['192.168.1.2', 'up', '1.2', '', '', '', '22 443'],
    ]);
  });
});
