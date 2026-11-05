/**
 * Command history and dim autocomplete (P3-T12), pure parts.
 *
 * `LineTracker` follows what the user typed on the current line from the keystrokes sent;
 * anything it cannot follow (cursor keys, the device's own Tab completion, a paste) makes the
 * line unknown until the next Enter. A line is only recorded when the screen showed it
 * (devices do not echo passwords) and it does not look like it carries a secret.
 */
import builtins from '../data/commands.json';

/** Same words as `storage/history.rs`. */
const SECRET = /passw|secret|communit|key|psk|passphrase|token|credential|auth/i;
export const MAX_COMMAND = 300;
/** Characters typed before a suggestion is shown. */
export const MIN_PREFIX = 2;

export function storable(command: string): boolean {
  const c = command.trim();
  // eslint-disable-next-line no-control-regex
  return c.length > 0 && c.length <= MAX_COMMAND && !/[\x00-\x1f\x7f]/.test(c) && !SECRET.test(c);
}

export class LineTracker {
  /** Typed text of the current line; null = unknown. */
  line: string | null = '';

  /** Feeds data sent to the device; returns the finished lines (Enter). */
  feed(data: string): string[] {
    const done: string[] = [];
    for (let i = 0; i < data.length; i++) {
      const ch = data[i] as string;
      if (ch === '\r' || ch === '\n') {
        if (this.line) done.push(this.line);
        this.line = '';
        if (ch === '\r' && data[i + 1] === '\n') i++;
      } else if (ch === '\x7f' || ch === '\b') {
        if (this.line) this.line = this.line.slice(0, -1);
      } else if (ch === '\x03' || ch === '\x15' || ch === '\x1a') {
        this.line = ''; // ^C, ^U, ^Z: the line is gone
      } else if (ch === '\x1b') {
        this.line = null; // cursor keys, history recall …
        // Skip the rest of a CSI/SS3 sequence.
        if (data[i + 1] === '[' || data[i + 1] === 'O') {
          i += 2;
          while (i < data.length && !/[@-~]/.test(data[i] as string)) i++;
        }
      } else if (ch < ' ') {
        this.line = null; // Tab (device completion), ^A/^E/^W …
      } else if (this.line !== null) {
        this.line += ch;
      }
    }
    return done;
  }

  /** A paste or anything else that bypasses `feed`. */
  lose(): void {
    this.line = null;
  }
}

/** The rest of the best match for `typed`: history (most recent first), then built-ins. */
export function suggest(
  typed: string,
  history: readonly string[],
  family: string | null,
): string | null {
  if (typed.length < MIN_PREFIX || typed.trim() === '') return null;
  const t = typed.toLowerCase();
  const pick = (list: readonly string[]) =>
    list.find((c) => c.length > typed.length && c.toLowerCase().startsWith(t));
  const sets = builtins as unknown as Record<string, string[]>;
  const hit = pick(history) ?? pick(sets[family ?? ''] ?? sets.default ?? []);
  return hit ? hit.slice(typed.length) : null;
}
