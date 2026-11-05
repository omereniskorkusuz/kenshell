import { describe, expect, it } from 'vitest';
import { findLinks, ifaceCommands } from './links';

const kinds = (line: string) => findLinks(line).map((l) => `${l.kind}:${l.text}`);

describe('clickable terminal items (P3-T11)', () => {
  it('finds IPs and interfaces (Ruijie spacing too), not versions or OIDs', () => {
    expect(kinds('Vlan10   10.0.10.2   YES NVRAM  up   up')).toEqual([
      'iface:Vlan10',
      'ip:10.0.10.2',
    ]);
    expect(kinds('GigabitEthernet 0/24 is UP, address 192.168.1.1/24')).toEqual([
      'iface:GigabitEthernet 0/24',
      'ip:192.168.1.1',
    ]);
    expect(kinds('Version 15.2(7)E8, OID 1.3.6.1.4.1.9, 256.1.1.1')).toEqual([]);
  });

  it('interface commands follow the vendor', () => {
    expect(ifaceCommands('cisco', 'Gi1/0/24')).toEqual({
      show: 'show interface Gi1/0/24',
      config: 'show running-config interface Gi1/0/24',
    });
    expect(ifaceCommands(null, 'GigabitEthernet 0/1').show).toBe(
      'show interface GigabitEthernet 0/1',
    );
    expect(ifaceCommands('huawei', 'GE0/0/1').show).toBe('display interface GE0/0/1');
    expect(ifaceCommands('juniper', 'ge-0/0/1').config).toBe(
      'show configuration interfaces ge-0/0/1',
    );
    expect(ifaceCommands('alcatel', '1/1/1').config).toBeNull();
  });
});
