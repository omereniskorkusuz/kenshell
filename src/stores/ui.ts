/** Transient UI state (not persisted): overlays, side panel, status message, announcements. */
import { create } from 'zustand';
import type { SessionInput } from '../lib/ipc';

export type PaletteScope = 'all' | 'consoles';
export type SidePanelTab = 'notes' | 'analyzer' | 'snippets' | 'tools';

export interface EditorState {
  /** Saved session being edited, or null for a new one. */
  sessionId: number | null;
  /** Prefill for a new session (e.g. from a detected console). */
  initial?: Partial<SessionInput>;
}

interface UiState {
  paletteOpen: boolean;
  paletteQuery: string;
  /** 'consoles' = only the detected consoles group (from "Connect detected console"). */
  paletteScope: PaletteScope;
  settingsOpen: boolean;
  editor: EditorState | null;
  sidePanelOpen: boolean;
  sidePanelTab: SidePanelTab;
  statusMessage: { id: number; text: string } | null;
  announcement: { id: number; text: string } | null;
  quickConnectFocus: number;
  /** Tab whose scrollback search bar is open. */
  findKey: string | null;
  openFind(key: string): void;
  closeFind(): void;

  openPalette(query?: string, scope?: PaletteScope): void;
  closePalette(): void;
  setSettingsOpen(open: boolean): void;
  openEditor(state: EditorState): void;
  closeEditor(): void;
  toggleSidePanel(open?: boolean): void;
  setSidePanelTab(tab: SidePanelTab): void;
  /** Transient status bar message (auto-clears). */
  flash(text: string): void;
  /** Screen reader announcement (aria-live polite). */
  announce(text: string): void;
  focusQuickConnect(): void;
}

let seq = 1;
let flashTimer: ReturnType<typeof setTimeout> | null = null;
export const FLASH_MS = 4000;

export const useUi = create<UiState>()((set, get) => ({
  paletteOpen: false,
  paletteQuery: '',
  paletteScope: 'all',
  settingsOpen: false,
  editor: null,
  sidePanelOpen: false,
  sidePanelTab: 'notes',
  statusMessage: null,
  announcement: null,
  quickConnectFocus: 0,
  findKey: null,
  openFind: (key) => set({ findKey: key }),
  closeFind: () => set({ findKey: null }),

  openPalette: (query = '', scope = 'all') =>
    set({ paletteOpen: true, paletteQuery: query, paletteScope: scope }),
  closePalette: () => set({ paletteOpen: false }),
  setSettingsOpen: (open) => set({ settingsOpen: open }),
  openEditor: (editor) => set({ editor }),
  closeEditor: () => set({ editor: null }),
  toggleSidePanel: (open) => set({ sidePanelOpen: open ?? !get().sidePanelOpen }),
  setSidePanelTab: (tab) => set({ sidePanelTab: tab, sidePanelOpen: true }),
  flash(text) {
    if (flashTimer) clearTimeout(flashTimer);
    const id = seq++;
    set({ statusMessage: { id, text } });
    flashTimer = setTimeout(() => {
      if (get().statusMessage?.id === id) set({ statusMessage: null });
    }, FLASH_MS);
  },
  announce: (text) => set({ announcement: { id: seq++, text } }),
  focusQuickConnect: () => set({ quickConnectFocus: seq++ }),
}));
