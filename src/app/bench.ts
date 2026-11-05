/**
 * Benchmark hook (runs only when the backend was started with KENSHELL_BENCH and
 * KENSHELL_BENCH_OPEN): opens N quick-connect tabs and shows each once so every terminal
 * renders its scrollback, then returns to the first. Used by `bench/measure-app.mjs --sessions`.
 */
import { DEFAULT_PORTS, DEFAULT_SSH, type BenchOpen } from '../lib/ipc';
import { parseQuickConnect } from '../lib/quickConnect';
import { useTabs } from '../stores/tabs';

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

export async function openBenchTabs(bench: BenchOpen): Promise<void> {
  const intent = parseQuickConnect(bench.target, []);
  if (intent.kind !== 'network') return;
  const keys: string[] = [];
  for (let i = 0; i < bench.count; i++) {
    keys.push(
      useTabs.getState().openTarget(
        {
          type: 'network',
          protocol: intent.protocol,
          host: intent.host,
          port: intent.port ?? DEFAULT_PORTS[intent.protocol],
          username: intent.username,
          ssh: DEFAULT_SSH,
        },
        { name: `bench-${i + 1}` },
      ),
    );
  }
  await sleep(1500);
  if (!bench.visit) return;
  for (const key of keys) {
    useTabs.getState().activate(key);
    await sleep(300);
  }
  if (keys[0]) useTabs.getState().activate(keys[0]);
}
