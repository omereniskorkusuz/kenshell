/**
 * Update prompt state. A found update opens a small card on the main window (top right);
 * "Later" hides it for this run (it asks again on the next start), "Update now" downloads,
 * installs and restarts. Nothing installs without that click.
 */
import { create } from 'zustand';
import type { UpdateInfo } from '../lib/ipc';

export type UpdatePhase = 'available' | 'downloading' | 'error';

interface UpdateState {
  info: UpdateInfo | null;
  phase: UpdatePhase;
  /** 0–100 while downloading; null = size unknown. */
  percent: number | null;
  error: string | null;
  /** Card hidden with "Later" (the update is still known, e.g. for Settings). */
  dismissed: boolean;
  found(info: UpdateInfo): void;
  later(): void;
  progress(done: number, total: number | null): void;
  failed(message: string): void;
}

export const useUpdate = create<UpdateState>()((set) => ({
  info: null,
  phase: 'available',
  percent: null,
  error: null,
  dismissed: false,
  found: (info) => set({ info, phase: 'available', percent: null, error: null, dismissed: false }),
  later: () => set({ dismissed: true }),
  progress: (done, total) =>
    set({
      phase: 'downloading',
      percent: total ? Math.min(100, Math.floor((done / total) * 100)) : null,
    }),
  failed: (message) => set({ phase: 'error', error: message }),
}));

// Development aid: `kenshellUpdate.getState().found({...})` in the dev console shows the card.
if (import.meta.env.DEV) (globalThis as Record<string, unknown>).kenshellUpdate = useUpdate;
