/**
 * Bulk edit of saved sessions (P2-T10). The backend applies the same rules
 * (`SessionPatch::apply` in model.rs); this copy serves the browser mock and the tests.
 */
import type { SessionInput } from './ipc';

/** A field set to `value` (null clears it); an absent field stays as it is. */
export interface Change<T> {
  value: T;
}

export interface SessionPatch {
  folder?: Change<number | null>;
  envTag?: Change<string | null>;
  addTags?: string[];
  removeTags?: string[];
  favorite?: boolean;
  /** Network sessions only. */
  username?: Change<string | null>;
  credentialProfile?: Change<number | null>;
}

const same = (a: string, b: string) => a.trim().toLowerCase() === b.trim().toLowerCase();

export function applyPatch(s: SessionInput, p: SessionPatch): SessionInput {
  const out: SessionInput = { ...s, tags: [...s.tags] };
  if (p.folder) out.folderId = p.folder.value;
  if (p.envTag) out.envTag = p.envTag.value;
  out.tags = out.tags.filter((t) => !(p.removeTags ?? []).some((r) => same(t, r)));
  for (const tag of p.addTags ?? []) {
    if (tag.trim() && !out.tags.some((t) => same(t, tag))) out.tags.push(tag.trim());
  }
  if (p.favorite !== undefined) out.favorite = p.favorite;
  if (out.protocol !== 'serial') {
    if (p.username) out.username = p.username.value;
    if (p.credentialProfile) out.credentialProfileId = p.credentialProfile.value;
  }
  return out;
}

/** Splits "a, b  c" into tags (comma or whitespace separated, no empties, no repeats). */
export function parseTagList(text: string): string[] {
  const out: string[] = [];
  for (const raw of text.split(/[,\s]+/)) {
    const t = raw.trim();
    if (t && !out.some((x) => same(x, t))) out.push(t);
  }
  return out;
}

/** Range selection: ids between `anchor` and `to` (inclusive) in visible order. */
export function rangeBetween(order: number[], anchor: number, to: number): number[] {
  const a = order.indexOf(anchor);
  const b = order.indexOf(to);
  if (a < 0 || b < 0) return [to];
  return order.slice(Math.min(a, b), Math.max(a, b) + 1);
}
