/**
 * Ping / traceroute from this computer (P5-T05). Global so a run keeps going while the
 * panel shows another tool, and so the terminal's IP menu can start one directly.
 */
import { create } from 'zustand';
import { diag, type DiagKind } from '../lib/ipc';

const MAX_LINES = 500;

interface DiagState {
  kind: DiagKind;
  target: string;
  /** Ping count; null = until stopped. */
  count: number | null;
  resolve: boolean;
  lines: string[];
  running: number | null;
  /** Exit code of the last run (null = stopped), undefined before any run. */
  exit: number | null | undefined;
  error: unknown;
  setOptions(o: Partial<Pick<DiagState, 'kind' | 'target' | 'count' | 'resolve'>>): void;
  start(o?: Partial<Pick<DiagState, 'kind' | 'target'>>): Promise<void>;
  stop(): void;
}

let nextJob = 1;

export const useDiag = create<DiagState>()((set, get) => ({
  kind: 'ping',
  target: '',
  count: 4,
  resolve: false,
  lines: [],
  running: null,
  exit: undefined,
  error: null,

  setOptions: (o) => set(o),

  async start(o) {
    if (o) set(o);
    const { running, kind, target, count, resolve } = get();
    if (running !== null) await diag.stop(running).catch(() => {});
    const job = nextJob++;
    set({ running: job, lines: [], exit: undefined, error: null });
    const onLine = (line: string) => {
      if (get().running !== job) return;
      const lines = [...get().lines, line];
      set({ lines: lines.length > MAX_LINES ? lines.slice(-MAX_LINES) : lines });
    };
    try {
      const exit = await diag.run(job, kind, target.trim(), count, resolve, onLine);
      if (get().running === job) set({ running: null, exit });
    } catch (e) {
      if (get().running === job) set({ running: null, error: e });
    }
  },

  stop() {
    const job = get().running;
    if (job !== null) void diag.stop(job).catch(() => {});
  },
}));
