/** Browser-demo data for the topology map: a small campus seen from each of its devices. */
import type { Discovery, FlowSnapshot, Neighbor, SnmpDevice } from '../ipc';

const nb = (
  name: string,
  ip: string | null,
  platform: string | null,
  capability: string | null,
  localInterface: string,
  remoteInterface: string | null,
  chassis: string | null = null,
  vendor: string | null = null,
): Neighbor => ({
  name,
  ip,
  platform,
  capability,
  localInterface,
  remoteInterface,
  cdp: false,
  lldp: true,
  chassis,
  vendor,
});

const mac = (m: string, vlan: string, iface: string, vendor: string | null) => ({
  mac: m,
  vlan,
  interface: iface,
  vendor,
});

const CORE: Discovery = {
  neighbors: [
    nb(
      'fw-edge',
      '10.0.0.254',
      'FW-1500 firewall',
      'Router',
      'TenGigabitEthernet1/1/1',
      'port1',
      '0c1122aa0001',
      'Example Security Inc.',
    ),
    nb(
      'access-sw3.lab.example.com',
      '10.0.0.20',
      'S5750-48GT4XS',
      'Bridge',
      'GigabitEthernet1/0/47',
      'GigabitEthernet 0/49',
      '5869aa001111',
      'Example Networks Co.',
    ),
    nb(
      'access-sw4',
      '10.0.0.21',
      'S2910-24GT4XS',
      'Bridge',
      'GigabitEthernet1/0/48',
      'GigabitEthernet 0/25',
      '5869aa002222',
      'Example Networks Co.',
    ),
    nb(
      'ap-floor2',
      '10.20.0.15',
      'AP-2802I',
      'WLAN Access Point',
      'GigabitEthernet1/0/5',
      'GigabitEthernet0',
      '001a2b030405',
      'Example Wireless',
    ),
  ],
  macs: [
    mac('0011223344a1', '10', 'Gi1/0/10', 'Example Printers Ltd.'),
    mac('0011223344b2', '10', 'Gi1/0/11', 'Example Computers'),
    // Learned through the uplinks: belong to the other side.
    mac('3c5a00000001', '20', 'Gi1/0/47', null),
    mac('3c5a00000002', '20', 'Gi1/0/47', null),
  ],
  arp: [
    { ip: '10.0.0.1', mac: '001a2b0000c1' },
    { ip: '10.10.0.31', mac: '0011223344a1' },
    { ip: '10.10.0.52', mac: '0011223344b2' },
    { ip: '10.20.0.101', mac: '3c5a00000001' },
    { ip: '10.20.0.102', mac: '3c5a00000002' },
  ],
  ownMac: '001a2b0000c1',
  ownVendor: 'Example Systems',
};

const ACCESS3: Discovery = {
  neighbors: [
    nb(
      'core-sw1',
      '10.0.0.1',
      'CS-3850-48P',
      'Bridge, Router',
      'GigabitEthernet 0/49',
      'GigabitEthernet1/0/47',
      '001a2b0000c1',
      'Example Systems',
    ),
    nb(
      'ap-floor3',
      '10.20.0.16',
      'RAP-62 wall',
      'WLAN Access Point',
      'GigabitEthernet 0/1',
      'Gi0',
      '5869aa003333',
      'Example Networks Co.',
    ),
    nb(
      'phone-3012',
      '10.30.0.12',
      'IP phone',
      'Telephone',
      'GigabitEthernet 0/7',
      'Port 1',
      null,
      null,
    ),
  ],
  macs: [
    mac('3c5a00000001', '20', 'GigabitEthernet 0/2', 'Example Computers'),
    mac('3c5a00000002', '20', 'GigabitEthernet 0/3', 'Example Computers'),
    ...Array.from({ length: 7 }, (_, i) =>
      mac(`a0b1c2d3e40${i}`, '30', 'GigabitEthernet 0/12', null),
    ),
    mac('001a2b0000c1', '1', 'GigabitEthernet 0/49', null),
  ],
  arp: [{ ip: '10.0.0.20', mac: '5869aa001111' }],
  ownMac: '5869aa001111',
  ownVendor: 'Example Networks Co.',
};

const OTHER: Discovery = {
  neighbors: [
    nb(
      'core-sw1',
      '10.0.0.1',
      'CS-3850-48P',
      'Bridge, Router',
      'GigabitEthernet0/1',
      'GigabitEthernet1/0/2',
      '001a2b0000c1',
      'Example Systems',
    ),
  ],
  macs: [],
  arp: [],
  ownMac: null,
  ownVendor: null,
};

export function mockDiscovery(host: string, hosts: boolean): Discovery {
  const h = host.toLowerCase();
  const d = h.includes('access-sw3') ? ACCESS3 : h.includes('core') ? CORE : OTHER;
  return hosts ? d : { ...d, macs: [], arp: d.arp };
}

/** Demo SNMP agent: a distribution switch behind core-sw1 that no session is open to. */
export function mockSnmp(host: string): SnmpDevice {
  return {
    name: host.includes('edge') ? 'edge-rtr' : 'dist-sw2',
    description: 'L3 switch software',
    discovery: {
      neighbors: [
        nb(
          'core-sw1',
          '10.0.0.1',
          'CS-3850-48P',
          'Bridge, Router',
          'Te1/0/1',
          'GigabitEthernet1/0/2',
          '001a2b0000c1',
          'Example Systems',
        ),
        nb(
          'ap-roof',
          '10.20.0.40',
          'Outdoor AP',
          'WLAN Access Point',
          'Te1/0/2',
          'eth0',
          '5869aa004444',
          'Example Networks Co.',
        ),
      ],
      macs: [mac('3c5a000000d5', '10', 'Gi1/0/10', 'Example Computers')],
      arp: [{ ip: '10.10.0.77', mac: '3c5a000000d5' }],
      ownMac: '001a2b0000d2',
      ownVendor: 'Example Systems',
    },
    interfaces: [
      [1, 'Te1/0/1'],
      [2, 'Te1/0/2'],
      [10, 'Gi1/0/10'],
    ],
  };
}

/** Demo flow collector: access-sw3 samples two stations, an unknown router exports one. */
export function mockFlow(running: boolean, sflowPort: number, netflowPort: number): FlowSnapshot {
  return {
    running,
    sflowPort: running && sflowPort ? sflowPort : null,
    netflowPort: running && netflowPort ? netflowPort : null,
    sflowDatagrams: running ? 42 : 0,
    netflowDatagrams: running ? 7 : 0,
    hosts: running
      ? [
          {
            agent: '10.0.0.20',
            ifIndex: 8,
            mac: '3c5a000000f8',
            ip: '10.20.0.108',
            vendor: 'Example Computers',
          },
          {
            agent: '10.0.0.20',
            ifIndex: 9,
            mac: '0011223344f9',
            ip: '10.20.0.109',
            vendor: 'Example Printers Ltd.',
          },
          { agent: '10.99.0.1', ifIndex: 3, mac: null, ip: '10.30.0.9', vendor: null },
        ]
      : [],
  };
}
