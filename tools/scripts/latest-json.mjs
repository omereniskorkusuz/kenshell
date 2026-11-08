#!/usr/bin/env node
/**
 * Builds the updater manifest `latest.json` (P5-T10, ADR-021) from signed release
 * bundles:
 *   node tools/scripts/latest-json.mjs <version> <base-url> <dir> > latest.json
 * <dir> holds the CI artifacts: the macOS `.app.tar.gz` (universal) and the Windows
 * NSIS `.exe`, each with its `.sig` (written by `tauri build` when
 * TAURI_SIGNING_PRIVATE_KEY is set). <base-url> is where those files will be
 * downloadable, e.g. https://example.com/kenshell/v0.2.0.
 */
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { basename, join } from 'node:path';
import { fileURLToPath } from 'node:url';

function walk(dir) {
  return readdirSync(dir).flatMap((n) => {
    const p = join(dir, n);
    return statSync(p).isDirectory() ? walk(p) : [p];
  });
}

/** Pure part: platform entries from file names and their signatures. */
export function manifest(version, baseUrl, files, readSig, notes = '', pubDate = null) {
  const base = baseUrl.replace(/\/+$/, '');
  const platforms = {};
  const add = (keys, file) => {
    const sig = files.includes(`${file}.sig`) ? readSig(`${file}.sig`).trim() : null;
    if (!sig) return;
    for (const k of keys)
      platforms[k] = { signature: sig, url: `${base}/${encodeURIComponent(basename(file))}` };
  };
  for (const f of files) {
    if (f.endsWith('.app.tar.gz')) add(['darwin-aarch64', 'darwin-x86_64'], f);
    else if (/-setup\.exe$/.test(f)) add(['windows-x86_64'], f);
    else if (f.endsWith('.AppImage')) add(['linux-x86_64'], f);
  }
  if (Object.keys(platforms).length === 0) throw new Error('no signed bundles found');
  return { version, notes, pub_date: pubDate ?? new Date().toISOString(), platforms };
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const [version, baseUrl, dir] = process.argv.slice(2);
  if (!version || !baseUrl || !dir) {
    console.error('usage: latest-json.mjs <version> <base-url> <dir>');
    process.exit(2);
  }
  const files = walk(dir);
  const out = manifest(version.replace(/^v/, ''), baseUrl, files, (p) => readFileSync(p, 'utf8'));
  process.stdout.write(`${JSON.stringify(out, null, 2)}\n`);
}
