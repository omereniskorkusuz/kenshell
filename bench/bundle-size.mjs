#!/usr/bin/env node
// Measures the built frontend (dist/) and fails if JS gzip exceeds the budget.
// Usage: npm run build && node bench/bundle-size.mjs [--json]
import { readFileSync, readdirSync, statSync } from 'node:fs';
import { join, extname } from 'node:path';
import { gzipSync } from 'node:zlib';

const root = new URL('..', import.meta.url).pathname;
const budget = JSON.parse(readFileSync(join(root, 'bench/budget.json'), 'utf8'));
const dist = join(root, 'dist');

function walk(dir) {
  return readdirSync(dir).flatMap((f) => {
    const p = join(dir, f);
    return statSync(p).isDirectory() ? walk(p) : [p];
  });
}

let files;
try {
  files = walk(dist);
} catch {
  console.error('dist/ not found — run `npm run build` first');
  process.exit(2);
}

const sum = { js: { raw: 0, gz: 0 }, css: { raw: 0, gz: 0 }, other: { raw: 0, gz: 0 } };
const rows = [];
for (const f of files) {
  const buf = readFileSync(f);
  const gz = gzipSync(buf, { level: 9 }).length;
  const ext = extname(f).slice(1);
  const kind = ext === 'js' ? 'js' : ext === 'css' ? 'css' : 'other';
  sum[kind].raw += buf.length;
  sum[kind].gz += gz;
  rows.push({ file: f.slice(dist.length + 1), raw: buf.length, gz });
}

const kb = (n) => (n / 1024).toFixed(1);
const result = {
  jsGzipKB: +kb(sum.js.gz),
  cssGzipKB: +kb(sum.css.gz),
  totalGzipKB: +kb(sum.js.gz + sum.css.gz + sum.other.gz),
  budgetJsGzipKB: budget.jsGzipKB,
  ok: sum.js.gz / 1024 <= budget.jsGzipKB,
  files: rows.sort((a, b) => b.gz - a.gz),
};

if (process.argv.includes('--json')) {
  console.log(JSON.stringify(result, null, 2));
} else {
  for (const r of result.files) console.log(`${kb(r.gz).padStart(8)} KB gz  ${r.file}`);
  console.log(
    `JS ${result.jsGzipKB} KB gz (budget ${budget.jsGzipKB}) · CSS ${result.cssGzipKB} KB gz`,
  );
  console.log(result.ok ? 'bundle: OK' : 'bundle: OVER BUDGET');
}
process.exit(result.ok ? 0 : 1);
