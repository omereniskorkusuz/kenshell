/** The user's snippets (P3-T15), loaded on first use; the panel and the palette share it. */
import { create } from 'zustand';
import { api, type Snippet, type SnippetInput } from '../lib/ipc';

interface SnippetsState {
  list: Snippet[];
  loaded: boolean;
  load(): Promise<void>;
  save(id: number | null, input: SnippetInput): Promise<Snippet>;
  remove(id: number): Promise<void>;
}

export const useSnippets = create<SnippetsState>()((set, get) => ({
  list: [],
  loaded: false,

  async load() {
    const list = await api.snippetsList().catch(() => [] as Snippet[]);
    set({ list, loaded: true });
  },

  async save(id, input) {
    const s = id === null ? await api.snippetCreate(input) : await api.snippetUpdate(id, input);
    set({ list: [...get().list.filter((x) => x.id !== s.id), s] });
    return s;
  },

  async remove(id) {
    await api.snippetDelete(id);
    set({ list: get().list.filter((x) => x.id !== id) });
  },
}));
