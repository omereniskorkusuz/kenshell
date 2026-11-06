/**
 * Network scanner helpers (P4-T14): IPv4 arithmetic, free-address blocks, grid cells and
 * CSV rows from a scan result.
 */
export const ipToNum = (ip: string): number =>
  ip.split('.').reduce((n, o) => n * 256 + (Number(o) & 255), 0);

export const numToIp = (n: number): string =>
  [24, 16, 8, 0].map((s) => Math.floor(n / 2 ** s) % 256).join('.');

export interface ScanHost {
  ip: string;
  alive: boolean;
  icmp: boolean;
  openPorts: number[];
  mac: string | null;
  hostname: string | null;
  own: boolean;
  gateway: boolean;
  /** Filled from the OUI table (P4-T15). */
  vendor?: string | null;
}

export interface FreeBlock {
  first: string;
  last: string;
  count: number;
}

/** Consecutive addresses that did not answer, in order. */
export function freeBlocks(hosts: readonly ScanHost[]): FreeBlock[] {
  const sorted = [...hosts].sort((a, b) => ipToNum(a.ip) - ipToNum(b.ip));
  const out: FreeBlock[] = [];
  let start: number | null = null;
  let prev = -2;
  const close = () => {
    if (start !== null)
      out.push({ first: numToIp(start), last: numToIp(prev), count: prev - start + 1 });
    start = null;
  };
  for (const h of sorted) {
    const n = ipToNum(h.ip);
    if (h.alive || n !== prev + 1) close();
    if (!h.alive && start === null) start = n;
    prev = n;
  }
  close();
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

/**
 * Grid cells for /24 blocks: every address of each /24 the scan touched; addresses that
 * were not scanned (network, broadcast, outside the range) are `outside`.
 */
export function gridCells(
  hosts: readonly ScanHost[],
): { base: string; cells: { ip: string; kind: CellKind; host?: ScanHost }[] }[] {
  const byNum = new Map(hosts.map((h) => [ipToNum(h.ip), h]));
  const bases = [...new Set(hosts.map((h) => ipToNum(h.ip) - (ipToNum(h.ip) % 256)))].sort(
    (a, b) => a - b,
  );
  return bases.map((base) => ({
    base: numToIp(base),
    cells: Array.from({ length: 256 }, (_, i) => {
      const h = byNum.get(base + i);
      const kind: CellKind = !h
        ? 'outside'
        : h.own
          ? 'own'
          : h.gateway
            ? 'gateway'
            : h.alive
              ? 'used'
              : 'free';
      return { ip: numToIp(base + i), kind, ...(h ? { host: h } : {}) };
    }),
  }));
}

export const CSV_HEADER = ['ip', 'status', 'hostname', 'mac', 'vendor', 'openPorts'] as const;

export function csvRows(hosts: readonly ScanHost[], status: (h: ScanHost) => string): string[][] {
  return [...hosts]
    .sort((a, b) => ipToNum(a.ip) - ipToNum(b.ip))
    .map((h) => [
      h.ip,
      status(h),
      h.hostname ?? '',
      h.mac ?? '',
      h.vendor ?? '',
      h.openPorts.join(' '),
    ]);
}
