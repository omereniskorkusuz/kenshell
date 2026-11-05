/**
 * Command palette query prefixes: ">" = actions only, "@" = sessions only,
 * "/" = snippets only (P3-T17).
 */
export type PaletteMode = 'all' | 'actions' | 'sessions' | 'consoles' | 'snippets';

export function parsePaletteQuery(
  query: string,
  scope: 'all' | 'consoles' = 'all',
): { mode: PaletteMode; search: string } {
  const q = query.trimStart();
  if (q.startsWith('>')) return { mode: 'actions', search: q.slice(1).trim() };
  if (q.startsWith('@')) return { mode: 'sessions', search: q.slice(1).trim() };
  if (q.startsWith('/')) return { mode: 'snippets', search: q.slice(1).trim() };
  return { mode: scope === 'consoles' ? 'consoles' : 'all', search: q.trim() };
}

/** Boolean settings the palette can switch (P3-T17), with their label keys. */
export const PALETTE_TOGGLES = [
  ['terminal.highlight', 'settings.highlight'],
  ['terminal.autocomplete', 'settings.autocomplete'],
  ['terminal.autoMore', 'settings.autoMore'],
  ['terminal.timestamps', 'settings.lineTimestamps'],
  ['terminal.copyOnSelect', 'settings.copyOnSelect'],
  ['terminal.rightClickPaste', 'settings.rightClickPaste'],
  ['terminal.cursorBlink', 'settings.cursorBlink'],
  ['device.identify', 'settings.identify'],
  ['log.enabled', 'settings.logEnabled'],
  ['console.hotplugToast', 'settings.hotplugToast'],
  ['console.autoReconnect', 'settings.autoReconnect'],
  ['connection.autoReconnect', 'settings.netAutoReconnect'],
] as const;
