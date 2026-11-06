/**
 * Embedded file servers (P5-T02 TFTP, P5-T03 FTP / SCP): global so the status bar shows
 * what is running even with the panel closed.
 */
import { create } from 'zustand';
import {
  fileServers,
  type ServerEvent,
  type ServerKind,
  type ServerOptions,
  type ServerStatus,
} from '../lib/ipc';

export interface ServedTransfer {
  key: string;
  server: ServerKind;
  peer: string;
  file: string;
  upload: boolean;
  size: number | null;
  bytes: number;
  state: 'running' | 'done' | 'failed';
  error?: string;
}

interface ServersState {
  servers: ServerStatus[];
  transfers: ServedTransfer[];
  start(opts: ServerOptions): Promise<void>;
  stop(kind: ServerKind): Promise<void>;
}

const MAX = 50;

export const useServers = create<ServersState>()((set, get) => ({
  servers: [],
  transfers: [],

  async start(opts) {
    const server = opts.kind;
    const onEvent = (e: ServerEvent) => {
      const key = `${server}-${e.id}`;
      const list = get().transfers;
      if (e.kind === 'started') {
        set({
          transfers: [
            {
              key,
              server,
              peer: e.peer,
              file: e.file,
              upload: e.upload,
              size: e.size,
              bytes: 0,
              state: 'running' as const,
            },
            ...list,
          ].slice(0, MAX),
        });
        return;
      }
      set({
        transfers: list.map((t) =>
          t.key !== key
            ? t
            : e.kind === 'progress'
              ? { ...t, bytes: e.bytes }
              : e.kind === 'done'
                ? { ...t, bytes: e.bytes, state: 'done' as const }
                : { ...t, state: 'failed' as const, error: e.error },
        ),
      });
    };
    // A restarted server numbers its transfers from 1 again.
    set({ transfers: get().transfers.filter((t) => t.server !== server) });
    set({ servers: await fileServers.start(opts, onEvent) });
  },

  async stop(kind) {
    set({ servers: await fileServers.stop(kind) });
  },
}));
