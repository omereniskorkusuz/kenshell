// README screenshots: the browser demo (mock data, no real network) in headless Chrome,
// driven over the DevTools protocol, at 2x. Start the demo first (`npm run dev`), then:
//   node tools/scripts/readme-screenshots.mjs docs/assets [home session topology scan]
// Files: screenshot-<scene>.png. CHROME=<path> overrides the Chrome binary.
import { spawn } from 'node:child_process';
import { mkdirSync, mkdtempSync, writeFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

const OUT = process.argv[2] ?? 'docs/assets';
const ONLY = process.argv.slice(3);
const URL = 'http://localhost:1420/';
const CHROME =
  process.env.CHROME ??
  {
    darwin: '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome',
    win32: 'C:/Program Files/Google/Chrome/Application/chrome.exe',
  }[process.platform] ??
  'google-chrome';
const PROFILE = mkdtempSync(join(tmpdir(), 'kenshell-shots-'));
const W = 1440;
const H = 880;
mkdirSync(OUT, { recursive: true });

const chrome = spawn(
  CHROME,
  [
    '--headless=new',
    '--remote-debugging-port=9333',
    `--user-data-dir=${PROFILE}`,
    '--no-first-run', // WebGL does not paint in headless Chrome; without it the terminal uses the DOM renderer.
    '--disable-3d-apis',
    '--disable-webgl',
    '--no-default-browser-check',
    '--hide-scrollbars',
    `--window-size=${W},${H}`,
    'about:blank',
  ],
  { stdio: 'ignore' },
);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

let ws;
for (let i = 0; i < 50 && !ws; i++) {
  await sleep(200);
  try {
    const list = await (await fetch('http://127.0.0.1:9333/json/list')).json();
    const page = list.find((p) => p.type === 'page');
    if (page) ws = new WebSocket(page.webSocketDebuggerUrl);
  } catch {
    // Chrome is still starting.
  }
}
await new Promise((r) => ws.addEventListener('open', r, { once: true }));
let id = 0;
const pending = new Map();
ws.addEventListener('message', (e) => {
  const m = JSON.parse(e.data);
  if (m.id && pending.has(m.id)) {
    pending.get(m.id)(m);
    pending.delete(m.id);
  }
});
const send = (method, params = {}) =>
  new Promise((resolve, reject) => {
    const n = ++id;
    pending.set(n, (m) =>
      m.error ? reject(new Error(`${method}: ${m.error.message}`)) : resolve(m.result),
    );
    ws.send(JSON.stringify({ id: n, method, params }));
  });
const js = async (expr) => {
  const r = await send('Runtime.evaluate', {
    expression: expr,
    awaitPromise: true,
    returnByValue: true,
  });
  if (r.exceptionDetails) throw new Error(`${expr}: ${r.exceptionDetails.exception?.description}`);
  return r.result.value;
};
// Click / double-click the first element whose aria-label, title or text matches.
const click = (text, opts = {}) =>
  js(`(() => {
    const want = ${JSON.stringify(text)};
    const sel = ${JSON.stringify(opts.selector ?? 'button,[role=treeitem],[role=tab],a,summary,label')};
    const els = [...document.querySelectorAll(sel)];
    const el = els.find(e => e.getAttribute('aria-label') === want)
      ?? els.find(e => (e.textContent || '').trim() === want)
      ?? els.find(e => (e.getAttribute('aria-label') || '').startsWith(want))
      ?? els.find(e => (e.textContent || '').trim().startsWith(want));
    if (!el) throw new Error('not found: ' + want);
    ${opts.double ? "el.dispatchEvent(new MouseEvent('dblclick', {bubbles: true}));" : 'el.click();'}
    return true;
  })()`);
const key = async (k) => {
  const code = { Enter: 13, Escape: 27 }[k];
  const base = { key: k, code: k, windowsVirtualKeyCode: code, nativeVirtualKeyCode: code };
  await send('Input.dispatchKeyEvent', {
    type: 'keyDown',
    ...base,
    ...(k === 'Enter' ? { text: '\r' } : {}),
  });
  await send('Input.dispatchKeyEvent', { type: 'keyUp', ...base });
};
const typeLine = async (text) => {
  await js(
    `document.querySelector('.pane-active .xterm-helper-textarea, .xterm-helper-textarea')?.focus()`,
  );
  await send('Input.insertText', { text });
  await key('Enter');
  await sleep(500);
};
const shot = async (name) => {
  await sleep(400);
  const r = await send('Page.captureScreenshot', { format: 'png' });
  writeFileSync(`${OUT}/screenshot-${name}.png`, Buffer.from(r.data, 'base64'));
  console.log('saved', name);
};
const fresh = async () => {
  await send('Page.navigate', { url: URL });
  await sleep(2500);
  await js(`localStorage.clear(); sessionStorage.clear(); true`);
  await send('Page.navigate', { url: URL });
  await sleep(2500);
};

await send('Page.enable');
await send('Runtime.enable');
await send('Emulation.setDeviceMetricsOverride', {
  width: W,
  height: H,
  deviceScaleFactor: 2,
  mobile: false,
});
await send('Emulation.setEmulatedMedia', {
  features: [{ name: 'prefers-color-scheme', value: 'dark' }],
});

const scenes = {
  async home() {
    await fresh();
    await shot('home');
  },
  async session() {
    await fresh();
    await click('core-sw1', { selector: '[role=treeitem]', double: true });
    await sleep(2500);
    await typeLine('enable');
    await typeLine('show ip interface brief');
    await click('Toggle side panel');
    await sleep(500);
    await click('Analyzer', { selector: '[role=tab]' });
    await sleep(800);
    await js(
      `[...document.querySelectorAll('button')].find(b => /last output|Analyze last/i.test(b.textContent))?.click(); true`,
    );
    await sleep(1200);
    await shot('session-analyzer');
  },
  async topology() {
    await fresh();
    for (const s of ['core-sw1', 'access-sw3']) {
      await click(s, { selector: '[role=treeitem]', double: true });
      await sleep(2500);
    }
    await click('Topology');
    await sleep(5000);
    await shot('topology');
  },
  async scan() {
    await fresh();
    await click('Scan my network');
    await sleep(800);
    await click('Scan');
    await sleep(3500);
    await shot('scan-grid');
    await click('Used');
    await sleep(600);
    await shot('scan-list');
  },
};

try {
  for (const [name, run] of Object.entries(scenes)) {
    if (ONLY.length && !ONLY.includes(name)) continue;
    try {
      await run();
    } catch (e) {
      console.log('scene failed', name, e.message);
    }
  }
} finally {
  ws.close();
  const exited = new Promise((r) => chrome.once('exit', r));
  chrome.kill();
  await exited;
  rmSync(PROFILE, { recursive: true, force: true, maxRetries: 5, retryDelay: 200 });
}
