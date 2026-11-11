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
