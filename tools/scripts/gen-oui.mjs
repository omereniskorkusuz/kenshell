#!/usr/bin/env node
/**
 * Builds the embedded OUI table (P4-T15) from the IEEE MA-L registry CSV:
 *   node tools/scripts/gen-oui.mjs path/to/oui.csv
 * (oui.csv: https://standards-oui.ieee.org/oui/oui.csv — "Registry,Assignment,
 * Organization Name,Organization Address").
 *
 * Output src-tauri/data/oui.bin = raw deflate of:
 *   "OUI1" | u32 count | count × 3-byte prefix (sorted) | count × u16 name index |
 *   u32 names | names × (u16 byte length, UTF-8)
 * Vendor names are shortened (legal suffixes dropped) to keep the app small.
 */
import { readFileSync, writeFileSync } from 'node:fs';
import { deflateRawSync } from 'node:zlib';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

const here = dirname(fileURLToPath(import.meta.url));
const out = join(here, '../../src-tauri/data/oui.bin');

/** One CSV row (quoted fields with "" escapes). */
export function parseCsvLine(line) {
  const cells = [];
  let cur = '';
  let quoted = false;
  for (let i = 0; i < line.length; i++) {
    const c = line[i];
    if (quoted) {
      if (c === '"' && line[i + 1] === '"') {
        cur += '"';
        i++;
      } else if (c === '"') quoted = false;
      else cur += c;
    } else if (c === '"') quoted = true;
    else if (c === ',') {
      cells.push(cur);
      cur = '';
    } else cur += c;
  }
  cells.push(cur);
  return cells;
}

const SUFFIX =
  /[,.]?\s+(co\.?,?\s*ltd\.?|company,?\s*limited|limited|ltd\.?|inc\.?|incorporated|corporation|corp\.?|llc|l\.l\.c\.|gmbh|ag|s\.?a\.?|s\.?p\.?a\.?|b\.?v\.?|oy|ab|a\/s|pty\.?\s*ltd\.?|pte\.?\s*ltd\.?|co\.?|kg|sas|srl|s\.r\.l\.|technologies|technology|electronics)\.?$/i;

export function shortName(name) {
  let n = name.replace(/\s+/g, ' ').trim();
  for (let i = 0; i < 3; i++) {
    const m = n.replace(SUFFIX, '').trim();
    if (m === n || m.length < 2) break;
    n = m;
  }
  return n.replace(/[,.]+$/, '').slice(0, 60);
}

export function build(csv) {
  const entries = new Map();
  for (const line of csv.split(/\r?\n/).slice(1)) {
    if (!line.trim()) continue;
    const [registry, assignment, org] = parseCsvLine(line);
    if (registry !== 'MA-L' || !/^[0-9A-Fa-f]{6}$/.test(assignment ?? '') || !org) continue;
    entries.set(parseInt(assignment, 16), shortName(org));
  }
  const prefixes = [...entries.keys()].sort((a, b) => a - b);
  const names = [...new Set(entries.values())];
  const index = new Map(names.map((n, i) => [n, i]));
  if (names.length > 65535) throw new Error('too many names');
  const nameBytes = names.map((n) => Buffer.from(n, 'utf8'));
  const size = 4 + 4 + prefixes.length * 5 + 4 + nameBytes.reduce((s, b) => s + 2 + b.length, 0);
  const buf = Buffer.alloc(size);
  let o = 0;
  buf.write('OUI1', o);
  o += 4;
  buf.writeUInt32LE(prefixes.length, o);
  o += 4;
  for (const p of prefixes) {
    buf[o++] = (p >> 16) & 255;
    buf[o++] = (p >> 8) & 255;
    buf[o++] = p & 255;
  }
  for (const p of prefixes) {
    buf.writeUInt16LE(index.get(entries.get(p)), o);
    o += 2;
  }
  buf.writeUInt32LE(names.length, o);
  o += 4;
  for (const b of nameBytes) {
    buf.writeUInt16LE(b.length, o);
    o += 2;
    b.copy(buf, o);
    o += b.length;
  }
  return { bin: deflateRawSync(buf, { level: 9 }), count: prefixes.length, names: names.length };
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  const src = process.argv[2];
  if (!src) {
    console.error('usage: node tools/scripts/gen-oui.mjs path/to/oui.csv');
    process.exit(2);
  }
  const { bin, count, names } = build(readFileSync(src, 'utf8'));
  writeFileSync(out, bin);
  console.log(`oui.bin: ${count} prefixes, ${names} vendors, ${(bin.length / 1024).toFixed(0)} KB`);
}
