/**
 * Tab snapshot handed to a new window when a tab is detached (P2-T19). The backend keeps it
 * as an opaque string in memory only. A login prompt in progress is never carried over (the
 * store refuses to detach while one is pending), so no typed secret is in here.
 */
import type { LiveTab } from './sessionState';

export type TabFields = Omit<LiveTab, 'key' | 'runtimeId' | 'generation' | 'prompt'>;

export interface TabSnapshot {
  tab: TabFields;
  /** Rendered scrollback + screen as plain text (CRLF line ends). */
  screen: string;
  /** Output the old window had not rendered yet (raw device bytes). */
  pending: Uint8Array;
}

const VERSION = 1;
const WINDOW_LOCAL = ['key', 'runtimeId', 'generation', 'prompt'] as const;

function toBase64(bytes: Uint8Array): string {
  let bin = '';
  for (let i = 0; i < bytes.length; i += 0x8000) {
    bin += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
  }
  return btoa(bin);
}

function fromBase64(text: string): Uint8Array {
  const bin = atob(text);
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out;
}

export function encodeHandoff(tab: LiveTab, screen: string, pending: Uint8Array): string {
  const fields = Object.fromEntries(
    Object.entries(tab).filter(([k]) => !(WINDOW_LOCAL as readonly string[]).includes(k)),
  );
  return JSON.stringify({ v: VERSION, tab: fields, screen, pending: toBase64(pending) });
}

/** Parses a payload; null when it is not a snapshot of this version. */
export function decodeHandoff(payload: string): TabSnapshot | null {
  try {
    const raw = JSON.parse(payload) as {
      v?: unknown;
      tab?: Partial<TabFields>;
      screen?: unknown;
      pending?: unknown;
    };
    const tab = raw.tab;
    if (
      raw.v !== VERSION ||
      !tab ||
      typeof tab.name !== 'string' ||
      (tab.target?.type !== 'serial' && tab.target?.type !== 'network') ||
      typeof raw.screen !== 'string' ||
      typeof raw.pending !== 'string'
    ) {
      return null;
    }
    return { tab: tab as TabFields, screen: raw.screen, pending: fromBase64(raw.pending) };
  } catch {
    return null;
  }
}
