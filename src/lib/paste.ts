/**
 * Smart paste (P2-T20, pure; tested). Clipboard text is normalized to what a device
 * expects from Enter (CR). On slow links (serial, telnet) multi-line pastes are sent line
 * by line with a small delay so the device's input buffer does not overflow. No
 * confirmation dialog (product rule).
 */
import type { Protocol } from './ipc';

export const DEFAULT_PASTE_DELAY_MS = 50;

/** CRLF / LF / CR line ends → CR; a trailing newline is kept (it means "press Enter"). */
export function normalizePaste(text: string): string {
  return text.replace(/\r\n|\n|\r/g, '\r');
}

/** Splits into chunks of one line each (each ending with CR except maybe the last). */
export function pasteChunks(text: string): string[] {
  const norm = normalizePaste(text);
  const parts = norm.split(/(?<=\r)/);
  return parts.filter((p) => p.length > 0);
}

/** Line-by-line pacing only where it helps (slow serial/telnet devices). */
export function wantsPacing(protocol: Protocol, chunks: number, delayMs: number): boolean {
  return delayMs > 0 && chunks > 1 && (protocol === 'serial' || protocol === 'telnet');
}
