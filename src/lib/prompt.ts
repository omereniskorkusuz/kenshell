/**
 * PuTTY-style login prompts answered inside the terminal (pure; tested). The backend asks
 * (`SessionEvent.prompt`); keystrokes are collected locally — never sent to the device —
 * and secret items are not echoed. Enter moves to the next item; after the last one the
 * answers are submitted.
 */
import type { PromptRequest } from './ipc';

export interface PendingPrompt {
  request: PromptRequest;
  /** Index of the item being answered. */
  index: number;
  answers: string[];
  buffer: string;
}

export interface PromptStep {
  prompt: PendingPrompt | null;
  /** Text to write to the terminal (echo, newlines, next question). */
  echo: string;
  /** Set when all items were answered. */
  submit: string[] | null;
  /** Ctrl-C: abort the login. */
  cancel: boolean;
}

const CRLF = '\r\n';

/** Terminal text shown when a prompt starts. */
export function promptIntro(request: PromptRequest): string {
  const instruction = request.instruction.trim()
    ? request.instruction.replace(/\r?\n/g, CRLF) + CRLF
    : '';
  return CRLF + instruction + (request.items[0]?.text ?? '');
}

export function startPrompt(request: PromptRequest): PendingPrompt {
  return { request, index: 0, answers: [], buffer: '' };
}

export function feedPrompt(p: PendingPrompt, data: string): PromptStep {
  const cur: PendingPrompt = { ...p, answers: [...p.answers] };
  let echo = '';
  // Drop terminal escape sequences (arrows, function keys) entirely.
  // eslint-disable-next-line no-control-regex -- ESC is exactly what we strip here
  const text = data.replace(/\x1b(\[[0-9;?]*[ -/]*[@-~]|O.|.)/g, '');
  for (const ch of text) {
    const item = cur.request.items[cur.index];
    if (!item) break;
    if (ch === '\x03') return { prompt: null, echo: '^C' + CRLF, submit: null, cancel: true };
    if (ch === '\r' || ch === '\n') {
      cur.answers.push(cur.buffer);
      cur.buffer = '';
      cur.index += 1;
      echo += CRLF;
      const next = cur.request.items[cur.index];
      if (!next) return { prompt: null, echo, submit: cur.answers, cancel: false };
      echo += next.text;
      continue;
    }
    if (ch === '\x7f' || ch === '\b') {
      if (cur.buffer.length > 0) {
        cur.buffer = [...cur.buffer].slice(0, -1).join('');
        if (item.echo) echo += '\b \b';
      }
      continue;
    }
    if (ch === '\x15') {
      // Ctrl-U: clear the line.
      if (item.echo) echo += '\b \b'.repeat([...cur.buffer].length);
      cur.buffer = '';
      continue;
    }
    if (ch < ' ') continue;
    cur.buffer += ch;
    if (item.echo) echo += ch;
  }
  return { prompt: cur, echo, submit: null, cancel: false };
}
