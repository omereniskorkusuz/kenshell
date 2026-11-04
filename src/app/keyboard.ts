/**
 * Global shortcut dispatcher. Listens in the CAPTURE phase on window so app shortcuts win over
 * the focused terminal (xterm never sees a key we handled).
 */
import { isMac } from '../lib/ipc';
import { matchChord, parseChord, type Chord } from '../lib/shortcuts';
import { useUi } from '../stores/ui';
import { ACTIONS, isEnabled, type AppAction } from './actions';

const REPEATABLE = new Set(['tab.next', 'tab.prev']);

interface Bound {
  chord: Chord;
  action: AppAction;
}

export function installShortcuts(): () => void {
  const mac = isMac();
  const bound: Bound[] = ACTIONS.filter((a) => a.shortcut).map((action) => ({
    chord: parseChord(action.shortcut as string),
    action,
  }));

  const onKeyDown = (e: KeyboardEvent) => {
    if (e.isComposing) return;
    // Plain keys (no modifier) are never app shortcuts: they belong to the focused control.
    if (!e.metaKey && !e.ctrlKey && !e.altKey) return;
    const ui = useUi.getState();
    // Forms/dialogs own the keyboard; only the palette toggle stays global.
    const modal = ui.editor !== null || ui.settingsOpen;
    for (const { chord, action } of bound) {
      if (!matchChord(chord, e, mac)) continue;
      if (modal && action.id !== 'palette.open') return;
      if (ui.paletteOpen && action.id !== 'palette.open') return;
      // Always swallow the key (a held ⌘W must not leak ^W into the terminal), but only
      // repeat actions that make sense when held.
      e.preventDefault();
      e.stopPropagation();
      if (e.repeat && !REPEATABLE.has(action.id)) return;
      if (isEnabled(action)) action.run();
      return;
    }
  };

  window.addEventListener('keydown', onKeyDown, { capture: true });
  return () => window.removeEventListener('keydown', onKeyDown, { capture: true });
}
