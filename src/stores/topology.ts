/**
 * Topology map data: what each open session reported (read hidden with read-only
 * commands), keyed by tab. The graph itself is derived in `lib/topologyGraph.ts`.
 */
import { create } from 'zustand';
import { t } from '../i18n';
import { describeError } from '../lib/errors';
import { api, type Discovery } from '../lib/ipc';
import { useTabs } from './tabs';

export interface TopoEntry {
  discovery: Discovery | null;
  busy: boolean;
  error: string | null;
  /** Session generation the data belongs to (a reconnect reads again). */
  runtimeId: number | null;
}

interface TopologyState {
  byTab: Record<string, TopoEntry>;
  /** Also read MAC / ARP tables and show end devices. */
  hosts: boolean;
  /** Node chosen as the top of the hierarchy (null = automatic). */
  root: string | null;
  setHosts(on: boolean): void;
  setRoot(id: string | null): void;
  /** Reads every connected tab (`force`), or only the ones not read yet. */
  refresh(force?: boolean): void;
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

  return {
    byTab: {},
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
    },
  };
});
