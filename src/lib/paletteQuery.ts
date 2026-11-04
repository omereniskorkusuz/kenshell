/** Command palette query prefixes: ">" = actions only, "@" = sessions only. */
export type PaletteMode = 'all' | 'actions' | 'sessions' | 'consoles';

export function parsePaletteQuery(
  query: string,
  scope: 'all' | 'consoles' = 'all',
): { mode: PaletteMode; search: string } {
  const q = query.trimStart();
  if (q.startsWith('>')) return { mode: 'actions', search: q.slice(1).trim() };
  if (q.startsWith('@')) return { mode: 'sessions', search: q.slice(1).trim() };
  return { mode: scope === 'consoles' ? 'consoles' : 'all', search: q.trim() };
}
