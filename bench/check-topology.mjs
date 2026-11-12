#!/usr/bin/env node
// End-to-end check of the topology map in the REAL app (release binary, real backend,
// real webview), without needing to see the screen:
//
//   cargo build -p device-simulator && npm run app:build -- --bundles app
//   node bench/check-topology.mjs
//
// Starts the device simulator three times (two CLI devices that are each other's
// neighbours, and an SNMP agent for a third), launches the app in self-check mode
// (KENSHELL_BENCH + KENSHELL_CHECK: throw-away data directory, in-memory secrets), sends
// sFlow / NetFlow exports to it, and asserts on what the app reports it drew: the cards,
// the port labels, the sources, the Recent list. Exit code 0 = all good.
import { spawn } from 'node:child_process';
import { existsSync, mkdtempSync, rmSync } from 'node:fs';
import { createServer } from 'node:net';
import { createSocket } from 'node:dgram';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const isWin = process.platform === 'win32';
const exe = isWin ? '.exe' : '';
const arg = (name, fallback) => {
  const i = process.argv.indexOf(`--${name}`);
  return i > 0 ? process.argv[i + 1] : fallback;
};
const appBin = arg(
  'app',
  process.platform === 'darwin'
    ? join(root, 'target/release/bundle/macos/KenShell.app/Contents/MacOS/kenshell')
    : join(root, `target/release/kenshell${exe}`),
);
const sim = join(root, `target/debug/device-simulator${exe}`);
for (const [what, path] of [
  ['app binary', appBin],
  ['device simulator', sim],
]) {
  if (!existsSync(path)) {
    console.error(`${what} not found: ${path}`);
    process.exit(2);
  }
}

const freeTcp = () =>
  new Promise((resolve) => {
    const s = createServer().listen(0, '127.0.0.1', () => {
      const { port } = s.address();
      s.close(() => resolve(port));
    });
  });
const freeUdp = () =>
  new Promise((resolve) => {
    const s = createSocket('udp4').bind(0, '127.0.0.1', () => {
      const { port } = s.address();
      s.close(() => resolve(port));
    });
  });
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

const [corePort, accessPort] = [await freeTcp(), await freeTcp()];
const [snmpPort, sflowPort, netflowPort] = [await freeUdp(), await freeUdp(), await freeUdp()];
const dataDir = mkdtempSync(join(tmpdir(), 'kenshell-check-'));
const children = [];
const run = (bin, args, opts = {}) => {
  const c = spawn(bin, args, { stdio: 'ignore', ...opts });
  children.push(c);
  return c;
};
const cleanup = () => {
  for (const c of children) c.kill();
  rmSync(dataDir, { recursive: true, force: true });
};

run(sim, ['--profile', 'cisco-ios', '--port', String(corePort)]);
run(sim, ['--profile', 'ruijie-rgos', '--port', String(accessPort)]);
run(sim, ['--snmp', String(snmpPort), '--community', 'check-ro']);
await sleep(800);

const scenario = {
  sessions: [
    { target: `telnet 127.0.0.1 ${corePort}`, login: ['admin', 'admin'] },
    { target: `telnet 127.0.0.1 ${accessPort}`, login: ['admin', 'admin'] },
  ],
  snmp: { host: `127.0.0.1:${snmpPort}`, community: 'check-ro' },
  flow: { sflow: sflowPort, netflow: netflowPort, expectHosts: 3 },
  timeoutMs: 60000,
};
const app = run(appBin, [], {
  stdio: ['ignore', 'pipe', 'inherit'],
  env: {
    ...process.env,
    KENSHELL_BENCH: '1',
    KENSHELL_CHECK: JSON.stringify(scenario),
    KENSHELL_CHECK_DATA_DIR: dataDir,
    LANG: 'en_US.UTF-8',
  },
});
// The exports keep coming while the app starts, logs in and opens the map.
run(sim, [
  '--sflow-to',
  `127.0.0.1:${sflowPort}`,
  '--netflow-to',
  `127.0.0.1:${netflowPort}`,
  '--flow-rounds',
  '120',
]);

let report;
try {
  report = await new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error('no report within 150 s')), 150_000);
    let buf = '';
    app.stdout.on('data', (d) => {
      buf += d;
      const m = buf.match(/KENSHELL_CHECK_RESULT=(.*)\r?\n/);
      if (m) {
        clearTimeout(timer);
        resolve(JSON.parse(m[1]));
      }
    });
    app.on('exit', (c) => reject(new Error(`app exited early (${c})`)));
  });
} catch (e) {
  cleanup();
  console.error(`check failed: ${e.message}`);
  process.exit(1);
}
cleanup();

// ---------- assertions ----------
const problems = [];
const expect = (ok, what) => {
  console.log(`${ok ? '  ok ' : ' FAIL'}  ${what}`);
  if (!ok) problems.push(what);
};
const cards = report.cards ?? [];
const card = (name) => cards.find((c) => c.lines[0] === name);
const ports = report.ports ?? [];

if (process.argv.includes('--print')) console.log(JSON.stringify(report, null, 2));
expect(report.ok === true, `scenario ran to the end${report.error ? ` (${report.error})` : ''}`);
expect(
  (report.sessions ?? []).length === 2 &&
    report.sessions.every((s) => s.state === 'connected' && s.mode !== 'unknown' && !s.error),
  'both sessions connected, logged in and read without error',
);
expect(
  report.sessions?.[0]?.hostname === 'core-sw-01' &&
    report.sessions?.[1]?.hostname === 'access-sw-02',
  'host names taken from the prompts',
);
expect((report.tabs ?? []).length === 3, 'tab strip: the map tab and two session tabs');
expect(
  (report.recent ?? []).length === 2 &&
    report.recent.some((r) => r.includes(String(corePort))) &&
    report.recent.some((r) => r.includes(String(accessPort))),
  'both quick connections are in the Recent list',
);
expect(card('core-sw-01')?.source === 'session', 'core-sw-01 is one card, an open session');
expect(card('access-sw-02')?.source === 'session', 'access-sw-02 is one card, an open session');
expect(
  cards.filter((c) => c.lines[0] === 'core-sw-01').length === 1 &&
    cards.filter((c) => c.lines[0] === 'access-sw-02').length === 1 &&
    cards.filter((c) => c.lines[0].startsWith('dist-sw-01')).length === 1,
  'no device is drawn twice',
);
expect(
  card('dist-sw-01')?.source === 'snmp' &&
    report.snmp?.name === 'dist-sw-01' &&
    !report.snmp?.error,
  'dist-sw-01 (a neighbour of the core) is read over SNMP and tagged so',
);
expect(report.snmp?.communityStored === true, 'the community is stored (in memory in this run)');
for (const ap of ['AP-FLOOR1', 'ap-lobby', 'ap-roof']) {
  expect(card(ap)?.kind === 'ap', `${ap} is drawn as an access point`);
}
expect(
  ports.includes('Te1/0/5 → Te0/49') || ports.includes('Te0/49 → Te1/0/5'),
  'link core Te1/0/5 ↔ access Te0/49 labelled with both ports',
);
expect(
  ports.includes('Te1/0/6 ↔ Te0/50') ||
    ports.includes('Te0/50 ↔ Te1/0/6') ||
    ports.includes('Te1/0/6 → Te0/50') ||
    ports.includes('Te0/50 → Te1/0/6'),
  'the second, parallel uplink (Te1/0/6 ↔ Te0/50) is its own link',
);
expect(
  ports.filter((p) => p.includes('Te1/1/1') && p.includes('Te1/0/1')).length === 1,
  'link core Te1/1/1 ↔ dist Te1/0/1 is one line though a session and SNMP both report it',
);
expect(
  cards.some((c) => c.kind === 'host' && c.lines[0] === '10.10.0.77') &&
    ports.filter((p) => p === 'Gi1/0/10').length === 2,
  'an end device from the SNMP forwarding table hangs off dist-sw-01 Gi1/0/10',
);
expect(
  cards.some((c) => c.kind === 'host' && c.lines[0] === '10.10.0.31'),
  'an end device from the MAC table carries its ARP address (10.10.0.31)',
);
expect(
  cards.some((c) => c.kind === 'group' && /6/.test(c.lines[0])),
  'a port with six learned addresses is one "6 devices" card',
);
expect(
  report.flow?.running === true &&
    report.flow.sflowDatagrams > 0 &&
    report.flow.netflowDatagrams > 0 &&
    !report.flow.error,
  'the collector received sFlow and NetFlow exports',
);
expect(
  cards.some((c) => c.kind === 'host' && c.lines[0] === '10.20.0.107') && ports.includes('if7'),
  'an sFlow sighting hangs off access-sw-02 (found by its advertised address) on if7',
);
expect(
  cards.some((c) => c.kind === 'host' && c.lines[0] === '10.30.0.9'),
  'a NetFlow sighting is on the map',
);
expect(
  (report.terminals ?? []).length === 2 && report.terminals.every((t) => t.leak === false),
  'no hidden command or its output reached a terminal',
);
console.log(
  `\n${cards.length} cards, ${ports.length} port labels · status: ${report.status}\n` +
    (problems.length ? `topology check: ${problems.length} FAILED` : 'topology check: OK'),
);
process.exit(problems.length ? 1 : 0);
