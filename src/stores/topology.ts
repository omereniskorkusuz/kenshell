/**
 * Topology map data, per source: what each open session reported (read hidden with
 * read-only commands), what each SNMP host answered, and what the flow collector has
 * seen. The graph itself is derived in `lib/topologyGraph.ts`.
 */
import { create } from 'zustand';
import { t } from '../i18n';
import { describeError } from '../lib/errors';
import { CRAWL_MAX, CRAWL_PARALLEL, nextCandidates, type Known } from '../lib/crawl';
import {
  api,
  type Discovery,
  type FlowSnapshot,
  type SnmpCredentials,
  type SnmpDevice,
  type SnmpV3User,
} from '../lib/ipc';
import { getSetting, useSettings } from './settings';
import { useTabs } from './tabs';

export interface TopoEntry {
  discovery: Discovery | null;
  busy: boolean;
  error: string | null;
  /** Session generation the data belongs to (a reconnect reads again). */
  runtimeId: number | null;
  /** When it was last read (ms). */
  readAt: number | null;
}

/**
 * MAC and ARP entries age out of a switch within minutes. The map remembers every end
 * device it has seen (per reporting device) with the time it was last seen, so a quiet
 * printer does not vanish between two reads; old ones are drawn faded.
 */
export interface Sightings {
  macs: Map<string, Discovery['macs'][number] & { at: number }>;
  arp: Map<string, { ip: string; at: number }>;
}
/** Forgotten after this long without being seen again. */
export const SIGHTING_TTL_MS = 24 * 60 * 60 * 1000;

export function remember(prev: Sightings | undefined, d: Discovery, now: number): Sightings {
  const macs = new Map(prev?.macs);
  const arp = new Map(prev?.arp);
  for (const m of d.macs) macs.set(`${m.mac}|${m.interface}`, { ...m, at: now });
  for (const a of d.arp) arp.set(a.mac, { ip: a.ip, at: now });
  for (const [k, v] of macs) if (now - v.at > SIGHTING_TTL_MS) macs.delete(k);
  for (const [k, v] of arp) if (now - v.at > SIGHTING_TTL_MS) arp.delete(k);
  // A MAC that moved to another port is only where it was seen last.
  const latest = new Map<string, number>();
  for (const v of macs.values()) latest.set(v.mac, Math.max(latest.get(v.mac) ?? 0, v.at));
  for (const [k, v] of macs) if (v.at < (latest.get(v.mac) ?? 0)) macs.delete(k);
  return { macs, arp };
}

/** A discovery with every remembered end device, and when each was last seen. */
export function withSightings(
  d: Discovery,
  mem: Sightings | undefined,
): { discovery: Discovery; seen: Map<string, number> } {
  if (!mem) return { discovery: d, seen: new Map() };
  const seen = new Map<string, number>();
  for (const v of mem.macs.values()) seen.set(v.mac, v.at);
  return {
    discovery: {
      ...d,
      macs: [...mem.macs.values()].map((m) => ({
        mac: m.mac,
        vlan: m.vlan,
        interface: m.interface,
        vendor: m.vendor,
      })),
      arp: [...mem.arp].map(([mac, v]) => ({ mac, ip: v.ip })),
    },
    seen,
  };
}

export interface CrawlEntry {
  device: SnmpDevice | null;
  busy: boolean;
  error: string | null;
  /** Name the device was announced with, and who announced it. */
  name: string;
  from: string;
  depth: number;
}

export interface SnmpEntry {
  device: SnmpDevice | null;
  busy: boolean;
  error: string | null;
}

interface TopologyState {
  byTab: Record<string, TopoEntry>;
  /** Remembered end devices per tab / SNMP host (see `Sightings`). */
  sightings: Record<string, Sightings>;
  /** SNMP reads of the open sessions' own devices (`topology.snmpSessions`). */
  sessionSnmp: Record<string, SnmpEntry>;
  /** SNMP hosts (as typed) → what they answered. */
  snmp: Record<string, SnmpEntry>;
  /** Some SNMP credential (community or v3 user) is stored (null = not asked yet). */
  community: boolean | null;
  /** What is stored, for the Sources panel (never a password). */
  creds: SnmpCredentials | null;
  /** Devices found by automatic discovery, by address. */
  crawled: Record<string, CrawlEntry>;
  /** Discovery is following neighbours right now. */
  crawling: boolean;
  startCrawl(): void;
  stopCrawl(): void;
  setV3(user: SnmpV3User): Promise<void>;
  clearV3(): Promise<void>;
  flow: FlowSnapshot | null;
  flowError: string | null;
  /** Also read MAC / ARP tables and show end devices. */
  hosts: boolean;
  /** Node chosen as the top of the hierarchy (null = automatic). */
  root: string | null;
  setHosts(on: boolean): void;
  setRoot(id: string | null): void;
  /** Reads every source (`force`), or only what has not been read yet. */
  refresh(force?: boolean): void;
  /** Once per map opening: community state, and the collector if it is switched on. */
  loadSources(): Promise<void>;
  setCommunity(value: string): Promise<void>;
  clearCommunity(): Promise<void>;
  addSnmpHost(host: string): void;
  removeSnmpHost(host: string): void;
  setFlow(enabled: boolean): Promise<void>;
  /** Fetches what the collector has seen (no-op while it is off). */
  pollFlow(): Promise<void>;
}

export const useTopology = create<TopologyState>()((set, get) => {
  const patch = (key: string, next: Partial<TopoEntry>) =>
    set((s) => ({
      byTab: {
        ...s.byTab,
        [key]: {
          discovery: null,
          busy: false,
          error: null,
          runtimeId: null,
          readAt: null,
          ...s.byTab[key],
          ...next,
        },
      },
    }));
  const patchSnmp = (host: string, next: Partial<SnmpEntry>) =>
    set((s) => ({
      snmp: {
        ...s.snmp,
        [host]: { device: null, busy: false, error: null, ...s.snmp[host], ...next },
      },
    }));

  const read = async (key: string, runtimeId: number, family: string | null) => {
    const hosts = get().hosts;
    patch(key, { busy: true, error: null, runtimeId });
    try {
      const discovery = await api.topologyDiscover(runtimeId, family, hosts);
      const now = Date.now();
      patch(key, { discovery, busy: false, readAt: now });
      queueMicrotask(pump);
      set((s) => ({
        sightings: { ...s.sightings, [key]: remember(s.sightings[key], discovery, now) },
      }));
    } catch (e) {
      patch(key, { busy: false, error: describeError(e, t).message });
    }
  };

  const readSnmp = async (host: string) => {
    patchSnmp(host, { busy: true, error: null });
    try {
      const device = await api.topologySnmp(host);
      // Removed while it was being read?
      if (getSetting('topology.snmpHosts').includes(host)) {
        patchSnmp(host, { device, busy: false });
        queueMicrotask(pump);
        const now = Date.now();
        set((s) => ({
          sightings: {
            ...s.sightings,
            [`snmp:${host}`]: remember(s.sightings[`snmp:${host}`], device.discovery, now),
          },
        }));
      }
    } catch (e) {
      patchSnmp(host, { busy: false, error: describeError(e, t).message });
    }
  };

  /** SNMP for a device we already have a session to: more tables, no CLI parsing. */
  const readSessionSnmp = async (key: string, host: string) => {
    set((s) => ({
      sessionSnmp: {
        ...s.sessionSnmp,
        [key]: { device: null, error: null, ...s.sessionSnmp[key], busy: true },
      },
    }));
    try {
      const device = await api.topologySnmp(host);
      const now = Date.now();
      set((s) => ({
        sessionSnmp: { ...s.sessionSnmp, [key]: { device, busy: false, error: null } },
        // (the crawl continues from what this device reported)
        sightings: {
          ...s.sightings,
          [`ss:${key}`]: remember(s.sightings[`ss:${key}`], device.discovery, now),
        },
      }));
      queueMicrotask(pump);
    } catch (e) {
      set((s) => ({
        sessionSnmp: {
          ...s.sessionSnmp,
          [key]: { device: null, busy: false, error: describeError(e, t).message },
        },
      }));
    }
  };

  /** Everything already read, as the crawl planner sees it. */
  const knownDevices = (): { known: Known[]; seen: Set<string> } => {
    const known: Known[] = [];
    const seen = new Set<string>();
    for (const tab of useTabs.getState().tabs) {
      if (tab.target.type === 'network') seen.add(tab.target.host);
      known.push({ name: tab.name, depth: 0, discovery: get().byTab[tab.key]?.discovery ?? null });
      const s = get().sessionSnmp[tab.key]?.device;
      if (s) known.push({ name: tab.name, depth: 0, discovery: s.discovery });
    }
    for (const [host, e] of Object.entries(get().snmp)) {
      seen.add(host.replace(/:\d+$/, ''));
      known.push({
        name: e.device?.name ?? host,
        depth: 0,
        discovery: e.device?.discovery ?? null,
      });
    }
    for (const [ip, e] of Object.entries(get().crawled)) {
      seen.add(ip);
      known.push({
        name: e.device?.name ?? e.name,
        depth: e.depth,
        discovery: e.device?.discovery ?? null,
      });
    }
    return { known, seen };
  };

  let stoppedByUser = false;

  /** Starts as many reads as there is room for; ends the crawl when nothing is left. */
  const pump = () => {
    // Running when started by hand, or always while automatic discovery is on (unless
    // the user stopped it).
    const wanted = get().crawling || (getSetting('topology.autoCrawl') && !stoppedByUser);
    if (!wanted || !get().community) return;
    const crawled = Object.values(get().crawled);
    const inFlight = crawled.filter((e) => e.busy).length;
    const room = Math.min(CRAWL_PARALLEL - inFlight, CRAWL_MAX - crawled.length);
    const { known, seen } = knownDevices();
    const next = room > 0 ? nextCandidates(known, seen, room) : [];
    if (next.length === 0) {
      if (inFlight === 0 && get().crawling) set({ crawling: false });
      return;
    }
    set((s) => ({
      crawling: true,
      crawled: {
        ...s.crawled,
        ...Object.fromEntries(
          next.map((c) => [
            c.ip,
            { device: null, busy: true, error: null, name: c.name, from: c.from, depth: c.depth },
          ]),
        ),
      },
    }));
    for (const c of next) void readCrawled(c.ip);
  };

  const readCrawled = async (ip: string) => {
    const patchCrawl = (next: Partial<CrawlEntry>) =>
      set((s) => {
        const cur = s.crawled[ip];
        return cur ? { crawled: { ...s.crawled, [ip]: { ...cur, ...next } } } : {};
      });
    try {
      const device = await api.topologySnmp(ip);
      const now = Date.now();
      patchCrawl({ device, busy: false, error: null });
      set((s) => ({
        sightings: {
          ...s.sightings,
          [`crawl:${ip}`]: remember(s.sightings[`crawl:${ip}`], device.discovery, now),
        },
      }));
    } catch (e) {
      patchCrawl({ busy: false, error: describeError(e, t).message });
    }
    pump();
  };

  const loadCreds = async () => {
    try {
      const creds = await api.snmpCredentials();
      set({ creds, community: creds.community || creds.v3 !== null });
    } catch {
      set({ creds: { community: false, v3: null }, community: false });
    }
  };

  const startFlow = async () => {
    try {
      const flow = await api.flowStart(
        getSetting('topology.sflowPort'),
        getSetting('topology.netflowPort'),
      );
      set({ flow, flowError: null });
    } catch (e) {
      const d = describeError(e, t);
      set({ flowError: d.detail ? `${d.message} (${d.detail})` : d.message });
    }
  };

  return {
    byTab: {},
    sightings: {},
    sessionSnmp: {},
    snmp: {},
    community: null,
    creds: null,
    crawled: {},
    crawling: false,
    startCrawl() {
      stoppedByUser = false;
      set({ crawling: true });
      pump();
    },
    stopCrawl() {
      stoppedByUser = true;
      set({ crawling: false });
    },
    async setV3(user) {
      await api.snmpV3Set(user);
      await loadCreds();
      // Hosts that refused before may answer now.
      set({ snmp: {}, sessionSnmp: {}, crawled: {} });
      get().refresh(true);
    },
    async clearV3() {
      await api.snmpV3Clear();
      await loadCreds();
    },
    flow: null,
    flowError: null,
    hosts: true,
    root: null,
    setHosts(on) {
      if (on === get().hosts) return;
      set({ hosts: on });
      get().refresh(true);
    },
    setRoot: (root) => set({ root }),

    refresh(force = false) {
      const tabs = useTabs.getState().tabs;
      // Forget tabs that are gone.
      const alive = new Set(tabs.map((x) => x.key));
      const kept = Object.fromEntries(Object.entries(get().byTab).filter(([k]) => alive.has(k)));
      if (Object.keys(kept).length !== Object.keys(get().byTab).length) set({ byTab: kept });
      for (const tab of tabs) {
        if (tab.state !== 'connected' || tab.runtimeId === null) continue;
        const cur = get().byTab[tab.key];
        if (cur?.busy) continue;
        if (!force && cur && cur.runtimeId === tab.runtimeId && (cur.discovery || cur.error)) {
          continue;
        }
        void read(tab.key, tab.runtimeId, tab.device.vendor?.family ?? null);
      }
      // The same devices over SNMP too, when a community is stored.
      if (get().community && getSetting('topology.snmpSessions')) {
        for (const tab of tabs) {
          if (tab.state !== 'connected' || tab.target.type !== 'network') continue;
          const cur = get().sessionSnmp[tab.key];
          if (cur?.busy || (!force && cur && (cur.device || cur.error))) continue;
          void readSessionSnmp(tab.key, tab.target.host);
        }
      }

      const wanted = getSetting('topology.snmpHosts');
      const stillWanted = Object.fromEntries(
        Object.entries(get().snmp).filter(([h]) => wanted.includes(h)),
      );
      if (Object.keys(stillWanted).length !== Object.keys(get().snmp).length) {
        set({ snmp: stillWanted });
      }
      if (get().community) {
        for (const host of wanted) {
          const cur = get().snmp[host];
          if (cur?.busy || (!force && cur && (cur.device || cur.error))) continue;
          void readSnmp(host);
        }
      }
      if (force) {
        void get().pollFlow();
        // Read the devices found by discovery again too (their tables age out as well).
        for (const [ip, e] of Object.entries(get().crawled)) {
          if (e.device && !e.busy) {
            set((s) => ({ crawled: { ...s.crawled, [ip]: { ...e, busy: true } } }));
            void readCrawled(ip);
          }
        }
      }
      queueMicrotask(pump);
    },

    async loadSources() {
      await loadCreds();
      if (getSetting('topology.flowEnabled')) {
        const snap = await api.flowSnapshot().catch(() => null);
        if (snap?.running) set({ flow: snap, flowError: null });
        else await startFlow();
      }
      get().refresh();
    },

    async setCommunity(value) {
      await api.snmpCommunitySet(value);
      await loadCreds();
      set({ snmp: {}, sessionSnmp: {}, crawled: {} });
      get().refresh(true);
    },
    async clearCommunity() {
      await api.snmpCommunityClear();
      await loadCreds();
    },

    addSnmpHost(host) {
      const h = host.trim();
      const list = getSetting('topology.snmpHosts');
      if (h === '' || /\s/.test(h) || list.includes(h)) return;
      useSettings.getState().set('topology.snmpHosts', [...list, h]);
      get().refresh();
    },
    removeSnmpHost(host) {
      useSettings.getState().set(
        'topology.snmpHosts',
        getSetting('topology.snmpHosts').filter((h) => h !== host),
      );
      get().refresh();
    },

    async setFlow(enabled) {
      useSettings.getState().set('topology.flowEnabled', enabled);
      if (enabled) {
        await startFlow();
      } else {
        const flow = await api.flowStop().catch(() => null);
        set({ flow, flowError: null });
      }
    },
    async pollFlow() {
      if (!get().flow?.running) return;
      const flow = await api.flowSnapshot().catch(() => null);
      if (flow) set({ flow });
    },
  };
});
