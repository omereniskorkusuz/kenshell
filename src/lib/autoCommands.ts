/**
 * Post-connect commands (P2-T21, pure helpers; tested). A command is only sent when the
 * device has been quiet for a moment and the last line is not a login question, so on
 * Telnet/serial the user logs in first and nothing is typed into "Username:".
 */

export const QUIET_MS = 600;
/** Give up waiting for a prompt after this long (the tab stays as it is). */
export const MAX_WAIT_MS = 60_000;

const LOGIN_PROMPT = /(user ?name|login|password|passcode)\s*:\s*$/i;

/** Keeps the last `max` characters of (latin-1 decoded) output. */
export function appendTail(tail: string, bytes: Uint8Array, max = 256): string {
  let s = tail;
  for (const b of bytes) s += String.fromCharCode(b);
  return s.length > max ? s.slice(s.length - max) : s;
}

export function lastLine(tail: string): string {
  // eslint-disable-next-line no-control-regex -- strip ANSI escapes before matching
  const clean = tail.replace(/\x1b\[[0-9;?]*[ -/]*[@-~]/g, '');
  const lines = clean.split(/\r\n|\n|\r/);
  return lines[lines.length - 1] ?? '';
}

export function atLoginPrompt(tail: string): boolean {
  return LOGIN_PROMPT.test(lastLine(tail));
}

export function readyToSend(tail: string, lastDataAt: number, now: number): boolean {
  return now - lastDataAt >= QUIET_MS && !atLoginPrompt(tail);
}
