/**
 * Topology map data, per source: what each open session reported (read hidden with
 * read-only commands), what each SNMP host answered, and what the flow collector has
 * seen. The graph itself is derived in `lib/topologyGraph.ts`.
 */
import { create } from 'zustand';
import { t } from '../i18n';
import { describeError } from '../lib/errors';
import { api, type Discovery, type FlowSnapshot, type SnmpDevice } from '../lib/ipc';
import { getSetting, useSettings } from './settings';
import { useTabs } from './tabs';

export interface TopoEntry {
  discovery: Discovery | null;
  busy: boolean;
  error: string | null;
  /** Session generation the data belongs to (a reconnect reads again). */
  runtimeId: number | null;
}

export interface SnmpEntry {
  device: SnmpDevice | null;
  busy: boolean;
  error: string | null;
}

interface TopologyState {
  byTab: Record<string, TopoEntry>;
  /** SNMP hosts (as typed) → what they answered. */
  snmp: Record<string, SnmpEntry>;
  /** A community is stored in the keychain (null = not asked yet). */
  community: boolean | null;
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
      patch(key, { discovery, busy: false });
    } catch (e) {
      patch(key, { busy: false, error: describeError(e, t).message });
    }
  };

  const readSnmp = async (host: string) => {
    patchSnmp(host, { busy: true, error: null });
    try {
      const device = await api.topologySnmp(host);
      // Removed while it was being read?
      if (getSetting('topology.snmpHosts').includes(host)) patchSnmp(host, { device, busy: false });
    } catch (e) {
      patchSnmp(host, { busy: false, error: describeError(e, t).message });
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
    snmp: {},
    community: null,
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
      if (force) void get().pollFlow();
    },

    async loadSources() {
      try {
        set({ community: await api.snmpCommunityStatus() });
      } catch {
        set({ community: false });
      }
      if (getSetting('topology.flowEnabled')) {
        const snap = await api.flowSnapshot().catch(() => null);
        if (snap?.running) set({ flow: snap, flowError: null });
        else await startFlow();
      }
      get().refresh();
    },

    async setCommunity(value) {
      await api.snmpCommunitySet(value);
      set({ community: true });
      get().refresh(true);
    },
    async clearCommunity() {
      await api.snmpCommunityClear();
      set({ community: false, snmp: {} });
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
