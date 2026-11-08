/**
 * Note lookups outside the notes panel (P3-T09): interface notes per note owner (for the
 * terminal tooltip) and pinned notes' first lines (subtitles in the session list).
 * The notes panel calls `changed` after every edit.
 */
import { create } from 'zustand';
import { api, type PinnedNote } from '../lib/ipc';

export const ownerKey = (sessionId: number | null, deviceId: number | null) =>
  `${sessionId ?? '-'}:${deviceId ?? '-'}`;

interface NotesState {
  /** owner → interface key → note text (pinned first, then newest). */
  byOwner: Record<string, Record<string, string>>;
  pinned: PinnedNote[];
  /** Bumped on every change, so the side panel and the notes bar reload each other's edits. */
  rev: number;
  loadOwner(sessionId: number | null, deviceId: number | null): Promise<void>;
  loadPinned(): Promise<void>;
  changed(sessionId: number | null, deviceId: number | null): void;
}

export const useNotes = create<NotesState>()((set) => ({
  byOwner: {},
  pinned: [],
  rev: 0,

  async loadOwner(sessionId, deviceId) {
    if (sessionId === null && deviceId === null) return;
    const notes = await api.notesList(sessionId, deviceId).catch(() => []);
    const map: Record<string, string> = {};
    for (const n of notes) {
      if (n.interfaceKey && !(n.interfaceKey in map)) map[n.interfaceKey] = n.body;
    }
    set((s) => ({ byOwner: { ...s.byOwner, [ownerKey(sessionId, deviceId)]: map } }));
  },

  async loadPinned() {
    set({ pinned: await api.notesPinned().catch(() => []) });
  },

  changed(sessionId, deviceId) {
    set((s) => ({ rev: s.rev + 1 }));
    void useNotes.getState().loadOwner(sessionId, deviceId);
    void useNotes.getState().loadPinned();
  },
}));

/** The pinned note subtitle for a saved session (its own or its device's, newest). */
export function pinnedFor(
  pinned: readonly PinnedNote[],
  sessionId: number,
  deviceId: number | null,
): string | null {
  const hit = pinned.find(
    (p) => p.sessionId === sessionId || (deviceId !== null && p.deviceId === deviceId),
  );
  return hit?.text ?? null;
}
