#!/usr/bin/env node
// Launches the release app, measures startup time, idle memory (app + WebView processes)
// and idle CPU, plus installer/bundle size. Compares against bench/budget.json.
//
// Usage: node bench/measure-app.mjs [--bin <path>] [--runs 3] [--idle 5] [--json]
// macOS: memory = phys_footprint (Activity Monitor "Memory") via `footprint`, summed over the
//        app process and the WebKit XPC processes it spawned.
// Windows: memory = private bytes of the app and its msedgewebview2 descendants.
import { spawn, execFileSync } from 'node:child_process';
import { existsSync, readdirSync, readFileSync, statSync, writeFileSync, mkdirSync } from 'node:fs';
import { join } from 'node:path';

const root = new URL('..', import.meta.url).pathname.replace(/^\/([A-Za-z]:)/, '$1');
const budget = JSON.parse(readFileSync(join(root, 'bench/budget.json'), 'utf8'));
const arg = (name, def) => {
  const i = process.argv.indexOf(`--${name}`);
  return i > 0 ? process.argv[i + 1] : def;
};
const isMac = process.platform === 'darwin';
const isWin = process.platform === 'win32';
const runs = Number(arg('runs', 3));
const idleSec = Number(arg('idle', 5));
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const bundleDir = join(root, 'target/release/bundle');

function defaultBin() {
  if (isMac) return join(bundleDir, 'macos/KenShell.app/Contents/MacOS/kenshell');
  if (isWin) return join(root, 'target/release/kenshell.exe');
  return join(root, 'target/release/kenshell');
}

function dirSize(p) {
  const s = statSync(p);
  if (!s.isDirectory()) return s.size;
  return readdirSync(p).reduce((acc, f) => acc + dirSize(join(p, f)), 0);
}

function installerSizes() {
  const out = {};
  for (const kind of ['dmg', 'macos', 'nsis', 'msi', 'appimage', 'deb']) {
    const d = join(bundleDir, kind);
    if (!existsSync(d)) continue;
    for (const f of readdirSync(d)) {
      if (/\.(dmg|app|exe|msi|AppImage|deb)$/.test(f))
        out[`${kind}/${f}`] = +(dirSize(join(d, f)) / 1048576).toFixed(2);
    }
  }
  return out;
}

// ---------- process helpers ----------
function webkitPidsMac() {
  const ps = execFileSync('ps', ['-axo', 'pid=,comm='], { encoding: 'utf8' });
  return new Set(
    ps
      .split('\n')
      .filter((l) => /com\.apple\.WebKit\.(WebContent|Networking|GPU)/.test(l))
      .map((l) => Number(l.trim().split(/\s+/)[0])),
  );
}

function footprintBytesMac(pids) {
  const out = execFileSync(
    'footprint',
    ['-f', 'bytes', ...pids.flatMap((p) => ['-p', String(p)])],
    {
      encoding: 'utf8',
    },
  );
  const per = [...out.matchAll(/\[(\d+)\]:.*?Footprint:\s*(\d+)\s*B/g)].map((m) => ({
    pid: Number(m[1]),
    bytes: Number(m[2]),
  }));
  return per;
}

function cpuSecondsMac(pids) {
  const out = execFileSync('ps', ['-o', 'time=', '-p', pids.join(',')], { encoding: 'utf8' });
  return out
    .trim()
    .split('\n')
    .reduce((acc, t) => {
      const parts = t.trim().split(':').map(Number);
      return acc + parts.reduce((a, v) => a * 60 + v, 0);
    }, 0);
}

function winProcs() {
  const ps =
    'Get-CimInstance Win32_Process | Select-Object ProcessId,ParentProcessId,Name,PrivatePageCount,KernelModeTime,UserModeTime | ConvertTo-Json -Compress';
  return JSON.parse(
    execFileSync('powershell', ['-NoProfile', '-Command', ps], { encoding: 'utf8' }),
  );
}

function winTree(rootPid) {
  const all = winProcs();
  const tree = new Set([rootPid]);
  let grew = true;
  while (grew) {
    grew = false;
    for (const p of all)
      if (tree.has(p.ParentProcessId) && !tree.has(p.ProcessId)) {
        tree.add(p.ProcessId);
        grew = true;
      }
  }
  return all.filter((p) => tree.has(p.ProcessId));
}

// ---------- one run ----------
async function runOnce(bin) {
  const before = isMac ? webkitPidsMac() : new Set();
  const t0 = performance.now();
  const child = spawn(bin, [], {
    env: { ...process.env, KENSHELL_BENCH: '1' },
    stdio: ['ignore', 'pipe', 'inherit'],
  });
  const ready = await new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error('app did not report ready within 20s')), 20000);
    let buf = '';
    child.stdout.on('data', (d) => {
      buf += d;
      const m = buf.match(/KENSHELL_READY_MS=(\d+)/);
      if (m) {
        clearTimeout(timer);
        resolve({ wallMs: Math.round(performance.now() - t0), inProcessMs: Number(m[1]) });
      }
    });
    child.on('exit', (c) => reject(new Error(`app exited early (${c})`)));
  });

  await sleep(idleSec * 1000);
  let memMB, cpuPct, procs;
  if (isMac) {
    const pids = [child.pid, ...[...webkitPidsMac()].filter((p) => !before.has(p))];
    const fp = footprintBytesMac(pids);
    memMB = fp.reduce((a, p) => a + p.bytes, 0) / 1048576;
    const c0 = cpuSecondsMac(pids);
    await sleep(3000);
    cpuPct = ((cpuSecondsMac(pids) - c0) / 3) * 100;
    procs = fp.map((p) => ({ pid: p.pid, mb: +(p.bytes / 1048576).toFixed(1) }));
  } else if (isWin) {
    const t1 = winTree(child.pid);
    memMB = t1.reduce((a, p) => a + Number(p.PrivatePageCount), 0) / 1048576;
    const cpu = (ps) =>
      ps.reduce((a, p) => a + Number(p.KernelModeTime) + Number(p.UserModeTime), 0) / 1e7;
    const c0 = cpu(t1);
    await sleep(3000);
    cpuPct = ((cpu(winTree(child.pid)) - c0) / 3) * 100;
    procs = t1.map((p) => ({
      pid: p.ProcessId,
      name: p.Name,
      mb: +(Number(p.PrivatePageCount) / 1048576).toFixed(1),
    }));
  }
  child.kill();
  await sleep(1000);
  return { ...ready, idleRamMB: +memMB.toFixed(1), idleCpuPercent: +cpuPct.toFixed(2), procs };
}

if (process.argv.includes('--sizes-only')) {
  // CI mode: no GUI launch, only check installer size against the budget.
  const sizes = installerSizes();
  const limit = isMac ? budget.installerMB.macos : budget.installerMB.windows;
  const inst = Object.entries(sizes).filter(([k]) => /^(dmg|nsis|msi)\//.test(k));
  console.log(JSON.stringify({ sizes, limitMB: limit }, null, 2));
  process.exit(inst.every(([, mb]) => mb <= limit) ? 0 : 1);
}

const bin = arg('bin', defaultBin());
if (!existsSync(bin)) {
  console.error(`binary not found: ${bin} — run \`npm run app:build\` first`);
  process.exit(2);
}

const results = [];
for (let i = 0; i < runs; i++) results.push(await runOnce(bin));
const median = (xs) => [...xs].sort((a, b) => a - b)[Math.floor(xs.length / 2)];
const sizes = installerSizes();
const installer = Object.entries(sizes).find(([k]) => k.startsWith(isMac ? 'dmg/' : 'nsis/'));
const summary = {
  platform: `${process.platform}-${process.arch}`,
  firstStartMs: results[0].wallMs,
  medianStartMs: median(results.map((r) => r.wallMs)),
  idleRamMB: median(results.map((r) => r.idleRamMB)),
  idleCpuPercent: median(results.map((r) => r.idleCpuPercent)),
  installerMB: installer ? installer[1] : null,
  sizes,
  runs: results,
};
const limitInstaller = isMac ? budget.installerMB.macos : budget.installerMB.windows;
summary.ok = {
  start: summary.medianStartMs <= budget.coldStartMs,
  ram: summary.idleRamMB <= budget.idleRamMB,
  cpu: summary.idleCpuPercent <= budget.idleCpuPercent,
  installer: summary.installerMB === null || summary.installerMB <= limitInstaller,
};

mkdirSync(join(root, 'bench/results'), { recursive: true });
writeFileSync(
  join(root, `bench/results/app-${process.platform}.local.json`),
  JSON.stringify(summary, null, 2),
);
if (process.argv.includes('--json')) console.log(JSON.stringify(summary, null, 2));
else {
  console.log(
    `start: first ${summary.firstStartMs} ms · median ${summary.medianStartMs} ms (budget ${budget.coldStartMs})`,
  );
  console.log(
    `idle RAM: ${summary.idleRamMB} MB (budget ${budget.idleRamMB}) · idle CPU: ${summary.idleCpuPercent}%`,
  );
  console.log(
    `installer: ${summary.installerMB ?? '—'} MB (budget ${limitInstaller}) · sizes ${JSON.stringify(sizes)}`,
  );
  console.log(`processes (last run): ${JSON.stringify(results.at(-1).procs)}`);
  console.log(
    Object.values(summary.ok).every(Boolean)
      ? 'app: OK'
      : `app: OVER BUDGET ${JSON.stringify(summary.ok)}`,
  );
}
process.exit(Object.values(summary.ok).every(Boolean) ? 0 : 1);
