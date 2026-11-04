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
