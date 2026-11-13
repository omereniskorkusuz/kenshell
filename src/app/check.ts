/**
 * Self-check (runs only when the backend was started with KENSHELL_BENCH and
 * KENSHELL_CHECK, by `bench/check-topology.mjs`): plays a scenario in the real app against
 * the device simulator and reports what ended up on screen. The run uses a throw-away
 * data directory and in-memory secrets; nothing of the user's is read or written.
 *
 * Scenario: open the given sessions and log in, add an SNMP host, switch the flow
 * collector on, open the topology map, wait until every source has answered, then read
 * the rendered map (cards, port labels) and the sidebar's Recent list from the DOM.
 */
import { DEFAULT_PORTS, DEFAULT_SSH, api, checkReport } from '../lib/ipc';
import { parseQuickConnect } from '../lib/quickConnect';
import { useSettings } from '../stores/settings';
import { getTerminal } from '../terminal/registry';
import { useTabs } from '../stores/tabs';
import { useTopology } from '../stores/topology';
import { useUi } from '../stores/ui';

interface Scenario {
  sessions: { target: string; login: string[] }[];
  snmp?: { host: string; community: string };
  flow?: { sflow: number; netflow: number; expectHosts: number };
  timeoutMs?: number;
}

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

async function until(what: string, ok: () => boolean, timeoutMs: number): Promise<void> {
  const start = Date.now();
  while (!ok()) {
    if (Date.now() - start > timeoutMs) throw new Error(`timed out: ${what}`);
    await sleep(150);
  }
}

const tab = (key: string) => useTabs.getState().tabs.find((x) => x.key === key);
const texts = (selector: string): string[] =>
  [...document.querySelectorAll<HTMLElement>(selector)].map((e) => e.innerText.trim());

export async function runCheck(raw: string): Promise<void> {
  const report: Record<string, unknown> = { ok: false };
  try {
    const spec = JSON.parse(raw) as Scenario;
    const timeout = spec.timeoutMs ?? 60_000;

    // 1. Sessions: connect, then answer the login prompts.
    const keys: string[] = [];
    for (const s of spec.sessions) {
      const intent = parseQuickConnect(s.target, []);
      if (intent.kind !== 'network') throw new Error(`not a network target: ${s.target}`);
      keys.push(
        useTabs.getState().openTarget({
          type: 'network',
          protocol: intent.protocol,
          host: intent.host,
          port: intent.port ?? DEFAULT_PORTS[intent.protocol],
          username: intent.username,
          ssh: DEFAULT_SSH,
        }),
      );
    }
    for (const [i, key] of keys.entries()) {
      await until(`session ${i + 1} connected`, () => tab(key)?.state === 'connected', timeout);
      await sleep(600);
      for (const line of spec.sessions[i]?.login ?? []) {
        useTabs.getState().write(key, `${line}\r`);
        await sleep(600);
      }
      // The mode stripe follows the prompt: a known mode means we are logged in.
      await until(
        `session ${i + 1} at a prompt`,
        () => (tab(key)?.mode ?? 'unknown') !== 'unknown',
        timeout,
      );
    }

    // 2. Other sources.
    if (spec.snmp) {
      await api.snmpCommunitySet(spec.snmp.community);
      useSettings.getState().set('topology.snmpHosts', [spec.snmp.host]);
    }
    if (spec.flow) {
      useSettings.getState().set('topology.sflowPort', spec.flow.sflow);
      useSettings.getState().set('topology.netflowPort', spec.flow.netflow);
      useSettings.getState().set('topology.flowEnabled', true);
    }

    // 3. The map, like the user opens it.
    useUi.getState().openTopology();
    await until(
      'map on screen',
      () => document.querySelector('[data-testid="topology-map"]') !== null,
      timeout,
    );
    const topo = () => useTopology.getState();
    await until(
      'every session read',
      () => keys.every((k) => topo().byTab[k]?.discovery || topo().byTab[k]?.error),
      timeout,
    );
    if (spec.snmp) {
      const host = spec.snmp.host;
      await until(
        'SNMP host read',
        () => Boolean(topo().snmp[host]?.device || topo().snmp[host]?.error),
        timeout,
      );
    }
    if (spec.flow) {
      const want = spec.flow.expectHosts;
      await until(
        'flow exports seen',
        () => {
          void topo().pollFlow();
          return (topo().flow?.hosts.length ?? 0) >= want;
        },
        timeout,
      );
    }
    // Let React draw the final state.
    await sleep(800);

    // 4. What is on screen.
    report.sessions = keys.map((k) => {
      const x = tab(k);
      return {
        name: x?.name,
        state: x?.state,
        mode: x?.mode,
        hostname: x?.device.hostname,
        family: x?.device.vendor?.family ?? null,
        error: topo().byTab[k]?.error ?? null,
      };
    });
    report.tabs = texts('[role="tab"]');
    report.recent = texts('section[aria-labelledby="recent-heading"] .tree-row .mono');
    report.status = texts('[data-testid="topology-map"] [role="status"]')[0] ?? '';
    report.cards = [...document.querySelectorAll<HTMLElement>('.topo-card')].map((e) => ({
      kind: e.dataset.kind,
      source: e.dataset.source ?? null,
      gateway: e.dataset.gateway !== undefined,
      top: Number.parseFloat(e.style.top || '0'),
      lines: e.innerText
        .split('\n')
        .map((l) => l.trim())
        .filter(Boolean),
    }));
    report.ports = texts('.topo-port');
    report.discoverySources = keys.map((k) => topo().byTab[k]?.discovery?.sources ?? []);
    report.gateways = keys.map((k) => topo().byTab[k]?.discovery?.gateway ?? null);
    report.snmp = spec.snmp
      ? {
          name: topo().snmp[spec.snmp.host]?.device?.name ?? null,
          error: topo().snmp[spec.snmp.host]?.error ?? null,
          communityStored: topo().community,
        }
      : null;
    report.flow = topo().flow
      ? {
          running: topo().flow?.running,
          sflowDatagrams: topo().flow?.sflowDatagrams,
          netflowDatagrams: topo().flow?.netflowDatagrams,
          hosts: topo().flow?.hosts.length,
          error: topo().flowError,
        }
      : { error: topo().flowError };
    // Hidden commands must not have reached any terminal: look at each one's scrollback
    // (this leaves the map, so it comes last).
    const terminals: Record<string, unknown>[] = [];
    for (const key of keys) {
      useTabs.getState().activate(key);
      await sleep(700);
      const text = getTerminal(key)?.exportText() ?? null;
      terminals.push({
        chars: text?.length ?? null,
        prompt: text === null ? null : text.trimEnd().split('\n').pop(),
        leak: text === null ? null : /lldp|mac.address|show ip arp|show arp|Chassis/i.test(text),
      });
    }
    report.terminals = terminals;
    report.ok = true;
  } catch (e) {
    report.error = e instanceof Error ? e.message : String(e);
  }
  await checkReport(JSON.stringify(report));
}
