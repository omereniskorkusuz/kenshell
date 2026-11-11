/** Transient UI state (not persisted): overlays, side panel, status message, announcements. */
import { create } from 'zustand';
import type { SessionInput } from '../lib/ipc';
import type { DropZone } from '../lib/split';

export type PaletteScope = 'all' | 'consoles';
export type SidePanelTab = 'notes' | 'analyzer' | 'snippets' | 'tools';
export type ToolId = 'transfer' | 'servers' | 'nettools' | 'ai';

/** A tab being dragged: where it would land (P2-T19). */
export interface TabDrag {
  key: string;
  /** Pane under the pointer and the zone of that pane. */
  pane: string | null;
  zone: DropZone | null;
  /** Pointer outside the window: releasing detaches the tab. */
  outside: boolean;
}

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
  /** Network scanner dialog (P4-T14). */
  scanOpen: boolean;
  setScanOpen(open: boolean): void;
  editor: EditorState | null;
  sidePanelOpen: boolean;
  sidePanelTab: SidePanelTab;
  /** The shortcut editor is waiting for a key (global shortcuts pause). */
  capturingShortcut: boolean;
  setCapturingShortcut(on: boolean): void;
  /** Recording open in the replay player (P5-T06). */
  replay: string | null;
  openReplay(path: string): void;
  closeReplay(): void;
  /** Tool shown in the side panel's Tools tab (Phase 5). */
  tool: ToolId;
  openTool(tool: ToolId): void;
  /** Tabs whose notes bar (under the terminal) is open. */
  notesBar: string[];
  toggleNotesBar(key: string): void;
  /** Text to start a new note with (terminal "Add note"); the notes panel takes it. */
  noteDraft: string | null;
  /** Snippet (item key) whose fill-in form the snippets panel opens (palette, P3-T17). */
  snippetFill: string | null;
  openSnippetFill(key: string): void;
  takeSnippetFill(): string | null;
  startNote(text: string): void;
  takeNoteDraft(): string | null;
  statusMessage: { id: number; text: string } | null;
  announcement: { id: number; text: string } | null;
  quickConnectFocus: number;
  /** Bumped to focus the sidebar session search. */
  searchFocus: number;
  focusSearch(): void;
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
  tabDrag: TabDrag | null;
  setTabDrag(drag: TabDrag | null): void;
  /** The topology map has a tab of its own; `Active` = it is the one shown. */
  topologyOpen: boolean;
  topologyActive: boolean;
  openTopology(): void;
  closeTopology(): void;
  /** A session tab was chosen: the map stays open but is no longer shown. */
  leaveTopology(): void;
}

let seq = 1;
let flashTimer: ReturnType<typeof setTimeout> | null = null;
export const FLASH_MS = 4000;

export const useUi = create<UiState>()((set, get) => ({
  paletteOpen: false,
  paletteQuery: '',
  paletteScope: 'all',
  settingsOpen: false,
  scanOpen: false,
  editor: null,
  sidePanelOpen: false,
  sidePanelTab: 'notes',
  tool: 'transfer',
  replay: null,
  capturingShortcut: false,
  setCapturingShortcut: (on) => set({ capturingShortcut: on }),
  openReplay: (path) => set({ replay: path }),
  closeReplay: () => set({ replay: null }),
  openTool: (tool) => set({ tool, sidePanelTab: 'tools', sidePanelOpen: true }),
  notesBar: [],
  toggleNotesBar: (key) =>
    set((s) => ({
      notesBar: s.notesBar.includes(key)
        ? s.notesBar.filter((k) => k !== key)
        : [...s.notesBar, key],
    })),
  noteDraft: null,
  snippetFill: null,
  statusMessage: null,
  announcement: null,
  quickConnectFocus: 0,
  searchFocus: 0,
  focusSearch: () => set({ searchFocus: seq++ }),
  findKey: null,
  openFind: (key) => set({ findKey: key }),
  closeFind: () => set({ findKey: null }),

  topologyOpen: false,
  topologyActive: false,
  openTopology: () => set({ topologyOpen: true, topologyActive: true }),
  closeTopology: () => set({ topologyOpen: false, topologyActive: false }),
  leaveTopology: () => {
    if (get().topologyActive) set({ topologyActive: false });
  },

  openPalette: (query = '', scope = 'all') =>
    set({ paletteOpen: true, paletteQuery: query, paletteScope: scope }),
  closePalette: () => set({ paletteOpen: false }),
  setSettingsOpen: (open) => set({ settingsOpen: open }),
  setScanOpen: (open) => set({ scanOpen: open }),
  openEditor: (editor) => set({ editor }),
  closeEditor: () => set({ editor: null }),
  toggleSidePanel: (open) => set({ sidePanelOpen: open ?? !get().sidePanelOpen }),
  setSidePanelTab: (tab) => set({ sidePanelTab: tab, sidePanelOpen: true }),
  openSnippetFill: (key) =>
    set({ snippetFill: key, sidePanelTab: 'snippets', sidePanelOpen: true }),
  takeSnippetFill() {
    const k = get().snippetFill;
    if (k !== null) set({ snippetFill: null });
    return k;
  },
  startNote: (text) => set({ noteDraft: text, sidePanelTab: 'notes', sidePanelOpen: true }),
  takeNoteDraft() {
    const d = get().noteDraft;
    if (d !== null) set({ noteDraft: null });
    return d;
  },
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
  tabDrag: null,
  setTabDrag: (tabDrag) => set({ tabDrag }),
}));
