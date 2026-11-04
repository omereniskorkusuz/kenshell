/** Detected USB/serial consoles + driver hints, kept live via `console://changed`. */
import { create } from 'zustand';
import { t } from '../i18n';
import { describeError } from '../lib/errors';
import {
  api,
  onConsoleChanged,
  type ConsoleChange,
  type ConsolePort,
  type ConsoleSnapshot,
} from '../lib/ipc';
import { getSetting } from './settings';
import { useTabs } from './tabs';
import { toast } from './toasts';

/** Quiet highlight for a newly attached row (Concept: 600 ms). */
export const HIGHLIGHT_MS = 600;

interface ConsolesState {
  snapshot: ConsoleSnapshot;
  loaded: boolean;
  highlighted: ReadonlySet<string>;
  rescanning: boolean;
  load(): Promise<void>;
  rescan(): Promise<void>;
  subscribe(): Promise<() => void>;
  apply(change: ConsoleChange): void;
}

/** "FTDI FT232R", or the best human label we have. */
export function chipLabel(port: ConsolePort): string {
  if (port.chip) return `${port.chip.vendor} ${port.chip.model}`;
  return port.product || port.manufacturer || t('consoles.unknownChip');
}

export const useConsoles = create<ConsolesState>()((set, get) => ({
  snapshot: { ports: [], driverHints: [] },
  loaded: false,
  highlighted: new Set<string>(),
  rescanning: false,

  async load() {
    try {
      set({ snapshot: await api.consoleList(), loaded: true });
    } catch (e) {
      set({ loaded: true });
      const d = describeError(e, t);
      toast({ kind: 'error', message: d.message, detail: d.detail });
    }
  },

  async rescan() {
    set({ rescanning: true });
    try {
      set({ snapshot: await api.consoleRescan() });
    } catch (e) {
      const d = describeError(e, t);
      toast({ kind: 'error', message: d.message, detail: d.detail });
    } finally {
      set({ rescanning: false });
    }
  },

  subscribe() {
    return onConsoleChanged((change) => get().apply(change));
  },

  apply(change) {
    set({ snapshot: change.snapshot });
    const added = change.snapshot.ports.filter((p) => change.attached.includes(p.key));
    if (added.length === 0) return;

    set({ highlighted: new Set([...get().highlighted, ...added.map((p) => p.key)]) });
    setTimeout(() => {
      const next = new Set(get().highlighted);
      added.forEach((p) => next.delete(p.key));
      set({ highlighted: next });
    }, HIGHLIGHT_MS);

    if (!getSetting('console.hotplugToast')) return;
    for (const port of added) {
      // A tab that is waiting for this cable reconnects on its own; no toast needed.
      const waiting = useTabs
        .getState()
        .tabs.some((x) => x.settings.port === port.path && x.health === 'cableRemoved');
      if (waiting) continue;
      toast({
        message: t('consoles.detectedToast', { chip: chipLabel(port), port: port.displayName }),
        actions: [
          {
            label: t('consoles.connect'),
            primary: true,
            run: () => useTabs.getState().openConsole(port),
          },
          { label: t('common.dismiss'), run: () => {} },
        ],
      });
    }
  },
}));
