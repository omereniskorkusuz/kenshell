/**
 * Network tools (P5-T04): IPv4 subnet calculator and MAC address formats. Pure
 * functions; the OUI vendor lookup goes through the backend's embedded table.
 */
import { ipToNum, numToIp } from './netscan';

export interface Subnet {
  address: string;
  prefix: number;
  mask: string;
  wildcard: string;
  network: string;
  broadcast: string;
  firstHost: string;
  lastHost: string;
  /** Usable hosts (/31 → 2 point-to-point, /32 → 1). */
  hosts: number;
  total: number;
  /** Address scope: private (RFC1918), loopback, link-local, CGNAT, multicast, public. */
  scope: 'private' | 'loopback' | 'linkLocal' | 'cgnat' | 'multicast' | 'reserved' | 'public';
  binaryMask: string;
}

const IPV4 = /^(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.(\d{1,3})$/;

export function isIpv4(s: string): boolean {
  const m = IPV4.exec(s.trim());
  return !!m && m.slice(1).every((o) => Number(o) <= 255);
}

/** A dotted mask's prefix length, or null when the bits are not contiguous. */
export function maskToPrefix(mask: string): number | null {
  if (!isIpv4(mask)) return null;
  const n = ipToNum(mask);
  const inv = ~n >>> 0;
  // Contiguous ones: the inverse + 1 is a power of two.
  if ((inv & (inv + 1)) !== 0) return null;
  return 32 - Math.log2(inv + 1);
}

export function prefixToMask(prefix: number): string {
  return numToIp(prefix === 0 ? 0 : (0xffffffff << (32 - prefix)) >>> 0);
}

function scopeOf(n: number): Subnet['scope'] {
  const a = n >>> 24;
  const b = (n >>> 16) & 0xff;
  if (a === 10 || (a === 172 && b >= 16 && b <= 31) || (a === 192 && b === 168)) return 'private';
  if (a === 127) return 'loopback';
  if (a === 169 && b === 254) return 'linkLocal';
  if (a === 100 && b >= 64 && b <= 127) return 'cgnat';
  if (a >= 224 && a <= 239) return 'multicast';
  if (a === 0 || a >= 240) return 'reserved';
  return 'public';
}

/**
 * Parses `10.1.2.3/24`, `10.1.2.3 255.255.255.0`, `10.1.2.3/255.255.255.0`, a Cisco
 * wildcard (`10.1.2.0 0.0.0.255`) or a bare address (/32). Null when invalid.
 */
export function subnet(input: string): Subnet | null {
  const parts = input.trim().split(/[\s/]+/);
  const [addr, second] = parts;
  if (!addr || parts.length > 2 || !isIpv4(addr)) return null;
  let prefix: number | null;
  if (second === undefined) prefix = 32;
  else if (/^\d{1,2}$/.test(second)) prefix = Number(second) <= 32 ? Number(second) : null;
  else {
    prefix = maskToPrefix(second);
    // A wildcard is an inverted mask (0.0.0.255); 0.0.0.0 stays a /0 mask.
    if (prefix === null && isIpv4(second)) {
      const inverted = numToIp(~ipToNum(second) >>> 0);
      prefix = maskToPrefix(inverted);
    }
  }
  if (prefix === null) return null;
  const n = ipToNum(addr);
  const maskNum = prefix === 0 ? 0 : (0xffffffff << (32 - prefix)) >>> 0;
  const network = (n & maskNum) >>> 0;
  const total = 2 ** (32 - prefix);
  const broadcast = (network + total - 1) >>> 0;
  const [first, last, hosts] =
    prefix >= 31 ? [network, broadcast, total] : [network + 1, broadcast - 1, total - 2];
  const mask = numToIp(maskNum);
  return {
    address: addr,
    prefix,
    mask,
    wildcard: numToIp(~maskNum >>> 0),
    network: numToIp(network),
    broadcast: numToIp(broadcast),
    firstHost: numToIp(first),
    lastHost: numToIp(last),
    hosts,
    total,
    scope: scopeOf(n),
    binaryMask: mask
      .split('.')
      .map((o) => Number(o).toString(2).padStart(8, '0'))
      .join('.'),
  };
}

export interface MacFormats {
  cisco: string;
  colon: string;
  dash: string;
  bare: string;
  /** I/G bit: group (multicast/broadcast) address. */
  multicast: boolean;
  /** U/L bit: locally administered (random / virtual). */
  local: boolean;
}

/** All common spellings of a MAC (any separator, any case). Null unless 12 hex digits. */
export function macFormats(input: string): MacFormats | null {
  const hex = input
    .trim()
    .toLowerCase()
    .replace(/[\s:.-]/g, '');
  if (!/^[0-9a-f]{12}$/.test(hex)) return null;
  const pairs = hex.match(/../g) ?? [];
  const first = parseInt(hex.slice(0, 2), 16);
  return {
    cisco: `${hex.slice(0, 4)}.${hex.slice(4, 8)}.${hex.slice(8)}`,
    colon: pairs.join(':'),
    dash: pairs.join('-').toUpperCase(),
    bare: hex,
    multicast: (first & 1) === 1,
    local: (first & 2) === 2,
  };
}
