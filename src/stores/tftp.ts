/**
 * TFTP server state (P5-T02): global so the status bar shows it while it runs, even with
 * the panel closed.
 */
import { create } from 'zustand';
import { tftp, type TftpEvent, type TftpStatus } from '../lib/ipc';

export interface TftpTransfer {
  id: number;
  peer: string;
  file: string;
  upload: boolean;
  size: number | null;
  bytes: number;
  state: 'running' | 'done' | 'failed';
  error?: string;
}

interface TftpState {
  status: TftpStatus;
  transfers: TftpTransfer[];
  start(opts: {
    root: string;
    bindIp: string;
    port: number;
    allowUpload: boolean;
    overwrite: boolean;
  }): Promise<void>;
  stop(): Promise<void>;
}

const MAX = 50;

export const useTftp = create<TftpState>()((set, get) => ({
  status: { running: false, addr: null, root: null },
  transfers: [],

  async start(opts) {
    const onEvent = (e: TftpEvent) => {
      const list = get().transfers;
      if (e.kind === 'started') {
        set({
          transfers: [
            {
              id: e.id,
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
          t.id !== e.id
            ? t
            : e.kind === 'progress'
              ? { ...t, bytes: e.bytes }
              : e.kind === 'done'
                ? { ...t, bytes: e.bytes, state: 'done' as const }
                : { ...t, state: 'failed' as const, error: e.error },
        ),
      });
    };
    set({ status: await tftp.start(opts, onEvent), transfers: [] });
  },

  async stop() {
    set({ status: await tftp.stop() });
  },
}));
