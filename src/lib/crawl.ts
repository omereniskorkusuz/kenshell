/**
 * Automatic discovery: from the devices already read (sessions, SNMP hosts), follow the
 * neighbours they report and read those over SNMP too, and their neighbours in turn,
 * until nothing new turns up. Read-only and bounded: only private addresses that a device
 * itself advertised (LLDP / CDP management address, default gateway), only devices that
 * forward traffic (switches, routers, access points, firewalls), and at most `max` of them.
 */
import type { Discovery } from './ipc';
import { kindOf } from './topologyGraph';

export const CRAWL_MAX = 200;
export const CRAWL_PARALLEL = 4;

export interface CrawlCandidate {
  ip: string;
  /** Name the device announced, for the progress list. */
  name: string;
  /** Who reported it. */
  from: string;
  /** Hops from the first device read. */
  depth: number;
}

const IPV4 = /^(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.(\d{1,3})$/;

/** Addresses that may be polled: private (RFC 1918, carrier-grade NAT), not special. */
export function crawlable(ip: string | null | undefined): ip is string {
  const m = IPV4.exec((ip ?? '').split('/')[0]?.trim() ?? '');
  if (!m) return false;
  const [a, b] = [Number(m[1]), Number(m[2])];
  if ([m[1], m[2], m[3], m[4]].some((x) => Number(x) > 255)) return false;
  if (Number(m[4]) === 0 || Number(m[4]) === 255) return false;
  return (
    a === 10 ||
    (a === 172 && b >= 16 && b <= 31) ||
    (a === 192 && b === 168) ||
    (a === 100 && b >= 64 && b <= 127)
  );
}

/** Devices worth reading: those that forward traffic; end stations and phones are not. */
function worth(capability: string | null, platform: string | null): boolean {
  const kind = kindOf(capability, platform);
  return (
    kind === null || kind === 'switch' || kind === 'router' || kind === 'ap' || kind === 'firewall'
  );
}

export interface Known {
  /** What the device reported; null while not read yet. */
  discovery: Discovery | null;
  name: string;
  depth: number;
}

/**
 * The next devices to read: reported by an already-read device, not read or tried yet
 * (`seen` holds every address already handled, in any state), at most `room` of them,
 * nearest first.
 */
export function nextCandidates(
  known: readonly Known[],
  seen: ReadonlySet<string>,
  room: number,
): CrawlCandidate[] {
  const out = new Map<string, CrawlCandidate>();
  for (const k of known) {
    const d = k.discovery;
    if (!d) continue;
    const add = (ip: string | null, name: string) => {
      const clean = ip?.split('/')[0]?.trim() ?? '';
      if (!crawlable(clean) || seen.has(clean)) return;
      const prev = out.get(clean);
      // Reported by several devices: the shortest way there counts.
      if (prev && prev.depth <= k.depth + 1) return;
      out.set(clean, { ip: clean, name, from: k.name, depth: k.depth + 1 });
    };
    for (const n of d.neighbors) if (worth(n.capability, n.platform)) add(n.ip, n.name);
    add(d.gateway, d.gateway ?? '');
  }
  return [...out.values()].sort((a, b) => a.depth - b.depth).slice(0, Math.max(0, room));
}
