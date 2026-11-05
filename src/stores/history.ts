/**
 * Command history per note owner (P3-T12): the device once known, otherwise the saved
 * session (backend `command_history`). Quick-connect tabs to unknown devices keep an
 * in-memory history for the tab's lifetime only.
 */
import { create } from 'zustand';
import { api } from '../lib/ipc';
import { storable } from '../terminal/autocomplete';
import { ownerKey } from './notes';

const MAX = 500;

interface HistoryState {
  byOwner: Record<string, string[]>;
  load(sessionId: number | null, deviceId: number | null): Promise<void>;
  add(key: string, sessionId: number | null, deviceId: number | null, command: string): void;
}

export const useHistory = create<HistoryState>()((set) => ({
  byOwner: {},

  async load(sessionId, deviceId) {
    if (sessionId === null && deviceId === null) return;
    const list = await api.historyList(sessionId, deviceId).catch(() => []);
    set((s) => ({ byOwner: { ...s.byOwner, [ownerKey(sessionId, deviceId)]: list } }));
  },

  /** `key` = ownerKey, or a tab key for tabs without an owner. */
  add(key, sessionId, deviceId, command) {
    const c = command.trim();
    if (!storable(c)) return;
    set((s) => {
      const list = [c, ...(s.byOwner[key] ?? []).filter((x) => x !== c)].slice(0, MAX);
      return { byOwner: { ...s.byOwner, [key]: list } };
    });
    if (sessionId !== null || deviceId !== null) {
      void api.historyAdd(sessionId, deviceId, c).catch(() => {});
    }
  },
}));

/** Where a tab's history lives. */
export const historyKey = (tabKey: string, sessionId: number | null, deviceId: number | null) =>
  sessionId === null && deviceId === null ? `tab:${tabKey}` : ownerKey(sessionId, deviceId);
