import { describe, expect, it } from 'vitest';
import type { Discovery } from '../lib/ipc';
import { remember, SIGHTING_TTL_MS, withSightings } from './topology';

const disc = (macs: [string, string][], arp: [string, string][] = []): Discovery => ({
  neighbors: [],
  macs: macs.map(([mac, iface]) => ({ mac, vlan: '1', interface: iface, vendor: null })),
  arp: arp.map(([mac, ip]) => ({ mac, ip })),
  ownMac: null,
  ownVendor: null,
  gateway: null,
  sources: [],
});

describe('remembered end devices', () => {
  it('keeps what aged out of the switch, follows a move, forgets after a day', () => {
    let mem = remember(
      undefined,
      disc(
        [
          ['aa', 'Gi0/1'],
          ['bb', 'Gi0/2'],
        ],
        [['aa', '10.0.0.5']],
      ),
      1000,
    );
    // Next read: the printer (bb) aged out of the table, aa moved to Gi0/9.
    mem = remember(mem, disc([['aa', 'Gi0/9']]), 2000);
    const { discovery, seen } = withSightings(disc([]), mem);
    expect(discovery.macs.map((m) => `${m.mac}@${m.interface}`).sort()).toEqual([
      'aa@Gi0/9',
      'bb@Gi0/2',
    ]);
    expect(discovery.arp).toEqual([{ mac: 'aa', ip: '10.0.0.5' }]);
    expect(seen.get('aa')).toBe(2000);
    expect(seen.get('bb')).toBe(1000);
    mem = remember(mem, disc([['aa', 'Gi0/9']]), 1000 + SIGHTING_TTL_MS + 1);
    expect([...mem.macs.values()].map((m) => m.mac)).toEqual(['aa']);
  });
});
