/**
 * Clickable items in terminal output (P3-T11): IPv4 addresses and interface names (every
 * spelling, via the shared normaliser), and the vendor's commands for an interface.
 */
import { scanIfaces, type Iface } from '../lib/iface';

export type TermLink =
  | { kind: 'ip'; start: number; end: number; text: string }
  | { kind: 'iface'; start: number; end: number; text: string; iface: Iface };

const OCTET = '(?:25[0-5]|2[0-4]\\d|1\\d\\d|[1-9]?\\d)';
/** A dotted quad not inside a longer dotted number (OIDs, versions). */
const IPV4 = new RegExp(`(?<![\\d.])(?:${OCTET}\\.){3}${OCTET}(?![\\d]|\\.\\d)`, 'g');
const MAX_LINE = 1000;

export function findLinks(line: string): TermLink[] {
  if (line.length > MAX_LINE) return [];
  const out: TermLink[] = [];
  IPV4.lastIndex = 0;
  for (let m = IPV4.exec(line); m; m = IPV4.exec(line)) {
    out.push({ kind: 'ip', start: m.index, end: m.index + m[0].length, text: m[0] });
  }
  for (const h of scanIfaces(line)) {
    if (out.some((l) => h.start < l.end && h.end > l.start)) continue;
    out.push({
      kind: 'iface',
      start: h.start,
      end: h.end,
      text: line.slice(h.start, h.end),
      iface: h.iface,
    });
  }
  return out.sort((a, b) => a.start - b.start);
}

/** Read-only commands about one interface, in the device's own syntax. */
export function ifaceCommands(
  family: string | null,
  name: string,
): { show: string; config: string | null } {
  switch (family) {
    case 'huawei':
      return {
        show: `display interface ${name}`,
        config: `display current-configuration interface ${name}`,
      };
    case 'juniper':
      return { show: `show interfaces ${name}`, config: `show configuration interfaces ${name}` };
    case 'alcatel':
      return { show: `show interfaces ${name}`, config: null };
    case 'fortinet':
      return {
        show: `diagnose hardware deviceinfo nic ${name}`,
        config: `show system interface ${name}`,
      };
    default:
      return { show: `show interface ${name}`, config: `show running-config interface ${name}` };
  }
}
