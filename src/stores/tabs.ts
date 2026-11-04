/**
 * Open tabs = live sessions. Bytes never pass through this store (they go straight to the
 * tab's TerminalSink); only state-machine events do.
 */
import { create } from 'zustand';
import { t } from '../i18n';
import { describeError } from '../lib/errors';
import { formatBytes } from '../lib/format';
import {
  api,
  DEFAULT_PORTS,
  DEFAULT_SERIAL,
  DEFAULT_SSH,
  liveOpen,
  type ConsolePort,
  type OpenTarget,
  type SerialSettings,
  type SessionEvent,
  type SessionRecord,
} from '../lib/ipc';
import { feedPrompt, promptIntro, startPrompt } from '../lib/prompt';
import {
  applySessionEvent,
  endpointLabel,
  portLabel,
  serialOf,
  tabStatus,
  type LiveTab,
} from '../lib/sessionState';
import { normalizeEnv } from '../lib/tree';
import { createSink, disposeSink, getSink } from '../terminal/sink';
import { useLibrary } from './library';
import { toast } from './toasts';
import { useUi } from './ui';

export interface OpenMeta {
  sessionId?: number | null;
  name?: string;
  envTag?: string | null;
}

interface ClosedTab {
  target: OpenTarget;
  meta: OpenMeta;
}

interface TabsState {
  tabs: LiveTab[];
  activeKey: string | null;
  closed: ClosedTab[];

  openSerial(settings: SerialSettings, meta?: OpenMeta): string;
  /** Opens any target (serial reuses the tab owning the port). */
  openTarget(target: OpenTarget, meta?: OpenMeta): string;
  openSession(session: SessionRecord): void;
  openConsole(port: ConsolePort): void;
  activate(key: string): void;
  next(): void;
  prev(): void;
  selectIndex(index: number): void;
  close(key?: string | null): void;
  reopenClosed(): void;
  reconnect(key?: string | null): void;
  sendBreak(key?: string | null): void;
  changeBaud(key: string, baud: number | null): void;
  write(key: string, data: string): void;
  /** Forgets the stored SSH host key of this tab's host and reconnects. */
  forgetHostKey(key?: string | null): void;
  resize(key: string, cols: number, rows: number): void;
}

/** Last measured terminal grid; used for the initial size of new sessions. */
const lastGrid = { cols: 80, rows: 24 };
const sentGrid = new Map<string, string>();
const resizeTimers = new Map<string, ReturnType<typeof setTimeout>>();
let keySeq = 1;
const MAX_CLOSED = 10;
const encoder = new TextEncoder();
const REOPEN_TOAST_MS = 10_000;

const samePort = (a: string, b: string) =>
  a.localeCompare(b, undefined, { sensitivity: 'accent' }) === 0;

export const useTabs = create<TabsState>()((set, get) => {
  const tab = (key: string) => get().tabs.find((x) => x.key === key);
  const update = (key: string, fn: (tab: LiveTab) => LiveTab) =>
    set((s) => ({ tabs: s.tabs.map((x) => (x.key === key ? fn(x) : x)) }));
  const resolveKey = (key?: string | null) => key ?? get().activeKey;

  const announce = (key: string) => {
    const cur = tab(key);
    if (cur) useUi.getState().announce(`${cur.name}: ${tabStatus(cur, t).text}`);
  };

  let recentReload: ReturnType<typeof setTimeout> | null = null;
  const refreshRecent = () => {
    if (recentReload) clearTimeout(recentReload);
    recentReload = setTimeout(() => void useLibrary.getState().reloadSessions(), 500);
  };

  /** (Re)starts the live session behind a tab. */
  const start = async (key: string) => {
    const cur = tab(key);
    if (!cur) return;
    const generation = cur.generation + 1;
    update(key, (x) => ({
      ...x,
      generation,
      runtimeId: null,
      state: 'connecting',
      error: null,
      autobaud: null,
      link: null,
      netLink: null,
      prompt: null,
      connectedAt: null,
      health: x.health === 'cableReturned' ? x.health : null,
    }));
    sentGrid.delete(key);
    const isCurrent = () => tab(key)?.generation === generation;
    const sink = getSink(key);

    const onData = (bytes: Uint8Array) => {
      if (!isCurrent()) return;
      sink?.push(bytes);
      if (tab(key)?.health === 'noData') update(key, (x) => ({ ...x, health: null }));
    };
    const onEvent = (ev: SessionEvent) => {
      if (!isCurrent()) return;
      const before = tab(key);
      if (!before) return;
      if (ev.type === 'prompt') {
        sink?.push(encoder.encode(promptIntro(ev.request)));
        update(key, (x) => ({ ...x, prompt: startPrompt(ev.request) }));
        announce(key);
        return;
      }
      if (ev.type === 'netLink' && ev.link.hostKey?.firstSeen) {
        useUi
          .getState()
          .flash(t('ssh.hostKeyLearned', { fingerprint: ev.link.hostKey.fingerprint }));
      }
      const after = applySessionEvent(before, ev, Date.now());
      update(key, () => after);
      if (before.state !== after.state || before.health !== after.health) announce(key);
      if (ev.type === 'state' && ev.state === 'connected' && after.sessionId !== null) {
        refreshRecent();
      }
    };

    try {
      const runtimeId = await liveOpen(
        {
          sessionId: cur.sessionId,
          target: cur.target,
          cols: lastGrid.cols,
          rows: lastGrid.rows,
        },
        onData,
        onEvent,
      );
      if (!isCurrent()) {
        void api.liveClose(runtimeId).catch(() => {});
        return;
      }
      update(key, (x) => ({ ...x, runtimeId }));
      sentGrid.set(key, `${lastGrid.cols}x${lastGrid.rows}`);
    } catch (e) {
      if (!isCurrent()) return;
      update(key, (x) => ({ ...x, state: 'disconnected', error: describeErrorAsApp(e) }));
      announce(key);
    }
  };

  const closeRuntime = (x: LiveTab) => {
    if (x.runtimeId !== null) void api.liveClose(x.runtimeId).catch(() => {});
  };

  /** Closes a tab's session and removes it; optionally offers "Reopen". */
  const remove = (cur: LiveTab, offerReopen: boolean) => {
    closeRuntime(cur);
    disposeSink(cur.key);
    sentGrid.delete(cur.key);
    const { tabs, activeKey } = get();
    const i = tabs.findIndex((x) => x.key === cur.key);
    const rest = tabs.filter((x) => x.key !== cur.key);
    const nextActive =
      activeKey === cur.key ? (rest[Math.min(i, rest.length - 1)]?.key ?? null) : activeKey;
    const closed: ClosedTab = {
      target: cur.target,
      meta: { sessionId: cur.sessionId, name: cur.name, envTag: cur.envTag },
    };
    set((s) => ({
      tabs: rest,
      activeKey: nextActive,
      closed: [...s.closed, closed].slice(-MAX_CLOSED),
    }));
    if (offerReopen) {
      toast({
        message: t('tabs.closed', { name: cur.name }),
        durationMs: REOPEN_TOAST_MS,
        actions: [{ label: t('tabs.reopen'), primary: true, run: () => get().reopenClosed() }],
      });
    }
  };

  return {
    tabs: [],
    activeKey: null,
    closed: [],

    openSerial(settings, meta = {}) {
      return get().openTarget({ type: 'serial', settings }, meta);
    },

    openTarget(target, meta = {}) {
      // A serial port is exclusive: reuse a tab that already owns it.
      const settings = target.type === 'serial' ? target.settings : null;
      const existing = settings
        ? get().tabs.find((x) => {
            const s = serialOf(x);
            return s !== null && samePort(s.port, settings.port);
          })
        : undefined;
      if (
        existing &&
        settings &&
        (existing.state !== 'disconnected' || serialOf(existing)?.baud === settings.baud)
      ) {
        get().activate(existing.key);
        if (existing.state === 'disconnected') get().reconnect(existing.key);
        else useUi.getState().flash(t('tabs.alreadyOpen', { port: portLabel(existing) }));
        return existing.key;
      }
      if (existing) remove(existing, false);

      const key = `t${keySeq++}`;
      createSink(key, {
        dropNotice: (n) =>
          `\r\n\x1b[2m${t('terminal.dropped', { size: formatBytes(n) })}\x1b[22m\r\n`,
      });
      const fresh: LiveTab = {
        key,
        runtimeId: null,
        generation: 0,
        sessionId: meta.sessionId ?? null,
        name: meta.name?.trim() || endpointLabel(target),
        envTag: normalizeEnv(meta.envTag),
        target: structuredClone(target),
        state: 'connecting',
        error: null,
        autobaud: null,
        link: null,
        netLink: null,
        prompt: null,
        health: null,
        connectedAt: null,
        mode: 'unknown',
      };
      set((s) => ({ tabs: [...s.tabs, fresh], activeKey: key }));
      void start(key);
      return key;
    },

    openSession(session) {
      const meta = { sessionId: session.id, name: session.name, envTag: session.envTag };
      if (session.protocol === 'serial') {
        if (!session.serial) {
          toast({ message: t('errors.protocolNotSupported') });
          return;
        }
        get().openSerial(session.serial, meta);
        return;
      }
      if (!session.host) {
        toast({ message: t('errors.invalid') });
        return;
      }
      get().openTarget(
        {
          type: 'network',
          protocol: session.protocol,
          host: session.host,
          port: session.port ?? DEFAULT_PORTS[session.protocol],
          username: session.username,
          ssh: { ...DEFAULT_SSH, ...(session.ssh ?? {}) },
        },
        meta,
      );
    },

    openConsole(port) {
      // Baud stays automatic: the backend uses the remembered speed for this cable first.
      get().openSerial({ ...DEFAULT_SERIAL, port: port.path }, { name: port.displayName });
    },

    activate(key) {
      if (tab(key)) set({ activeKey: key });
    },
    next() {
      const { tabs, activeKey } = get();
      if (tabs.length < 2) return;
      const i = tabs.findIndex((x) => x.key === activeKey);
      set({ activeKey: (tabs[(i + 1) % tabs.length] as LiveTab).key });
    },
    prev() {
      const { tabs, activeKey } = get();
      if (tabs.length < 2) return;
      const i = tabs.findIndex((x) => x.key === activeKey);
      set({ activeKey: (tabs[(i - 1 + tabs.length) % tabs.length] as LiveTab).key });
    },
    selectIndex(index) {
      const { tabs } = get();
      // Index 8 (⌘9) always means "last tab", like browsers.
      const target = index >= 8 ? tabs[tabs.length - 1] : tabs[index];
      if (target) set({ activeKey: target.key });
    },

    close(key) {
      const k = resolveKey(key);
      const cur = k ? tab(k) : undefined;
      if (cur) remove(cur, true);
    },

    reopenClosed() {
      const closed = [...get().closed];
      const last = closed.pop();
      if (!last) return;
      set({ closed });
      get().openTarget(last.target, last.meta);
    },

    reconnect(key) {
      const k = resolveKey(key);
      const cur = k ? tab(k) : undefined;
      if (!cur || cur.state === 'connected' || cur.state === 'detecting') return;
      if (cur.state === 'connecting' && cur.runtimeId !== null) return;
      if (cur.runtimeId === null) {
        void start(cur.key);
        return;
      }
      update(cur.key, (x) => ({ ...x, error: null }));
      api.liveReconnect(cur.runtimeId).catch((e: unknown) => {
        // The backend forgot the runtime (e.g. app state reset): open it again.
        if (describeError(e, t).code === 'notFound') void start(cur.key);
        else
          update(cur.key, (x) => ({ ...x, state: 'disconnected', error: describeErrorAsApp(e) }));
      });
    },

    sendBreak(key) {
      const k = resolveKey(key);
      const cur = k ? tab(k) : undefined;
      if (!cur || cur.runtimeId === null) return;
      if (cur.state !== 'connected') {
        useUi.getState().flash(t('status.breakNeedsConnection'));
        return;
      }
      api
        .liveBreak(cur.runtimeId)
        .then(() => useUi.getState().flash(t('status.breakSent')))
        .catch((e: unknown) => {
          const d = describeError(e, t, { port: portLabel(cur) });
          toast({ kind: 'error', message: d.message, detail: d.detail });
        });
    },

    changeBaud(key, baud) {
      const cur = tab(key);
      if (!cur || cur.target.type !== 'serial') return;
      closeRuntime(cur);
      update(key, (x) =>
        x.target.type === 'serial'
          ? { ...x, target: { ...x.target, settings: { ...x.target.settings, baud } } }
          : x,
      );
      void start(key);
    },

    forgetHostKey(key) {
      const k = resolveKey(key);
      const cur = k ? tab(k) : undefined;
      if (!cur || cur.target.type !== 'network') return;
      const host = cur.target.host;
      const port = cur.target.port ?? DEFAULT_PORTS[cur.target.protocol] ?? 22;
      api
        .knownHostForget(host, port)
        .then(() => {
          useUi.getState().flash(t('ssh.hostKeyForgotten'));
          get().reconnect(cur.key);
        })
        .catch((e: unknown) => {
          const d = describeError(e, t);
          toast({ kind: 'error', message: d.message, detail: d.detail });
        });
    },

    write(key, data) {
      const cur = tab(key);
      if (!cur) return;
      if (cur.prompt) {
        // Login answers stay local until complete; secrets are never echoed or logged.
        const step = feedPrompt(cur.prompt, data);
        if (step.echo) getSink(key)?.push(encoder.encode(step.echo));
        update(key, (x) => ({ ...x, prompt: step.prompt }));
        if (cur.runtimeId === null) return;
        if (step.submit) {
          void api.liveAnswer(cur.runtimeId, step.submit).catch(() => {});
        } else if (step.cancel) {
          closeRuntime(cur);
          update(key, (x) => ({ ...x, runtimeId: null, state: 'disconnected', prompt: null }));
          announce(key);
        }
        return;
      }
      if (cur.state === 'disconnected') {
        // Enter on a dead session reconnects instead of being swallowed.
        if (data.includes('\r')) get().reconnect(key);
        return;
      }
      if (cur.state !== 'connected' || cur.runtimeId === null) return;
      const sink = getSink(key);
      if (sink) sink.tx += new TextEncoder().encode(data).length;
      api.liveWrite(cur.runtimeId, data).catch(() => {
        // Write failures surface through the session's state events (deviceGone etc.).
      });
    },

    resize(key, cols, rows) {
      if (cols < 2 || rows < 1) return;
      lastGrid.cols = cols;
      lastGrid.rows = rows;
      const prev = resizeTimers.get(key);
      if (prev) clearTimeout(prev);
      resizeTimers.set(
        key,
        setTimeout(() => {
          resizeTimers.delete(key);
          const cur = tab(key);
          const grid = `${cols}x${rows}`;
          if (!cur || cur.runtimeId === null || sentGrid.get(key) === grid) return;
          sentGrid.set(key, grid);
          api.liveResize(cur.runtimeId, cols, rows).catch(() => {});
        }, 100),
      );
    },
  };
});

/** Normalizes any rejection into an AppError-shaped value for the tab state. */
function describeErrorAsApp(e: unknown) {
  const d = describeError(e, t);
  return { code: d.code, detail: d.detail };
}

export const useActiveTab = (): LiveTab | null =>
  useTabs((s) => s.tabs.find((x) => x.key === s.activeKey) ?? null);
