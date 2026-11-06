/**
 * Multi-device read-only commands (P4-T11): the client-side mirror of the backend's
 * `device::capture::read_only` (the backend enforces it; this only greys out the button),
 * and merging per-device tables into one with a device column.
 */
import type { AnalyzerTable } from './ipc';

const SAFE_PIPES = new Set([
  'include',
  'exclude',
  'begin',
  'section',
  'count',
  'i',
  'e',
  'b',
  's',
  'inc',
  'exc',
  'beg',
  'sec',
  'no-more',
  'match',
  'except',
  'find',
]);

export function isReadOnly(command: string): boolean {
  const c = command.trim();
  // eslint-disable-next-line no-control-regex
  if (!c || c.length > 300 || /[\x00-\x1f\x7f;]/.test(c)) return false;
  const [head = '', ...pipes] = c.split('|');
  const first = (head.trim().split(/\s+/)[0] ?? '').toLowerCase();
  const verb = first.length >= 2 && ('show'.startsWith(first) || 'display'.startsWith(first));
  return verb && pipes.every((p) => SAFE_PIPES.has((p.trim().split(/\s+/)[0] ?? '').toLowerCase()));
}

export interface DeviceResult {
  device: string;
  table: AnalyzerTable | null;
}

/** One table when every device's output went through the same parser; else null. */
export function mergeTables(results: readonly DeviceResult[]): AnalyzerTable | null {
  const tables = results.map((r) => r.table);
  const first = tables[0];
  if (!first || tables.some((t) => !t || t.parser !== first.parser)) return null;
  return {
    parser: first.parser,
    command: first.command,
    columns: [{ key: 'device', kind: 'text' }, ...first.columns],
    rows: results.flatMap((r) => (r.table?.rows ?? []).map((row) => [r.device, ...row])),
    truncated: tables.some((t) => t?.truncated),
  };
}
