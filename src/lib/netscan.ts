/**
 * Network scanner helpers (P4-T14, R-T24): IPv4 arithmetic, free-address blocks, grid
 * cells, search and CSV rows. A scan result holds only the live hosts plus the scanned
 * range; everything else in the range "probably free".
 */
export const ipToNum = (ip: string): number =>
  ip.split('.').reduce((n, o) => n * 256 + (Number(o) & 255), 0);

export const numToIp = (n: number): string =>
  [24, 16, 8, 0].map((s) => Math.floor(n / 2 ** s) % 256).join('.');

/** A live host. */
export interface ScanHost {
  ip: string;
  /** Ping round-trip time; null = found by TCP or the ARP table only. */
  rttMs: number | null;
  openPorts: number[];
  mac: string | null;
  /** Filled from the OUI table (P4-T15). */
  vendor: string | null;
  /** Reverse DNS, otherwise NetBIOS. */
  hostname: string | null;
  own: boolean;
  gateway: boolean;
}

export interface ScanRange {
  first: string;
  last: string;
}

export interface FreeBlock {
  first: string;
  last: string;
  count: number;
}

const byIp = (hosts: readonly ScanHost[]) =>
  [...hosts].sort((a, b) => ipToNum(a.ip) - ipToNum(b.ip));

/** Addresses of `range` that did not answer, as consecutive blocks in order. */
export function freeBlocks(range: ScanRange, hosts: readonly ScanHost[]): FreeBlock[] {
  const out: FreeBlock[] = [];
  const last = ipToNum(range.last);
  let next = ipToNum(range.first);
  const gapTo = (end: number) => {
    if (end >= next) out.push({ first: numToIp(next), last: numToIp(end), count: end - next + 1 });
  };
  for (const h of byIp(hosts)) {
    const n = ipToNum(h.ip);
    if (n < next || n > last) continue;
    gapTo(n - 1);
    next = n + 1;
  }
  gapTo(last);
  return out;
}

/** `192.168.1.50 – .89` (the shared prefix written once). */
export function blockLabel(b: FreeBlock): string {
  if (b.first === b.last) return b.first;
  const f = b.first.split('.');
  const l = b.last.split('.');
  let same = 0;
  while (same < 3 && f[same] === l[same]) same++;
  return `${b.first} – .${l.slice(same).join('.')}`;
}

/** Every address of the block, one per line (for copying). */
export function blockAddresses(b: FreeBlock): string {
  const out: string[] = [];
  for (let n = ipToNum(b.first); n <= ipToNum(b.last); n++) out.push(numToIp(n));
  return out.join('\n');
}

export type CellKind = 'used' | 'free' | 'own' | 'gateway' | 'outside';

/** Up to this many /24 blocks every block of the range is drawn; above, only those with a live host. */
export const GRID_ALL_BLOCKS = 4;

/**
 * Grid cells per /24 block: every address of the block; addresses outside the range
 * (network, broadcast, not asked) are `outside`. A wide range draws only the blocks
 * that hold a live host.
 */
export function gridCells(
  range: ScanRange,
  hosts: readonly ScanHost[],
): { base: string; cells: { ip: string; kind: CellKind; host?: ScanHost }[] }[] {
  const first = ipToNum(range.first);
  const last = ipToNum(range.last);
  const byNum = new Map(hosts.map((h) => [ipToNum(h.ip), h]));
  const firstBase = first - (first % 256);
  const blocks = Math.floor((last - firstBase) / 256) + 1;
  const bases =
    blocks <= GRID_ALL_BLOCKS
      ? Array.from({ length: blocks }, (_, i) => firstBase + i * 256)
      : [...new Set([...byNum.keys()].map((n) => n - (n % 256)))].sort((a, b) => a - b);
  return bases.map((base) => ({
    base: numToIp(base),
    cells: Array.from({ length: 256 }, (_, i) => {
      const n = base + i;
      const h = byNum.get(n);
      const kind: CellKind =
        n < first || n > last
          ? 'outside'
          : !h
            ? 'free'
            : h.own
              ? 'own'
              : h.gateway
                ? 'gateway'
                : 'used';
      return { ip: numToIp(n), kind, ...(h ? { host: h } : {}) };
    }),
  }));
}

/** Hosts whose IP, name, MAC or vendor contain `query` (case and MAC separators ignored). */
export function filterHosts(hosts: readonly ScanHost[], query: string): ScanHost[] {
  const q = query.trim().toLocaleLowerCase('tr');
  if (!q) return [...hosts];
  const bare = (s: string) => s.replace(/[^0-9a-f]/g, '');
  const qMac = bare(q);
  return hosts.filter((h) => {
    const text = [h.ip, h.hostname, h.vendor, h.mac]
      .filter(Boolean)
      .join(' ')
      .toLocaleLowerCase('tr');
    return text.includes(q) || (qMac.length >= 4 && h.mac !== null && bare(h.mac).includes(qMac));
  });
}

/** `0.4 ms`, `12 ms`, `<1 ms` (the system ping's `<1ms`); `ARP` / null when not pinged. */
export function rttLabel(h: ScanHost): string | null {
  if (h.rttMs === null) return h.mac ? 'ARP' : null;
  if (h.rttMs === 0) return '<1 ms';
  return `${h.rttMs < 10 ? h.rttMs.toFixed(1) : Math.round(h.rttMs)} ms`;
}

export const CSV_HEADER = [
  'ip',
  'status',
  'rttMs',
  'hostname',
  'mac',
  'vendor',
  'openPorts',
] as const;

export function csvRows(hosts: readonly ScanHost[], status: (h: ScanHost) => string): string[][] {
  return byIp(hosts).map((h) => [
    h.ip,
    status(h),
    h.rttMs === null ? '' : String(h.rttMs),
    h.hostname ?? '',
    h.mac ?? '',
    h.vendor ?? '',
    h.openPorts.join(' '),
  ]);
}
