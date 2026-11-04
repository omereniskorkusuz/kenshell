/**
 * Saved folders + sessions. Every mutation goes through the backend and then reloads the
 * affected list, so the UI always shows what is stored. Deletions are immediate and offer
 * Undo in a silent toast (no confirm dialogs — product rule).
 */
import { create } from 'zustand';
import { t } from '../i18n';
import { describeError } from '../lib/errors';
import { api, type Folder, type SessionInput, type SessionRecord } from '../lib/ipc';
import { toInput } from '../lib/tree';
import { toast } from './toasts';

export type RenameTarget = { kind: 'folder' | 'session'; id: number } | null;

interface LibraryState {
  folders: Folder[];
  sessions: SessionRecord[];
  loaded: boolean;
  renaming: RenameTarget;
  collapsed: Set<number>;

  load(): Promise<void>;
  reloadSessions(): Promise<void>;
  setRenaming(target: RenameTarget): void;
  toggleCollapsed(folderId: number, collapsed?: boolean): void;

  createFolder(parentId: number | null): Promise<Folder | null>;
  renameFolder(id: number, name: string): Promise<void>;
  moveFolder(id: number, parentId: number | null): Promise<void>;
  deleteFolder(id: number): Promise<void>;

  createSession(input: SessionInput): Promise<SessionRecord | null>;
  updateSession(id: number, input: SessionInput): Promise<SessionRecord | null>;
  renameSession(id: number, name: string): Promise<void>;
  moveSession(id: number, folderId: number | null): Promise<void>;
  duplicateSession(id: number): Promise<SessionRecord | null>;
  deleteSession(id: number): Promise<void>;
}

function reportError(e: unknown): void {
  const d = describeError(e, t);
  toast({ kind: 'error', message: d.message, detail: d.detail });
}

export const useLibrary = create<LibraryState>()((set, get) => {
  const reload = async () => {
    const [folders, sessions] = await Promise.all([api.folderList(), api.sessionList()]);
    set({ folders, sessions, loaded: true });
  };
  /** Runs a mutation, reloads, reports errors as toasts. */
  const mutate = async <T>(fn: () => Promise<T>): Promise<T | null> => {
    try {
      const out = await fn();
      await reload();
      return out;
    } catch (e) {
      reportError(e);
      await reload().catch(() => {});
      return null;
    }
  };

  return {
    folders: [],
    sessions: [],
    loaded: false,
    renaming: null,
    collapsed: new Set<number>(),

    async load() {
      try {
        await reload();
      } catch (e) {
        set({ loaded: true });
        reportError(e);
      }
    },
    async reloadSessions() {
      try {
        set({ sessions: await api.sessionList() });
      } catch {
        // keep the current list; the next mutation reports errors
      }
    },
    setRenaming: (renaming) => set({ renaming }),
    toggleCollapsed(folderId, collapsed) {
      const next = new Set(get().collapsed);
      const shouldCollapse = collapsed ?? !next.has(folderId);
      if (shouldCollapse) next.add(folderId);
      else next.delete(folderId);
      set({ collapsed: next });
    },

    async createFolder(parentId) {
      const folder = await mutate(() => api.folderCreate(t('library.newFolderName'), parentId));
      if (folder) {
        if (parentId !== null) get().toggleCollapsed(parentId, false);
        set({ renaming: { kind: 'folder', id: folder.id } });
      }
      return folder;
    },
    async renameFolder(id, name) {
      const clean = name.trim();
      const cur = get().folders.find((f) => f.id === id);
      if (!clean || !cur || cur.name === clean) return;
      await mutate(() => api.folderRename(id, clean));
    },
    async moveFolder(id, parentId) {
      const cur = get().folders.find((f) => f.id === id);
      if (!cur || cur.parentId === parentId || id === parentId) return;
      await mutate(() => api.folderMove(id, parentId));
    },
    async deleteFolder(id) {
      const { folders, sessions } = get();
      const folder = folders.find((f) => f.id === id);
      if (!folder) return;
      // Backend moves children to the parent; remember them so Undo can put them back.
      const childFolders = folders.filter((f) => f.parentId === id).map((f) => f.id);
      const childSessions = sessions.filter((s) => s.folderId === id).map((s) => s.id);
      const ok = await mutate(async () => {
        await api.folderDelete(id);
        return true;
      });
      if (!ok) return;
      toast({
        message: t('library.folderDeleted', { name: folder.name }),
        actions: [
          {
            label: t('common.undo'),
            primary: true,
            run: () =>
              void mutate(async () => {
                const again = await api.folderCreate(folder.name, folder.parentId);
                for (const fid of childFolders) await api.folderMove(fid, again.id);
                const current = await api.sessionList();
                for (const sid of childSessions) {
                  const s = current.find((x) => x.id === sid);
                  if (s) await api.sessionUpdate(sid, { ...toInput(s), folderId: again.id });
                }
                return again;
              }),
          },
        ],
      });
    },

    createSession: (input) => mutate(() => api.sessionCreate(input)),
    updateSession: (id, input) => mutate(() => api.sessionUpdate(id, input)),
    async renameSession(id, name) {
      const clean = name.trim();
      const s = get().sessions.find((x) => x.id === id);
      if (!clean || !s || s.name === clean) return;
      await mutate(() => api.sessionUpdate(id, { ...toInput(s), name: clean }));
    },
    async moveSession(id, folderId) {
      const s = get().sessions.find((x) => x.id === id);
      if (!s || s.folderId === folderId) return;
      await mutate(() => api.sessionUpdate(id, { ...toInput(s), folderId }));
    },
    duplicateSession: (id) => mutate(() => api.sessionDuplicate(id)),
    async deleteSession(id) {
      const s = get().sessions.find((x) => x.id === id);
      if (!s) return;
      const cached = toInput(s);
      const ok = await mutate(async () => {
        await api.sessionDelete(id);
        return true;
      });
      if (!ok) return;
      toast({
        message: t('library.sessionDeleted', { name: s.name }),
        actions: [
          {
            label: t('common.undo'),
            primary: true,
            // Recreate from the cached input (gets a new id; history fields start fresh).
            run: () => void mutate(() => api.sessionCreate(cached)),
          },
        ],
      });
    },
  };
});
