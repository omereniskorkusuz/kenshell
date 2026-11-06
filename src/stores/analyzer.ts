/**
 * Analyzer state (P4-T01): the known parsers (loaded once) and the last finished command
 * per tab that one of them can read (the panel analyses it automatically).
 */
import { create } from 'zustand';
import { commandMatches } from '../lib/analyzer';
import { api, type ParserInfo } from '../lib/ipc';

interface AnalyzerState {
  parsers: ParserInfo[] | null;
  /** Bumped when a command with a parser finished in a tab. */
  done: { tabKey: string; command: string; seq: number } | null;
  loadParsers(): Promise<ParserInfo[]>;
  commandDone(tabKey: string, command: string): Promise<void>;
}

let seq = 0;

export const useAnalyzer = create<AnalyzerState>()((set, get) => ({
  parsers: null,
  done: null,

  async loadParsers() {
    const have = get().parsers;
    if (have) return have;
    const parsers = await api.analyzerParsers().catch(() => [] as ParserInfo[]);
    set({ parsers });
    return parsers;
  },

  async commandDone(tabKey, command) {
    const parsers = await get().loadParsers();
    if (!parsers.some((p) => [p.command, ...p.aliases].some((c) => commandMatches(command, c))))
      return;
    set({ done: { tabKey, command, seq: ++seq } });
  },
}));
