import { describe, expect, it } from 'vitest';
import type { Discovery, Neighbor } from './ipc';
import { mockDiscovery } from './mock/topology';
import {
  buildGraph,
  GROUP_OVER,
  kindOf,
  layoutGraph,
  NODE_W,
  pickRoot,
  portShort,
  type DeviceInput,
} from './topologyGraph';

const nb = (over: Partial<Neighbor>): Neighbor => ({
  name: 'x',
  ip: null,
  platform: null,
  capability: null,
  localInterface: 'Gi0/1',
  remoteInterface: null,
  cdp: false,
  lldp: true,
  chassis: null,
  vendor: null,
  ...over,
});
const disc = (over: Partial<Discovery>): Discovery => ({
  neighbors: [],
  macs: [],
  arp: [],
  ownMac: null,
  ownVendor: null,
  ...over,
});
const dev = (key: string, name: string, discovery: Discovery | null, host: string | null = null) =>
  ({ key, name, host, model: null, discovery }) satisfies DeviceInput;

describe('topology graph', () => {
  it('two sessions that see each other are two nodes and one link with both ports', () => {
    const g = buildGraph(
      [
        dev(
          'a',
          'core-sw1',
          disc({
            neighbors: [
              nb({
                name: 'access-sw3.lab.example.com',
                localInterface: 'GigabitEthernet1/0/47',
                remoteInterface: 'GigabitEthernet 0/49',
              }),
            ],
          }),
        ),
        dev(
          'b',
          'access-sw3',
          disc({
            neighbors: [
              nb({ name: 'core-sw1', localInterface: 'Gi0/49', remoteInterface: 'Gi1/0/47' }),
            ],
          }),
        ),
      ],
      { hosts: false },
    );
    expect(g.nodes.map((n) => n.id).sort()).toEqual(['d:a', 'd:b']);
    expect(g.edges).toHaveLength(1);
    expect(g.edges[0]).toMatchObject({ a: 'd:a', b: 'd:b', aPort: 'Gi1/0/47', bPort: 'Gi0/49' });
  });

  it('matches a neighbour to an open session by address or chassis MAC, not only by name', () => {
    const g = buildGraph(
      [
        dev(
          'a',
          'core',
          disc({
            neighbors: [
              nb({ name: 'SW-FLOOR-2', ip: '10.0.0.20' }),
              nb({ name: 'other-name', chassis: 'aabb.ccdd.eeff', localInterface: 'Gi0/2' }),
            ],
          }),
        ),
        dev('b', 'edge-b', disc({}), '10.0.0.20'),
        dev('c', 'edge-c', disc({ ownMac: 'aabbccddeeff' })),
      ],
      { hosts: false },
    );
    expect(g.nodes).toHaveLength(3);
    expect(g.edges.map((e) => e.b).sort()).toEqual(['d:b', 'd:c']);
  });

  it('parallel links between the same two devices stay separate', () => {
    const g = buildGraph(
      [
        dev(
          'a',
          'core',
          disc({
            neighbors: [
              nb({ name: 'dist', localInterface: 'Gi0/1', remoteInterface: 'Gi0/1' }),
              nb({ name: 'dist', localInterface: 'Gi0/2', remoteInterface: 'Gi0/2' }),
            ],
          }),
        ),
      ],
      { hosts: false },
    );
    expect(g.nodes).toHaveLength(2);
    expect(g.edges).toHaveLength(2);
  });

  it('end devices: single addresses become hosts with their IP, crowded ports one group, link ports none', () => {
    const many = Array.from({ length: GROUP_OVER + 1 }, (_, i) => ({
      mac: `0000000000a${i}`,
      vlan: '1',
      interface: 'Gi0/9',
      vendor: null,
    }));
    const d = disc({
      neighbors: [nb({ name: 'core', localInterface: 'Gi0/49' })],
      macs: [
        { mac: '001122334455', vlan: '10', interface: 'GigabitEthernet0/2', vendor: 'Acme' },
        { mac: 'ffeeddccbbaa', vlan: '10', interface: 'Gi0/49', vendor: null },
        { mac: '0a0b0c0d0e0f', vlan: '10', interface: 'Port-channel1', vendor: null },
        ...many,
      ],
      arp: [{ ip: '10.1.1.7', mac: '001122334455' }],
    });
    const g = buildGraph([dev('a', 'acc', d)], { hosts: true });
    const host = g.nodes.find((n) => n.kind === 'host');
    expect(host).toMatchObject({ name: '10.1.1.7', vendor: 'Acme', mac: '001122334455' });
    expect(g.nodes.filter((n) => n.kind === 'host')).toHaveLength(1);
    expect(g.nodes.find((n) => n.kind === 'group')?.count).toBe(GROUP_OVER + 1);
    expect(g.edges.find((e) => e.b === host?.id)?.aPort).toBe('Gi0/2');
    expect(buildGraph([dev('a', 'acc', d)], { hosts: false }).nodes).toHaveLength(2);
  });

  it('roles and port names', () => {
    expect(kindOf('Router Switch IGMP', null)).toBe('switch');
    expect(kindOf('R', null)).toBe('router');
    expect(kindOf('B,R', null)).toBe('switch');
    expect(kindOf('WLAN Access Point', null)).toBe('ap');
    expect(kindOf('Trans-Bridge', 'AIR-AP2802I')).toBe('ap');
    expect(kindOf('Telephone', null)).toBe('phone');
    expect(kindOf('Router', 'FW-1500 firewall')).toBe('firewall');
    expect(kindOf(null, null)).toBeNull();
    expect(portShort('GigabitEthernet 0/49')).toBe('Gi0/49');
    expect(portShort('port1')).toBe('port1');
  });
});

describe('topology layout', () => {
  const devices = [
    dev('a', 'core-sw1', mockDiscovery('core-sw1', true), '10.0.0.1'),
    dev('b', 'access-sw3', mockDiscovery('access-sw3', true), '10.0.0.20'),
  ];
  const g = buildGraph(devices, { hosts: true });

  it('the demo campus: every device once, the firewall on top', () => {
    const names = g.nodes.map((n) => n.name);
    expect(names.filter((n) => n === 'core-sw1')).toHaveLength(1);
    expect(names.filter((n) => n === 'access-sw3')).toHaveLength(1);
    const root = pickRoot(g);
    expect(g.nodes.find((n) => n.id === root)?.name).toBe('fw-edge');
  });

  it('levels follow the links, nothing overlaps, every node is placed', () => {
    const l = layoutGraph(g);
    expect(l.pos.size).toBe(g.nodes.length);
    expect(l.tree).toHaveLength(g.nodes.length - l.roots.length);
    const id = (name: string) => g.nodes.find((n) => n.name === name)?.id ?? '';
    expect(l.pos.get(id('fw-edge'))?.level).toBe(0);
    expect(l.pos.get(id('core-sw1'))?.level).toBe(1);
    expect(l.pos.get(id('access-sw3'))?.level).toBe(2);
    expect(l.pos.get(id('ap-floor3'))?.level).toBe(3);
    const byLevel = new Map<number, number[]>();
    for (const p of l.pos.values()) byLevel.set(p.level, [...(byLevel.get(p.level) ?? []), p.x]);
    for (const xs of byLevel.values()) {
      xs.sort((a, b) => a - b);
      for (let i = 1; i < xs.length; i++)
        expect((xs[i] as number) - (xs[i - 1] as number)).toBeGreaterThanOrEqual(NODE_W);
    }
  });

  it('a chosen root goes to the top; separate islands and loops are handled', () => {
    const core = g.nodes.find((n) => n.name === 'core-sw1')?.id ?? '';
    expect(layoutGraph(g, core).pos.get(core)?.level).toBe(0);
    const loop = buildGraph(
      [
        dev(
          'a',
          'a',
          disc({
            neighbors: [
              nb({ name: 'b', localInterface: 'Gi0/1' }),
              nb({ name: 'c', localInterface: 'Gi0/2' }),
            ],
          }),
        ),
        dev('b', 'b', disc({ neighbors: [nb({ name: 'c', localInterface: 'Gi0/3' })] })),
        dev('z', 'alone', disc({})),
      ],
      { hosts: false },
    );
    const l = layoutGraph(loop);
    expect(l.roots).toHaveLength(2);
    expect(l.extra).toHaveLength(1);
    expect(l.pos.size).toBe(4);
    expect(layoutGraph({ nodes: [], edges: [] }).pos.size).toBe(0);
  });
});

describe('other sources', () => {
  const session = dev(
    'a',
    'core-sw1',
    disc({
      ownMac: '001a2b0000c1',
      neighbors: [
        nb({
          name: 'dist-sw2.lab',
          ip: '10.0.0.2',
          localInterface: 'Gi1/0/2',
          remoteInterface: 'Te1/0/1',
        }),
        nb({ name: 'access-sw3', ip: '10.0.0.20', localInterface: 'Gi1/0/47' }),
      ],
    }),
    '192.168.50.1',
  );
  const snmp: DeviceInput = {
    key: 'snmp:10.0.0.2',
    name: 'dist-sw2',
    host: '10.0.0.2',
    model: null,
    source: 'snmp',
    interfaces: [[10, 'Gi1/0/10']],
    discovery: disc({
      neighbors: [
        nb({ name: 'core-sw1', localInterface: 'Te1/0/1', remoteInterface: 'Gi1/0/2' }),
        nb({ name: 'ap-roof', localInterface: 'Te1/0/2' }),
      ],
      macs: [{ mac: '3c5a000000d5', vlan: '10', interface: 'Gi1/0/10', vendor: 'Acme' }],
    }),
  };

  it('an SNMP device joins the neighbour another source reported, with one link', () => {
    const g = buildGraph([snmp, session], { hosts: true });
    const dist = g.nodes.filter((n) => n.name.startsWith('dist-sw2'));
    expect(dist).toHaveLength(1);
    expect(dist[0]).toMatchObject({ source: 'snmp', tabKey: null, ip: '10.0.0.2' });
    expect(
      g.edges.filter((e) => [e.a, e.b].includes(dist[0]?.id ?? '') && [e.a, e.b].includes('d:a')),
    ).toHaveLength(1);
    expect(g.nodes.find((n) => n.name === 'ap-roof')).toBeTruthy();
    expect(g.nodes.find((n) => n.mac === '3c5a000000d5')?.kind).toBe('host');
  });

  it('a device with both a session and an SNMP poll is one node', () => {
    const same: DeviceInput = { ...snmp, key: 'snmp:x', name: 'CORE-SW1', discovery: disc({}) };
    const g = buildGraph([same, session], { hosts: false });
    expect(g.nodes.filter((n) => n.name.toLowerCase() === 'core-sw1')).toHaveLength(1);
    expect(g.nodes.find((n) => n.id === 'd:a')?.source).toBe('session');
  });

  it('flow sightings hang off the exporter port: by advertised address, named by ifIndex', () => {
    const flows = [
      { agent: '10.0.0.2', ifIndex: 10, mac: '3c5a000000d5', ip: '10.10.0.77', vendor: null },
      { agent: '10.0.0.2', ifIndex: 11, mac: 'aabbcc000011', ip: null, vendor: 'Acme' },
      { agent: '10.0.0.20', ifIndex: 7, mac: null, ip: '10.20.0.107', vendor: null },
      // The exporter's own neighbour and an unknown exporter.
      { agent: '10.0.0.2', ifIndex: 12, mac: '001a2b0000c1', ip: null, vendor: null },
      { agent: '10.99.0.1', ifIndex: 3, mac: null, ip: '10.30.0.9', vendor: null },
    ];
    const g = buildGraph([session, snmp], { hosts: true, flows });
    const edgeTo = (id: string) => g.edges.find((e) => e.b === id);
    // Seen in the MAC table and in a flow: one host, now with its address.
    const pc = g.nodes.filter((n) => n.mac === '3c5a000000d5');
    expect(pc).toHaveLength(1);
    expect(pc[0]?.ip).toBe('10.10.0.77');
    expect(edgeTo('h:3c5a000000d5')?.aPort).toBe('Gi1/0/10');
    // No interface name known: the ifIndex.
    expect(edgeTo('h:aabbcc000011')?.aPort).toBe('if11');
    // access-sw3 is known only through its advertised address.
    const access = g.nodes.find((n) => n.name === 'access-sw3');
    expect(edgeTo('hi:10.20.0.107')).toMatchObject({ a: access?.id, aPort: 'if7' });
    expect(g.nodes.some((n) => n.id === 'h:001a2b0000c1')).toBe(false);
    expect(g.nodes.find((n) => n.id === 'f:10.99.0.1')).toMatchObject({
      source: 'flow',
      kind: 'router',
    });
    expect(
      buildGraph([session, snmp], { hosts: false, flows }).nodes.some((n) => n.kind === 'host'),
    ).toBe(false);
  });
});
