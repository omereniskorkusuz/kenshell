/** Silent toasts (no sound, no dialogs). Bottom-right above the status bar. */
import { create } from 'zustand';

export interface ToastAction {
  label: string;
  run: () => void;
  /** Primary actions get the accent style. */
  primary?: boolean;
}

export type ToastKind = 'info' | 'success' | 'error';

export interface Toast {
  id: number;
  kind: ToastKind;
  message: string;
  /** Technical detail for a tooltip (never shown as the message). */
  detail?: string | null;
  actions: ToastAction[];
  durationMs: number;
}

export type ToastInput = Partial<Omit<Toast, 'id' | 'message'>> & { message: string };

export const DEFAULT_TOAST_MS = 8000;
const MAX_TOASTS = 4;

interface ToastState {
  toasts: Toast[];
  push(t: ToastInput): number;
  dismiss(id: number): void;
}

let nextId = 1;

export const useToasts = create<ToastState>()((set) => ({
  toasts: [],
  push(input) {
    const toast: Toast = {
      kind: 'info',
      actions: [],
      durationMs: DEFAULT_TOAST_MS,
      ...input,
      id: nextId++,
    };
    set((s) => ({ toasts: [...s.toasts, toast].slice(-MAX_TOASTS) }));
    return toast.id;
  },
  dismiss(id) {
    set((s) => ({ toasts: s.toasts.filter((t) => t.id !== id) }));
  },
}));

export const toast = (t: ToastInput): number => useToasts.getState().push(t);
