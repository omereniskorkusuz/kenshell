/**
 * Topology map model: merges what every open session knows (CDP / LLDP neighbours, MAC and
 * ARP tables) into one graph, and lays it out as a hierarchy (root on top).
 *
 * A device is one node however many sessions see it: matched by name (without domain),
 * management address or chassis MAC. A link is one edge even when both ends report it.
 */
import { parseIface } from './iface';
import type { Discovery } from './ipc';

export type NodeKind = 'router' | 'switch' | 'ap' | 'phone' | 'firewall' | 'host' | 'group';

/** Where a device's data comes from. */
export type NodeSource = 'session' | 'snmp' | 'flow';

/** One device we can ask directly: an open session or an SNMP agent. */
export interface DeviceInput {
  /** Tab key, or `snmp:<host>`. */
  key: string;
  name: string;
  /** Address the session / the poll goes to (null for serial). */
  host: string | null;
  model: string | null;
  discovery: Discovery | null;
  /** Default: `session`. */
  source?: 'session' | 'snmp';
  /** ifIndex → port name, when known (names the ports flow records refer to). */
  interfaces?: readonly (readonly [number, string])[];
}

/** An address a flow exporter saw entering one of its ports. */
export interface FlowSighting {
  agent: string;
  ifIndex: number;
  mac: string | null;
  ip: string | null;
  vendor: string | null;
}

export interface TopoNode {
  id: string;
  name: string;
  kind: NodeKind;
  ip: string | null;
  /** Manufacturer, from the device's MAC (IEEE registry). */
  vendor: string | null;
  /** Model / platform text the device reports. */
  model: string | null;
  /** 12 hex digits. */
  mac: string | null;
  /** Tab key when one of the open sessions is this device. */
  tabKey: string | null;
  /** How we know the device first-hand (null: only reported by others). */
  source: NodeSource | null;
  /** `group` only: how many addresses were seen on the port. */
  count: number;
}

export interface TopoEdge {
  id: string;
  a: string;
  b: string;
  /** Port on `a` / on `b` (display form), when known. */
  aPort: string | null;
  bPort: string | null;
}

export interface TopoGraph {
  nodes: TopoNode[];
  edges: TopoEdge[];
}

/** Ports with more learned addresses than this become one "n devices" node. */
export const GROUP_OVER = 4;

const IPV4 = /^\d{1,3}(?:\.\d{1,3}){3}$/;
const nameKey = (name: string): string => (name.split('.')[0] ?? name).trim().toLowerCase();
/** An address that identifies a device: IPv4, not loopback (many sessions can go to it). */
const cleanIp = (ip: string | null | undefined): string | null => {
  const v = ip?.split('/')[0]?.trim() ?? '';
  return IPV4.test(v) && !v.startsWith('127.') && v !== '0.0.0.0' ? v : null;
};
const macKey = (mac: string | null | undefined): string | null => {
  const hex = (mac ?? '').replace(/[^0-9a-f]/gi, '').toLowerCase();
  return hex.length === 12 ? hex : null;
};
/** `port1`, `Port 1` are port names of their own, not a short form of Port-channel. */
const iface = (port: string) => (/^port ?\d/i.test(port.trim()) ? null : parseIface(port));
/** Same port, any spelling. */
const portKey = (port: string | null): string | null =>
  port === null ? null : (iface(port)?.key ?? port.replace(/\s+/g, '').toLowerCase());
export const portShort = (port: string | null): string | null =>
  port === null ? null : (iface(port)?.short ?? port.trim());

/** `aabbccddeeff` → `aa:bb:cc:dd:ee:ff`. */
export const formatMac = (mac: string): string => mac.replace(/(..)(?=.)/g, '$1:');

/** Device role from the advertised capabilities and platform text. */
export function kindOf(capability: string | null, platform: string | null): NodeKind | null {
  const c = capability ?? '';
  const has = (word: RegExp, letter: string) =>
    word.test(c) || new RegExp(`(^|[\\s,])${letter}($|[\\s,])`).test(c);
  if (/firewall|\bfw\b|utm/i.test(`${c} ${platform ?? ''}`)) return 'firewall';
  if (has(/phone|telephone/i, 'T')) return 'phone';
  if (has(/wlan|access.?point/i, 'W') || /(^|[\s-])(r?ap\d|air-)/i.test(platform ?? ''))
    return 'ap';
  const router = has(/router/i, 'R');
  const bridge = has(/switch|bridge/i, 'B') || has(/switch/i, 'S');
  if (bridge) return 'switch';
  if (router) return 'router';
  if (has(/host|station/i, 'H')) return 'host';
  return null;
}

interface Candidate {
  mac: string | null;
  ip: string | null;
  vendor: string | null;
}

export function buildGraph(
  devices: readonly DeviceInput[],
  opts: { hosts: boolean; flows?: readonly FlowSighting[] | undefined },
): TopoGraph {
  const nodes = new Map<string, TopoNode>();
  const byName = new Map<string, string>();
  const byIp = new Map<string, string>();
  const byMac = new Map<string, string>();
  const edges: TopoEdge[] = [];

  const index = (n: TopoNode, ...alsoIp: (string | null)[]) => {
    // Not an address standing in for a missing name.
    if (n.name && n.kind !== 'host' && !IPV4.test(n.name) && macKey(n.name) === null) {
      byName.set(nameKey(n.name), n.id);
    }
    for (const ip of [n.ip, ...alsoIp]) if (ip) byIp.set(ip, n.id);
    if (n.mac) byMac.set(n.mac, n.id);
  };

  // 1. The devices we can ask directly. A device reachable both ways (a session and an
  //    SNMP poll) is one node; sessions come first.
  const nodeOf = new Map<string, string>();
  const ifNames = new Map<string, Map<number, string>>();
  const ordered = [...devices].sort(
    (a, b) => Number(a.source === 'snmp') - Number(b.source === 'snmp'),
  );
  for (const d of ordered) {
    const source = d.source ?? 'session';
    const ip = cleanIp(d.host);
    const mac = macKey(d.discovery?.ownMac);
    // Same name or same chassis MAC: the same device. The same address alone only when
    // the names do not disagree (two devices can sit behind one address).
    const sameIp = ip ? byIp.get(ip) : undefined;
    const agrees = (id: string | undefined) =>
      id !== undefined &&
      (!d.name || !nodes.get(id)?.name || nameKey(nodes.get(id)?.name ?? '') === nameKey(d.name));
    const known =
      source === 'snmp'
        ? ((d.name ? byName.get(nameKey(d.name)) : undefined) ??
          (mac ? byMac.get(mac) : undefined) ??
          (agrees(sameIp) ? sameIp : undefined))
        : undefined;
    const id = known ?? `d:${d.key}`;
    const n: TopoNode = nodes.get(id) ?? {
      id,
      name: d.name || ip || d.key,
      kind: 'switch',
      ip,
      vendor: null,
      model: null,
      mac: null,
      tabKey: source === 'session' ? d.key : null,
      source,
      count: 0,
    };
    n.vendor ??= d.discovery?.ownVendor ?? null;
    n.model ??= d.model;
    n.mac ??= mac;
    n.ip ??= ip;
    nodes.set(id, n);
    nodeOf.set(d.key, id);
    index(n, ip);
    if (d.interfaces?.length) {
      const names = ifNames.get(id) ?? new Map<number, string>();
      for (const [i, name] of d.interfaces) names.set(i, name);
      ifNames.set(id, names);
    }
  }

  // Every MAC → IP anyone knows.
  const arp = new Map<string, string>();
  for (const d of devices) for (const e of d.discovery?.arp ?? []) arp.set(e.mac, e.ip);

  const link = (a: string, aPort: string | null, b: string, bPort: string | null) => {
    if (a === b) return;
    const [ak, bk] = [portKey(aPort), portKey(bPort)];
    const fits = (x: string | null, y: string | null) => x === null || y === null || x === y;
    for (const e of edges) {
      const same = e.a === a && e.b === b;
      const flipped = e.a === b && e.b === a;
      if (!same && !flipped) continue;
      const [ea, eb] = same ? [e.aPort, e.bPort] : [e.bPort, e.aPort];
      if (!fits(portKey(ea), ak) || !fits(portKey(eb), bk)) continue;
      // The same link seen again (other protocol, other end or other source): fill in
      // what was missing.
      if (same) {
        e.aPort ??= portShort(aPort);
        e.bPort ??= portShort(bPort);
      } else {
        e.bPort ??= portShort(aPort);
        e.aPort ??= portShort(bPort);
      }
      return;
    }
    edges.push({
      id: `e${edges.length}`,
      a,
      b,
      aPort: portShort(aPort),
      bPort: portShort(bPort),
    });
  };

  // 2. Neighbours: an existing node (another source, or seen before) or a new one.
  const linkPorts = new Map<string, Set<string>>();
  for (const d of ordered) {
    const self = nodeOf.get(d.key) as string;
    const used = linkPorts.get(self) ?? new Set<string>();
    linkPorts.set(self, used);
    for (const nb of d.discovery?.neighbors ?? []) {
      const mac = macKey(nb.chassis);
      const ip = cleanIp(nb.ip) ?? (mac ? (arp.get(mac) ?? null) : null);
      // A device that announces no name comes through named by its chassis id.
      const named = macKey(nb.name) === null ? nb.name : '';
      const key = nameKey(named);
      // The same name is the same device only if the chassis MACs do not disagree:
      // several devices often keep the factory name.
      const fits = (id: string | undefined) => {
        const other = id === undefined ? undefined : nodes.get(id);
        return other !== undefined && (!mac || !other.mac || other.mac === mac) ? id : undefined;
      };
      let id =
        (mac ? byMac.get(mac) : undefined) ??
        fits(key ? byName.get(key) : undefined) ??
        fits(ip ? byIp.get(ip) : undefined);
      const kind = kindOf(nb.capability, nb.platform);
      if (!id) {
        id = `n:${mac || key || ip || nodes.size}`;
        nodes.set(id, {
          id,
          name: (named.split('.')[0] ?? named) || ip || (mac ? formatMac(mac) : ''),
          kind: kind ?? 'switch',
          ip,
          vendor: nb.vendor,
          model: nb.platform,
          mac,
          tabKey: null,
          source: null,
          count: 0,
        });
      }
      const n = nodes.get(id) as TopoNode;
      // Another device describes this one: take what we did not know ourselves.
      n.ip ??= ip;
      n.vendor ??= nb.vendor;
      n.model ??= nb.platform;
      n.mac ??= mac;
      if (kind && (n.source === null || kind !== 'host')) n.kind = kind;
      // The advertised management address also identifies it (a session may go to
      // another of its addresses).
      index(n, ip);
      const pk = portKey(nb.localInterface);
      if (pk) used.add(pk);
      link(self, nb.localInterface, id, nb.remoteInterface);
    }
  }

  // 3. End devices: addresses on ports that are not links to a known device, from MAC
  //    tables and from flow exports.
  if (opts.hosts) {
    const perPort = new Map<string, Map<string, { port: string; seen: Candidate[] }>>();
    const add = (node: string, port: string, c: Candidate) => {
      const pk = portKey(port) ?? port;
      const used = linkPorts.get(node);
      if (used?.has(pk)) return;
      // An aggregate without a neighbour entry on a device that has links is an uplink
      // whose members carry the neighbour: its addresses belong to the other side.
      if (used?.size && /^(port-?channel|aggregateport|bundle|ae|po|ag)\d/i.test(pk)) return;
      // An address of one of the known devices is not an end device.
      if ((c.mac && byMac.has(c.mac)) || (c.ip && byIp.has(c.ip))) return;
      const ports = perPort.get(node) ?? new Map<string, { port: string; seen: Candidate[] }>();
      perPort.set(node, ports);
      const g = ports.get(pk) ?? { port, seen: [] };
      ports.set(pk, g);
      const again = g.seen.find((x) => (c.mac ? x.mac === c.mac : x.mac === null && x.ip === c.ip));
      if (again) {
        again.ip ??= c.ip;
        again.vendor ??= c.vendor;
      } else {
        g.seen.push({ ...c });
      }
    };

    for (const d of ordered) {
      const self = nodeOf.get(d.key) as string;
      for (const m of d.discovery?.macs ?? []) {
        add(self, m.interface, { mac: m.mac, ip: arp.get(m.mac) ?? null, vendor: m.vendor });
      }
    }
    for (const f of opts.flows ?? []) {
      const agent = cleanIp(f.agent) ?? f.agent;
      let id = byIp.get(agent);
      if (!id) {
        // An exporter nobody else told us about: it gets a node of its own.
        id = `f:${agent}`;
        const n: TopoNode = {
          id,
          name: agent,
          kind: 'router',
          ip: cleanIp(agent),
          vendor: null,
          model: null,
          mac: null,
          tabKey: null,
          source: 'flow',
          count: 0,
        };
        nodes.set(id, n);
        index(n);
      }
      const mac = macKey(f.mac);
      add(id, ifNames.get(id)?.get(f.ifIndex) ?? `if${f.ifIndex}`, {
        mac,
        ip: cleanIp(f.ip) ?? (mac ? (arp.get(mac) ?? null) : null),
        vendor: f.vendor,
      });
    }

    for (const [self, ports] of perPort) {
      for (const [pk, g] of ports) {
        if (g.seen.length > GROUP_OVER) {
          const id = `g:${self}:${pk}`;
          nodes.set(id, {
            id,
            name: '',
            kind: 'group',
            ip: null,
            vendor: null,
            model: null,
            mac: null,
            tabKey: null,
            source: null,
            count: g.seen.length,
          });
          link(self, g.port, id, null);
          continue;
        }
        for (const c of g.seen) {
          const id = c.mac ? `h:${c.mac}` : `hi:${c.ip}`;
          // Already placed on another device's port (the first, most specific port wins).
          if (nodes.has(id)) continue;
          nodes.set(id, {
            id,
            name: c.ip ?? (c.mac ? formatMac(c.mac) : ''),
            kind: 'host',
            ip: c.ip,
            vendor: c.vendor,
            model: null,
            mac: c.mac,
            tabKey: null,
            source: null,
            count: 0,
          });
          link(self, g.port, id, null);
        }
      }
    }
  }

  return { nodes: [...nodes.values()], edges };
}

// ---------------------------------------------------------------------------------------
// Layout

export interface Placed {
  x: number;
  y: number;
  level: number;
}

export interface TopoLayout {
  pos: Map<string, Placed>;
  width: number;
  height: number;
  /** Edges of the hierarchy (parent → child), in drawing order. */
  tree: { edge: TopoEdge; parent: string; child: string }[];
  /** Links that are not part of the hierarchy (redundant paths). */
  extra: TopoEdge[];
  roots: string[];
}

export const NODE_W = 184;
export const NODE_H = 84;
export const GAP_X = 24;
export const GAP_Y = 96;

const naturalPort = (a: string | null, b: string | null): number =>
  (a ?? '').localeCompare(b ?? '', undefined, { numeric: true });

/** The most central infrastructure device: routers first, then by number of links. */
export function pickRoot(graph: TopoGraph, among?: ReadonlySet<string>): string | null {
  const degree = new Map<string, number>();
  // Links to other switches / routers count most: the device that ties the
  // infrastructure together belongs on top, not the one with the most stations.
  const infra = new Map<string, number>();
  const kinds = new Map(graph.nodes.map((n) => [n.id, n.kind]));
  const isInfra = (id: string) => ['router', 'switch', 'firewall'].includes(kinds.get(id) ?? '');
  for (const e of graph.edges) {
    degree.set(e.a, (degree.get(e.a) ?? 0) + 1);
    degree.set(e.b, (degree.get(e.b) ?? 0) + 1);
    if (isInfra(e.a) && isInfra(e.b)) {
      infra.set(e.a, (infra.get(e.a) ?? 0) + 1);
      infra.set(e.b, (infra.get(e.b) ?? 0) + 1);
    }
  }
  const rank: Record<NodeKind, number> = {
    router: 3,
    firewall: 3,
    switch: 2,
    ap: 1,
    phone: 0,
    host: 0,
    group: 0,
  };
  let best: TopoNode | null = null;
  let bestScore = -1;
  for (const n of graph.nodes) {
    if (among && !among.has(n.id)) continue;
    const score =
      rank[n.kind] * 10_000 +
      (infra.get(n.id) ?? 0) * 100 +
      (degree.get(n.id) ?? 0) * 2 +
      (n.source ? 1 : 0);
    if (score > bestScore) {
      best = n;
      bestScore = score;
    }
  }
  return best?.id ?? null;
}

/**
 * Hierarchical layout: breadth-first levels from the root, children ordered by the
 * parent's port, parents centred over their children. Unconnected parts get their own
 * root and are placed side by side.
 */
export function layoutGraph(graph: TopoGraph, root: string | null = null): TopoLayout {
  const adj = new Map<string, { edge: TopoEdge; other: string; port: string | null }[]>();
  for (const n of graph.nodes) adj.set(n.id, []);
  for (const e of graph.edges) {
    adj.get(e.a)?.push({ edge: e, other: e.b, port: e.aPort });
    adj.get(e.b)?.push({ edge: e, other: e.a, port: e.bPort });
  }
  for (const list of adj.values()) list.sort((x, y) => naturalPort(x.port, y.port));

  const level = new Map<string, number>();
  const children = new Map<string, string[]>();
  const tree: TopoLayout['tree'] = [];
  const treeEdges = new Set<string>();
  const roots: string[] = [];

  const grow = (start: string) => {
    roots.push(start);
    level.set(start, 0);
    const queue = [start];
    for (let i = 0; i < queue.length; i++) {
      const id = queue[i] as string;
      for (const { edge, other } of adj.get(id) ?? []) {
        if (level.has(other)) continue;
        level.set(other, (level.get(id) ?? 0) + 1);
        children.set(id, [...(children.get(id) ?? []), other]);
        tree.push({ edge, parent: id, child: other });
        treeEdges.add(edge.id);
        queue.push(other);
      }
    }
  };

  if (root && adj.has(root)) grow(root);
  for (;;) {
    const rest = new Set(graph.nodes.filter((n) => !level.has(n.id)).map((n) => n.id));
    if (rest.size === 0) break;
    grow(pickRoot(graph, rest) as string);
  }

  // Leaves take the next free column; a parent sits over the middle of its children.
  const pos = new Map<string, Placed>();
  let column = 0;
  let depth = 0;
  const place = (id: string): number => {
    const kids = children.get(id) ?? [];
    const lv = level.get(id) ?? 0;
    depth = Math.max(depth, lv);
    let x: number;
    if (kids.length === 0) {
      x = column * (NODE_W + GAP_X);
      column += 1;
    } else {
      const xs = kids.map(place);
      x = ((xs[0] as number) + (xs[xs.length - 1] as number)) / 2;
    }
    pos.set(id, { x, y: lv * (NODE_H + GAP_Y), level: lv });
    return x;
  };
  for (const r of roots) place(r);

  return {
    pos,
    width: Math.max(1, column) * (NODE_W + GAP_X) - GAP_X,
    height: (depth + 1) * NODE_H + depth * GAP_Y,
    tree,
    extra: graph.edges.filter((e) => !treeEdges.has(e.id)),
    roots,
  };
}
