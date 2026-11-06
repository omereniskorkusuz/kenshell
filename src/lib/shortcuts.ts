/**
 * Keyboard shortcut notation, matching and platform-aware display.
 *
 * Notation: "mod+shift+b", "ctrl+tab", "mod+,". `mod` = ⌘ on macOS, Ctrl elsewhere.
 * Keys are matched on `KeyboardEvent.key` (lower-cased), with `code` as a fallback for
 * digits and letters so Shift/Alt/keyboard layouts do not break them.
 */

export interface Chord {
  mod: boolean;
  ctrl: boolean;
  shift: boolean;
  alt: boolean;
  key: string;
}

export function parseChord(spec: string): Chord {
  const parts = spec.toLowerCase().split('+');
  // "mod++" style is not used; the last part is the key ("," and "[" are fine).
  const key = parts.pop() ?? '';
  return {
    mod: parts.includes('mod'),
    ctrl: parts.includes('ctrl'),
    shift: parts.includes('shift'),
    alt: parts.includes('alt'),
    key,
  };
}

export interface KeyLike {
  key: string;
  code?: string;
  metaKey: boolean;
  ctrlKey: boolean;
  shiftKey: boolean;
  altKey: boolean;
}

function keyOf(e: KeyLike): string[] {
  const keys = [e.key.toLowerCase()];
  const code = e.code ?? '';
  if (/^Key[A-Z]$/.test(code)) keys.push(code.slice(3).toLowerCase());
  else if (/^Digit\d$/.test(code)) keys.push(code.slice(5));
  else if (code === 'Comma') keys.push(',');
  else if (code === 'BracketLeft') keys.push('[');
  else if (code === 'BracketRight') keys.push(']');
  return keys;
}

export function matchChord(chord: Chord, e: KeyLike, mac: boolean): boolean {
  const wantMeta = mac && chord.mod;
  const wantCtrl = chord.ctrl || (!mac && chord.mod);
  if (e.metaKey !== wantMeta) return false;
  if (e.ctrlKey !== wantCtrl) return false;
  if (e.shiftKey !== chord.shift) return false;
  if (e.altKey !== chord.alt) return false;
  return keyOf(e).includes(chord.key);
}

const MAC_SYMBOLS: Record<string, string> = { mod: '⌘', ctrl: '⌃', shift: '⇧', alt: '⌥' };
const KEY_LABELS: Record<string, string> = {
  tab: 'Tab',
  enter: 'Enter',
  escape: 'Esc',
  arrowleft: '←',
  arrowright: '→',
  arrowup: '↑',
  arrowdown: '↓',
  pageup: 'PgUp',
  pagedown: 'PgDn',
  ',': ',',
};

/** "mod+shift+b" → "⇧⌘B" on macOS, "Ctrl+Shift+B" elsewhere. */
export function formatChord(spec: string, mac: boolean): string {
  const c = parseChord(spec);
  const key = KEY_LABELS[c.key] ?? c.key.toUpperCase();
  if (mac) {
    // macOS order: ⌃ ⌥ ⇧ ⌘ key
    let s = '';
    if (c.ctrl) s += MAC_SYMBOLS.ctrl;
    if (c.alt) s += MAC_SYMBOLS.alt;
    if (c.shift) s += MAC_SYMBOLS.shift;
    if (c.mod) s += MAC_SYMBOLS.mod;
    return s + key;
  }
  const parts: string[] = [];
  if (c.mod || c.ctrl) parts.push('Ctrl');
  if (c.alt) parts.push('Alt');
  if (c.shift) parts.push('Shift');
  parts.push(key);
  return parts.join('+');
}

// ------------------------------------------------------------ customization (P5-T07)

const MOD_KEYS = new Set(['meta', 'control', 'shift', 'alt', 'os', 'altgraph', 'capslock']);

/** Keys an app shortcut may never take: clipboard / undo / quit belong to the OS. */
export const RESERVED = ['mod+c', 'mod+v', 'mod+x', 'mod+a', 'mod+z', 'mod+q'] as const;

/** Canonical notation (modifier order mod, ctrl, alt, shift) so equal chords compare equal. */
export function normalizeSpec(spec: string): string {
  const c = parseChord(spec);
  return [c.mod && 'mod', c.ctrl && 'ctrl', c.alt && 'alt', c.shift && 'shift', c.key]
    .filter(Boolean)
    .join('+');
}

/**
 * The notation of a keydown being recorded as a new shortcut, or null when it cannot be
 * one: a lone modifier, or no ⌘/Ctrl/Alt (plain keys belong to the terminal).
 */
export function chordFromEvent(e: KeyLike, mac: boolean): string | null {
  const raw = e.key.toLowerCase();
  if (MOD_KEYS.has(raw)) return null;
  const code = e.code ?? '';
  const key = /^Key[A-Z]$/.test(code)
    ? code.slice(3).toLowerCase()
    : /^Digit\d$/.test(code)
      ? code.slice(5)
      : raw === ' '
        ? 'space'
        : raw;
  if (!key || key.includes('+')) return null;
  const mod = mac ? e.metaKey : e.ctrlKey;
  const ctrl = mac && e.ctrlKey;
  if (!mod && !ctrl && !e.altKey) return null;
  return [mod && 'mod', ctrl && 'ctrl', e.altKey && 'alt', e.shiftKey && 'shift', key]
    .filter(Boolean)
    .join('+');
}

const SPEC = /^((mod|ctrl|alt|shift)\+)+[^+\s]{1,12}$/;

/** User overrides: action id → notation, or null for "no shortcut". Invalid entries dropped. */
export function sanitizeOverrides(v: unknown): Record<string, string | null> {
  if (!v || typeof v !== 'object' || Array.isArray(v)) return {};
  const out: Record<string, string | null> = {};
  for (const [id, spec] of Object.entries(v as Record<string, unknown>).slice(0, 200)) {
    if (!/^[a-zA-Z][\w.]{0,59}$/.test(id)) continue;
    if (spec === null) out[id] = null;
    else if (typeof spec === 'string' && SPEC.test(spec)) out[id] = normalizeSpec(spec);
  }
  return out;
}

/** The shortcut in effect: the override if there is one (null = none), else the default. */
export function effective(
  id: string,
  fallback: string | undefined,
  overrides: Readonly<Record<string, string | null>>,
): string | undefined {
  if (id in overrides) return overrides[id] ?? undefined;
  return fallback;
}
