/**
 * AI assistant state (P5-T08). Text from the terminal is masked the moment it is
 * taken (`setContext`), so the preview the user reads is exactly what would be sent;
 * nothing is sent until the user presses Send. The client chunk is loaded on the
 * first send only.
 */
import { create } from 'zustand';
import type { AskError } from '../ai/client';
import { maskSecrets } from '../lib/aiMask';
import { api, inTauri } from '../lib/ipc';
import { getSetting } from './settings';

/** Longest terminal excerpt sent (characters); longer text keeps its end. */
export const MAX_CONTEXT = 24_000;

interface AiState {
  context: string;
  masked: number;
  trimmed: boolean;
  question: string;
  answer: string;
  running: boolean;
  error: AskError | { kind: 'noKey' } | null;
  setContext(text: string, question?: string): void;
  setQuestion(q: string): void;
  send(): Promise<void>;
  stop(): void;
  clear(): void;
}

let abort: AbortController | null = null;

export const useAi = create<AiState>()((set, get) => ({
  context: '',
  masked: 0,
  trimmed: false,
  question: '',
  answer: '',
  running: false,
  error: null,

  setContext(text, question) {
    const trimmed = text.length > MAX_CONTEXT;
    const m = maskSecrets(trimmed ? text.slice(-MAX_CONTEXT) : text);
    set({
      context: m.text,
      masked: m.count,
      trimmed,
      answer: '',
      error: null,
      ...(question !== undefined ? { question } : {}),
    });
  },

  setQuestion: (question) => set({ question }),

  async send() {
    const { context, question, running } = get();
    if (running || !question.trim()) return;
    abort = new AbortController();
    const signal = abort.signal;
    set({ running: true, answer: '', error: null });
    const onText = (delta: string) => {
      if (!signal.aborted) set({ answer: get().answer + delta });
    };
    try {
      if (!inTauri()) {
        // Browser demo: no network call.
        for (const w of 'Bu bir **demo** yanıtıdır.\n\n```\nshow interfaces status\n```'.split(
          /(?<= )/,
        )) {
          if (signal.aborted) break;
          onText(w);
          await new Promise((r) => setTimeout(r, 40));
        }
        return;
      }
      const apiKey = await api.aiKeyGet();
      if (!apiKey) {
        set({ error: { kind: 'noKey' } });
        return;
      }
      const { ask } = await import('../ai/client');
      // Re-mask the question too: it may be pasted config.
      const res = await ask({
        apiKey,
        model: getSetting('ai.model'),
        context,
        question: maskSecrets(question).text,
        onText,
        signal,
      });
      if (!res.ok && res.error.kind !== 'aborted') set({ error: res.error });
    } finally {
      set({ running: false });
      abort = null;
    }
  },

  stop() {
    abort?.abort();
  },

  clear() {
    abort?.abort();
    set({ context: '', masked: 0, trimmed: false, question: '', answer: '', error: null });
  },
}));
