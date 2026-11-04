/** Session library helpers: environment tags and the folder tree (PROD sorted first). */
import type { Folder, SessionInput, SessionRecord } from './ipc';

export type EnvKind = 'prod' | 'lab' | 'test' | 'custom';

export function normalizeEnv(tag: string | null | undefined): string | null {
  const t = (tag ?? '').trim();
  return t ? t : null;
}

export function envKind(tag: string | null | undefined): EnvKind | null {
  const t = normalizeEnv(tag)?.toLowerCase();
  if (!t) return null;
  if (t === 'prod' || t === 'lab' || t === 'test') return t;
  return 'custom';
}

/** Short visible label: PROD / LAB / TEST / custom text upper-cased. */
export function envLabel(tag: string | null | undefined): string | null {
  const t = normalizeEnv(tag);
  return t ? t.toUpperCase() : null;
}

/** One-letter marker used on tabs and sidebar rows (P/L/T/…). */
export function envLetter(tag: string | null | undefined): string | null {
  const t = normalizeEnv(tag);
  return t ? (Array.from(t)[0] ?? '').toLocaleUpperCase() : null;
}

export const isProd = (tag: string | null | undefined): boolean => envKind(tag) === 'prod';

export interface TreeFolderNode {
  kind: 'folder';
  folder: Folder;
  depth: number;
  hasProd: boolean;
  children: TreeNode[];
}

export interface TreeSessionNode {
  kind: 'session';
  session: SessionRecord;
  depth: number;
}

export type TreeNode = TreeFolderNode | TreeSessionNode;

const byName = (a: string, b: string) => a.localeCompare(b, undefined, { sensitivity: 'base' });

/**
 * Builds the nested tree. Folders before sessions; inside each group PROD first, then
 * `sortOrder`, then name. A folder counts as PROD when it is named "PROD" or contains a
 * PROD session anywhere below it. Orphans (missing parent) land at the root; cycles are cut.
 */
export function buildTree(
  folders: readonly Folder[],
  sessions: readonly SessionRecord[],
): TreeNode[] {
  const ids = new Set(folders.map((f) => f.id));
  const childFolders = new Map<number | null, Folder[]>();
  for (const f of folders) {
    const parent =
      f.parentId !== null && ids.has(f.parentId) && f.parentId !== f.id ? f.parentId : null;
    const list = childFolders.get(parent) ?? [];
    list.push(f);
    childFolders.set(parent, list);
  }
  const childSessions = new Map<number | null, SessionRecord[]>();
  for (const s of sessions) {
    const parent = s.folderId !== null && ids.has(s.folderId) ? s.folderId : null;
    const list = childSessions.get(parent) ?? [];
    list.push(s);
    childSessions.set(parent, list);
  }

  const visited = new Set<number>();
  const build = (parent: number | null, depth: number): TreeNode[] => {
    const folderNodes: TreeFolderNode[] = [];
    for (const f of childFolders.get(parent) ?? []) {
      if (visited.has(f.id)) continue;
      visited.add(f.id);
      const children = build(f.id, depth + 1);
      const hasProd =
        f.name.trim().toUpperCase() === 'PROD' ||
        children.some((c) => (c.kind === 'folder' ? c.hasProd : isProd(c.session.envTag)));
      folderNodes.push({ kind: 'folder', folder: f, depth, hasProd, children });
    }
    folderNodes.sort(
      (a, b) =>
        Number(b.hasProd) - Number(a.hasProd) ||
        a.folder.sortOrder - b.folder.sortOrder ||
        byName(a.folder.name, b.folder.name),
    );
    const sessionNodes: TreeSessionNode[] = (childSessions.get(parent) ?? [])
      .slice()
      .sort(
        (a, b) =>
          Number(isProd(b.envTag)) - Number(isProd(a.envTag)) ||
          a.sortOrder - b.sortOrder ||
          byName(a.name, b.name),
      )
      .map((session) => ({ kind: 'session', session, depth }));
    return [...folderNodes, ...sessionNodes];
  };
  const root = build(null, 0);
  // Folders unreachable from the root (pure cycles) are appended at the root.
  for (const f of folders) {
    if (!visited.has(f.id)) {
      visited.add(f.id);
      root.push({ kind: 'folder', folder: f, depth: 0, hasProd: false, children: build(f.id, 1) });
    }
  }
  return root;
}

/** `id` and every folder below it (used to forbid dropping a folder into itself). */
export function descendantFolderIds(folders: readonly Folder[], id: number): Set<number> {
  const out = new Set<number>([id]);
  let grew = true;
  while (grew) {
    grew = false;
    for (const f of folders) {
      if (f.parentId !== null && out.has(f.parentId) && !out.has(f.id)) {
        out.add(f.id);
        grew = true;
      }
    }
  }
  return out;
}

/** Folder path label for selects: "LAB / Rack B". */
export function folderPath(folders: readonly Folder[], id: number | null): string {
  const parts: string[] = [];
  const seen = new Set<number>();
  let cur = folders.find((f) => f.id === id);
  while (cur && !seen.has(cur.id)) {
    seen.add(cur.id);
    parts.unshift(cur.name);
    const parentId = cur.parentId;
    cur = parentId === null ? undefined : folders.find((f) => f.id === parentId);
  }
  return parts.join(' / ');
}

export function toInput(s: SessionRecord): SessionInput {
  return {
    folderId: s.folderId,
    name: s.name,
    protocol: s.protocol,
    host: s.host,
    port: s.port,
    username: s.username,
    serial: s.serial ? { ...s.serial } : null,
    envTag: s.envTag,
    tags: [...s.tags],
    favorite: s.favorite,
  };
}

/** Most recently connected sessions first. */
export function recentSessions(sessions: readonly SessionRecord[], limit = 5): SessionRecord[] {
  return sessions
    .filter((s) => s.lastConnectedAt !== null)
    .sort((a, b) => (b.lastConnectedAt ?? 0) - (a.lastConnectedAt ?? 0))
    .slice(0, limit);
}
