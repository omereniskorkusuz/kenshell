/**
 * Snippet library helpers (P3-T15): built-in starter sets + the user's snippets, vendor
 * filtering, folder grouping and `{{variable}}` discovery (filled in by P3-T16's form).
 */
import starter from '../data/snippets.json';
import type { Snippet } from './ipc';

/** A snippet as listed: the user's (`id` = number) or a read-only starter one. */
export interface SnippetItem {
  key: string;
  id: number | null;
  folder: string;
  name: string;
  body: string;
  vendor: string | null;
  builtin: boolean;
}

export const STARTER: SnippetItem[] = (
  starter.snippets as { id: string; vendor: string; folder: string; name: string; body: string }[]
).map((s) => ({
  key: `b:${s.id}`,
  id: null,
  folder: s.folder,
  name: s.name,
  body: s.body,
  vendor: s.vendor,
  builtin: true,
}));

export const toItem = (s: Snippet): SnippetItem => ({
  key: `u:${s.id}`,
  id: s.id,
  folder: s.folder,
  name: s.name,
  body: s.body,
  vendor: s.vendor,
  builtin: false,
});

/**
 * `vendor`: 'all', or a family (shows that family's and vendor-neutral snippets).
 * `query` matches name, folder and body (case-insensitive).
 */
export function filterSnippets(
  items: readonly SnippetItem[],
  vendor: string,
  query: string,
): SnippetItem[] {
  const q = query.trim().toLocaleLowerCase();
  return items.filter(
    (s) =>
      (vendor === 'all' || s.vendor === null || s.vendor === vendor) &&
      (!q || `${s.name}\n${s.folder}\n${s.body}`.toLocaleLowerCase().includes(q)),
  );
}

/** Folders in order (user snippets first, top level = ''), each sorted by name. */
export function groupSnippets(items: readonly SnippetItem[]): [string, SnippetItem[]][] {
  const groups = new Map<string, SnippetItem[]>();
  const order = [...items].sort(
    (a, b) =>
      Number(a.builtin) - Number(b.builtin) ||
      a.folder.localeCompare(b.folder, undefined, { sensitivity: 'base' }) ||
      a.name.localeCompare(b.name, undefined, { sensitivity: 'base' }),
  );
  for (const s of order) {
    const g = groups.get(s.folder) ?? [];
    g.push(s);
    groups.set(s.folder, g);
  }
  return [...groups];
}

const VAR = /\{\{\s*([A-Za-z_][\w-]*)\s*\}\}/g;

/** Variable names in order of first use. */
export function snippetVariables(body: string): string[] {
  const seen: string[] = [];
  for (const m of body.matchAll(VAR)) {
    const name = m[1] as string;
    if (!seen.includes(name)) seen.push(name);
  }
  return seen;
}

/** Text to send: one command per line, CR line ends, a final Enter. */
export function snippetPayload(body: string): string {
  const lines = body.replace(/\r\n?/g, '\n').replace(/\n+$/, '').split('\n');
  return `${lines.join('\r')}\r`;
}
