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

/** One open session's contribution. */
export interface DeviceInput {
  /** Tab key. */
  key: string;
  name: string;
  /** Address the session connects to (null for serial). */
  host: string | null;
  model: string | null;
  discovery: Discovery | null;
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
  /** `group` only: how many addresses were learned on the port. */
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
const cleanIp = (ip: string | null | undefined): string | null => {
  const v = ip?.split('/')[0]?.trim() ?? '';
  return IPV4.test(v) ? v : null;
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

export function buildGraph(devices: readonly DeviceInput[], opts: { hosts: boolean }): TopoGraph {
  const nodes = new Map<string, TopoNode>();
  const byName = new Map<string, string>();
  const byIp = new Map<string, string>();
  const byMac = new Map<string, string>();
  const edges: TopoEdge[] = [];

  const index = (n: TopoNode) => {
    if (n.name) byName.set(nameKey(n.name), n.id);
    if (n.ip) byIp.set(n.ip, n.id);
    if (n.mac) byMac.set(n.mac, n.id);
  };

  // 1. The devices we are connected to.
  for (const d of devices) {
    const id = `d:${d.key}`;
    const n: TopoNode = {
      id,
      name: d.name,
      kind: 'switch',
      ip: cleanIp(d.host),
      vendor: d.discovery?.ownVendor ?? null,
      model: d.model,
      mac: macKey(d.discovery?.ownMac),
      tabKey: d.key,
      count: 0,
    };
    nodes.set(id, n);
    index(n);
  }

  // Every MAC → IP any session knows.
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
      // The same link seen again (other protocol or other end): fill in what was missing.
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

  // 2. Neighbours: an existing node (another session, or seen before) or a new one.
  const linkPorts = new Map<string, Set<string>>();
  for (const d of devices) {
    const self = `d:${d.key}`;
    const used = new Set<string>();
    linkPorts.set(self, used);
    for (const nb of d.discovery?.neighbors ?? []) {
      const ip = cleanIp(nb.ip);
      const mac = macKey(nb.chassis);
      const key = nameKey(nb.name);
      let id =
        (key ? byName.get(key) : undefined) ??
        (mac ? byMac.get(mac) : undefined) ??
        (ip ? byIp.get(ip) : undefined);
      const kind = kindOf(nb.capability, nb.platform);
      if (!id) {
        id = `n:${key || mac || ip || nodes.size}`;
        nodes.set(id, {
          id,
          name: nb.name.split('.')[0] ?? nb.name,
          kind: kind ?? 'switch',
          ip,
          vendor: nb.vendor,
          model: nb.platform,
          mac,
          tabKey: null,
          count: 0,
        });
      }
      const n = nodes.get(id) as TopoNode;
      // Another device describes this one: take what we did not know ourselves.
      n.ip ??= ip;
      n.vendor ??= nb.vendor;
      n.model ??= nb.platform;
      n.mac ??= mac;
      if (kind && (n.tabKey === null || kind !== 'host')) n.kind = kind;
      index(n);
      const pk = portKey(nb.localInterface);
      if (pk) used.add(pk);
      link(self, nb.localInterface, id, nb.remoteInterface);
    }
  }

  // 3. End devices: addresses learned on ports that are not links to a known device.
  if (opts.hosts) {
    for (const d of devices) {
      const self = `d:${d.key}`;
      const used = linkPorts.get(self) ?? new Set<string>();
      const hasLinks = used.size > 0;
      const perPort = new Map<string, { port: string; macs: Discovery['macs'] }>();
      for (const m of d.discovery?.macs ?? []) {
        const pk = portKey(m.interface) ?? m.interface;
        if (used.has(pk)) continue;
        // An aggregate without a neighbour entry on a device that has links is an uplink
        // whose members carry the neighbour: its addresses belong to the other side.
        if (hasLinks && /^(port-?channel|aggregateport|bundle|ae|po|ag)\d/i.test(pk)) continue;
        // A MAC that is one of the known devices is not an end device.
        if (byMac.has(m.mac)) continue;
        const g = perPort.get(pk) ?? { port: m.interface, macs: [] };
        g.macs.push(m);
        perPort.set(pk, g);
      }
      for (const [pk, g] of perPort) {
        if (g.macs.length > GROUP_OVER) {
          const id = `g:${d.key}:${pk}`;
          nodes.set(id, {
            id,
            name: '',
            kind: 'group',
            ip: null,
            vendor: null,
            model: null,
            mac: null,
            tabKey: null,
            count: g.macs.length,
          });
          link(self, g.port, id, null);
          continue;
        }
        for (const m of g.macs) {
          const id = `h:${m.mac}`;
          // Already placed on another device's port (the first, most specific port wins).
          if (nodes.has(id)) continue;
          const ip = arp.get(m.mac) ?? null;
          nodes.set(id, {
            id,
            name: ip ?? formatMac(m.mac),
            kind: 'host',
            ip,
            vendor: m.vendor,
            model: null,
            mac: m.mac,
            tabKey: null,
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
  for (const e of graph.edges) {
    degree.set(e.a, (degree.get(e.a) ?? 0) + 1);
    degree.set(e.b, (degree.get(e.b) ?? 0) + 1);
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
    const score = rank[n.kind] * 1000 + (degree.get(n.id) ?? 0) * 2 + (n.tabKey ? 1 : 0);
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
