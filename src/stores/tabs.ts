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
  canDetach,
  DEFAULT_PORTS,
  DEFAULT_SERIAL,
  DEFAULT_SSH,
  liveAttach,
  liveOpen,
  type ConsolePort,
  type Handoff,
  type OpenTarget,
  type SerialSettings,
  type SessionEvent,
  type SessionRecord,
} from '../lib/ipc';
import { appendTail, MAX_WAIT_MS, readyToSend } from '../lib/autoCommands';
import { decodeHandoff, encodeHandoff } from '../lib/handoff';
import { normalizePaste, pasteChunks, wantsPacing } from '../lib/paste';
import { feedPrompt, promptIntro, startPrompt } from '../lib/prompt';
import {
  applySessionEvent,
  endpointLabel,
  portLabel,
  serialOf,
  NO_DEVICE,
  tabStatus,
  type LiveTab,
} from '../lib/sessionState';
import {
  activateIn,
  addPane,
  dropOn,
  MAX_PANES,
  moveItem,
  neighbor,
  removePane,
  type Arrow,
  type DropZone,
  type SplitDir,
  type SplitView,
} from '../lib/split';
import { normalizeEnv } from '../lib/tree';
import { getTerminal } from '../terminal/registry';
import { createSink, disposeSink, getSink } from '../terminal/sink';
import { useLibrary } from './library';
import { getSetting, useSettings } from './settings';
import { toast } from './toasts';
import { useUi } from './ui';
import { addHistory } from '../lib/history';

export interface OpenMeta {
  sessionId?: number | null;
  name?: string;
  envTag?: string | null;
  /** Post-connect commands of the saved session. */
  autoCommands?: string[];
}

interface ClosedTab {
  target: OpenTarget;
  meta: OpenMeta;
}

interface TabsState {
  tabs: LiveTab[];
  activeKey: string | null;
  closed: ClosedTab[];
  /** Tabs shown side by side (2–4); empty keys = single view of the active tab. */
  view: SplitView;
  /** Broadcast group (P3-T13): typing or pasting in one of these tabs goes to all of them. */
  broadcast: string[];

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
  /** Starts / stops recording the tab's output (P5-T06). */
  toggleRecording(key?: string | null): Promise<void>;
  /** Saves the tab's scrollback as plain text or a coloured HTML page (P5-T06). */
  exportOutput(format: 'txt' | 'html', key?: string | null): Promise<void>;
  changeBaud(key: string, baud: number | null): void;
  write(key: string, data: string): void;
  /** Keyboard input of a tab (to the whole broadcast group when the tab is in it). */
  input(key: string, data: string): void;
  /** A paste in a tab (to the whole broadcast group when the tab is in it). */
  pasteInput(key: string, text: string): void;
  /** Adds the tab to / removes it from the broadcast group (default: the active tab). */
  toggleBroadcast(key?: string | null): void;
  broadcastAll(): void;
  stopBroadcast(): void;
  /** Pastes text: CR line ends; serial/telnet multi-line pastes are paced per line. */
  paste(key: string, text: string): void;
  /** Forgets the stored SSH host key of this tab's host and reconnects. */
  forgetHostKey(key?: string | null): void;
  resize(key: string, cols: number, rows: number): void;

  /** Moves a tab to position `to` in the tab strip. */
  move(key: string, to: number): void;
  /** Moves the active tab one place left (-1) or right (+1). */
  moveBy(delta: number): void;
  /** Shows another tab beside the active one (default: the next tab not yet shown). */
  splitAdd(dir: SplitDir, key?: string): void;
  /** A tab dragged onto a shown pane (edge = beside it, center = in its place). */
  dropTab(drag: string, target: string, zone: DropZone): void;
  /** Takes a tab out of the split view (default: the active one). */
  unsplit(key?: string | null): void;
  singleView(): void;
  focusPane(arrow: Arrow): void;
  /** Moves a tab with its live session into a new window (dropped at screen point `at`). */
  detach(key?: string | null, at?: { x: number; y: number }): Promise<void>;
  /** Detached window start-up: takes over the tab handed to this window. */
  adopt(handoff: Handoff): void;
}

/** Last measured terminal grid; used for the initial size of new sessions. */
const lastGrid = { cols: 80, rows: 24 };
const sentGrid = new Map<string, string>();
const resizeTimers = new Map<string, ReturnType<typeof setTimeout>>();
let keySeq = 1;
const MAX_CLOSED = 10;
/** Recent output per tab (for auto-command prompt detection) and its last arrival time. */
const tails = new Map<string, string>();
const lastData = new Map<string, number>();
const autoMeta = new Map<string, string[]>();
const encoder = new TextEncoder();
const REOPEN_TOAST_MS = 10_000;
/** End markers (empty data batch + `parked` event) per tab being moved to another window. */
const drains = new Map<string, { data: boolean; event: boolean; done: () => void }>();
const DRAIN_TIMEOUT_MS = 1500;

function markDrained(key: string, part: 'data' | 'event'): void {
  const d = drains.get(key);
  if (!d) return;
  d[part] = true;
  if (d.data && d.event) d.done();
}

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
      logPath: null,
      recordPath: null,
      connectedAt: null,
      health: x.health === 'cableReturned' ? x.health : null,
    }));
    sentGrid.delete(key);
    const { onData, onEvent } = wire(key, generation);
    const isCurrent = () => tab(key)?.generation === generation;

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

  /** Output and event handlers of one session generation of a tab. */
  const wire = (key: string, generation: number) => {
    const isCurrent = () => tab(key)?.generation === generation;
    const sink = getSink(key);

    const onData = (bytes: Uint8Array) => {
      if (!isCurrent()) return;
      if (bytes.length === 0) {
        markDrained(key, 'data');
        return;
      }
      sink?.push(bytes);
      tails.set(key, appendTail(tails.get(key) ?? '', bytes));
      lastData.set(key, Date.now());
      if (tab(key)?.health === 'noData') update(key, (x) => ({ ...x, health: null }));
    };
    const onEvent = (ev: SessionEvent) => {
      if (!isCurrent()) return;
      const before = tab(key);
      if (!before) return;
      if (ev.type === 'parked') {
        markDrained(key, 'event');
        return;
      }
      if (ev.type === 'prompt') {
        sink?.push(encoder.encode(promptIntro(ev.request)));
        update(key, (x) => ({ ...x, prompt: startPrompt(ev.request) }));
        announce(key);
        return;
      }
      if (ev.type === 'logging' && ev.error) {
        const d = describeError(ev.error, t);
        toast({ kind: 'error', message: t('log.openFailed'), detail: d.detail });
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
      if (ev.type === 'state' && ev.state === 'connected' && before.state !== 'connected') {
        // Every connection goes to the history, saved session or not.
        useSettings.getState().set(
          'history.connections',
          addHistory(getSetting('history.connections'), {
            name: after.name,
            envTag: after.envTag,
            sessionId: after.sessionId,
            target: structuredClone(after.target),
            at: Date.now(),
          }),
        );
        runAutoCommands(key, generation);
      }
    };
    return { onData, onEvent };
  };

  /** Points the tab's live session output at this window again (after a failed move). */
  const reattach = async (key: string, runtimeId: number) => {
    const cur = tab(key);
    if (!cur) return;
    const { onData, onEvent } = wire(key, cur.generation);
    try {
      await liveAttach(runtimeId, onData, onEvent);
    } catch (e) {
      update(key, (x) => ({
        ...x,
        runtimeId: null,
        state: 'disconnected',
        error: describeErrorAsApp(e),
      }));
    }
  };

  /** Activates `key`; in split view a tab not shown takes the focused pane's place. */
  const show = (key: string) => {
    if (!tab(key)) return;
    useUi.getState().leaveTopology();
    set((s) => ({ activeKey: key, view: activateIn(s.view, s.activeKey, key) }));
  };
  const shownKeys = (): string[] => {
    const { view, activeKey } = get();
    return view.keys.length >= 2 ? view.keys : activeKey ? [activeKey] : [];
  };

  /** Sends the saved session's post-connect commands, one by one, at a quiet prompt. */
  const runAutoCommands = (key: string, generation: number) => {
    const cmds = autoMeta.get(key) ?? [];
    if (cmds.length === 0) return;
    const started = Date.now();
    lastData.set(key, lastData.get(key) ?? started);
    let i = 0;
    const tick = () => {
      const cur = tab(key);
      if (!cur || cur.generation !== generation || cur.state !== 'connected') return;
      const now = Date.now();
      if (now - started > MAX_WAIT_MS) return;
      if (!readyToSend(tails.get(key) ?? '', lastData.get(key) ?? 0, now)) {
        setTimeout(tick, 150);
        return;
      }
      const cmd = cmds[i++];
      if (cmd === undefined) return;
      lastData.set(key, now); // the echo/output of this command must settle first
      get().write(key, `${cmd}\r`);
      if (i < cmds.length) setTimeout(tick, 150);
    };
    setTimeout(tick, 150);
  };

  const closeRuntime = (x: LiveTab) => {
    if (x.runtimeId !== null) void api.liveClose(x.runtimeId).catch(() => {});
  };

  /**
   * Removes a tab: closes its session (unless it moved to another window) and optionally
   * offers "Reopen".
   */
  /** Where input typed in `key` goes: the tab itself, plus the other connected tabs of its
   * broadcast group (not ones waiting for a login answer, which stays per tab). */
  const broadcastTargets = (key: string): string[] => {
    const { broadcast, tabs } = get();
    if (!broadcast.includes(key)) return [key];
    const others = tabs.filter(
      (x) =>
        x.key !== key && broadcast.includes(x.key) && x.state === 'connected' && x.prompt === null,
    );
    return [key, ...others.map((x) => x.key)];
  };

  const remove = (cur: LiveTab, opts: { reopen: boolean; keepSession?: boolean }) => {
    if (!opts.keepSession) closeRuntime(cur);
    disposeSink(cur.key);
    tails.delete(cur.key);
    lastData.delete(cur.key);
    autoMeta.delete(cur.key);
    sentGrid.delete(cur.key);
    const { tabs, activeKey, view } = get();
    const i = tabs.findIndex((x) => x.key === cur.key);
    const rest = tabs.filter((x) => x.key !== cur.key);
    let nextActive =
      activeKey === cur.key ? (rest[Math.min(i, rest.length - 1)]?.key ?? null) : activeKey;
    // Closing a shown pane focuses a neighbouring pane, not a hidden tab.
    const at = view.keys.indexOf(cur.key);
    if (activeKey === cur.key && at >= 0) {
      const others = view.keys.filter((k) => k !== cur.key);
      nextActive = others[Math.min(at, others.length - 1)] ?? nextActive;
    }
    const closed: ClosedTab = {
      target: cur.target,
      meta: { sessionId: cur.sessionId, name: cur.name, envTag: cur.envTag },
    };
    set((s) => ({
      tabs: rest,
      activeKey: nextActive,
      view: removePane(s.view, cur.key),
      broadcast: s.broadcast.filter((k) => k !== cur.key),
      closed: opts.keepSession ? s.closed : [...s.closed, closed].slice(-MAX_CLOSED),
    }));
    if (opts.reopen) {
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
    view: { keys: [], dir: 'cols' },
    broadcast: [],

    input(key, data) {
      for (const k of broadcastTargets(key)) get().write(k, data);
    },

    pasteInput(key, text) {
      for (const k of broadcastTargets(key)) get().paste(k, text);
    },

    toggleBroadcast(key) {
      const k = resolveKey(key);
      if (!k) return;
      set((s) => ({
        broadcast: s.broadcast.includes(k)
          ? s.broadcast.filter((x) => x !== k)
          : [...s.broadcast, k],
      }));
    },

    broadcastAll() {
      set((s) => ({ broadcast: s.tabs.map((x) => x.key) }));
    },

    stopBroadcast() {
      set({ broadcast: [] });
    },

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
      if (existing) remove(existing, { reopen: false });

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
        logPath: null,
        recordPath: null,
        health: null,
        connectedAt: null,
        mode: 'unknown',
        submode: null,
        device: NO_DEVICE,
        reloadAt: null,
      };
      autoMeta.set(key, meta.autoCommands ?? []);
      // Opened from a "new session" tab: that tab turns into this session.
      useUi.getState().consumeNewTab();
      set((s) => ({ tabs: [...s.tabs, fresh] }));
      show(key);
      void start(key);
      return key;
    },

    openSession(session) {
      const meta = {
        sessionId: session.id,
        name: session.name,
        envTag: session.envTag,
        autoCommands: session.autoCommands,
      };
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
      show(key);
    },
    next() {
      const { tabs, activeKey } = get();
      if (tabs.length < 2) return;
      const i = tabs.findIndex((x) => x.key === activeKey);
      show((tabs[(i + 1) % tabs.length] as LiveTab).key);
    },
    prev() {
      const { tabs, activeKey } = get();
      if (tabs.length < 2) return;
      const i = tabs.findIndex((x) => x.key === activeKey);
      show((tabs[(i - 1 + tabs.length) % tabs.length] as LiveTab).key);
    },
    selectIndex(index) {
      const { tabs } = get();
      // Index 8 (⌘9) always means "last tab", like browsers.
      const target = index >= 8 ? tabs[tabs.length - 1] : tabs[index];
      if (target) show(target.key);
    },

    close(key) {
      const k = resolveKey(key);
      const cur = k ? tab(k) : undefined;
      if (cur) remove(cur, { reopen: true });
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

    async toggleRecording(key) {
      const k = resolveKey(key);
      const cur = k ? tab(k) : undefined;
      if (!k || !cur || cur.runtimeId === null) return;
      const id = cur.runtimeId;
      try {
        if (cur.recordPath) {
          const path = await api.liveRecordStop(id);
          update(k, (x) => ({ ...x, recordPath: null }));
          if (path) useUi.getState().flash(t('status.recordingSaved', { path }));
        } else {
          const size = getTerminal(k)?.size() ?? { cols: 80, rows: 24 };
          const path = await api.liveRecordStart(id, cur.name, size.cols, size.rows);
          update(k, (x) => ({ ...x, recordPath: path }));
        }
      } catch (e) {
        const d = describeError(e, t);
        toast({ kind: 'error', message: d.message, detail: d.detail });
      }
    },

    async exportOutput(format, key) {
      const k = resolveKey(key);
      const cur = k ? tab(k) : undefined;
      const term = k ? getTerminal(k) : undefined;
      if (!cur || !term) return;
      const stamp = new Date().toISOString().slice(0, 16).replace(/[:T]/g, '-');
      const base = `${cur.name.replace(/[^\w.-]+/g, '_')}_${stamp}`;
      const path = await api.pickPath('save', [format], `${base}.${format}`).catch(() => null);
      if (!path) return;
      const text = format === 'txt' ? term.exportText() : term.exportHtml(cur.name);
      try {
        await api.textExport(path, text);
        useUi.getState().flash(t('status.exported', { path }));
      } catch (e) {
        const d = describeError(e, t);
        toast({ kind: 'error', message: d.message, detail: d.detail });
      }
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

    paste(key, text) {
      const cur = tab(key);
      if (!cur || !text) return;
      if (cur.prompt) {
        // Pasting a password into a login prompt: feed it like typing (first line only).
        get().write(key, normalizePaste(text).split('\r')[0] ?? '');
        return;
      }
      const chunks = pasteChunks(text);
      const delay = useSettings.getState().values['terminal.pasteDelayMs'];
      const protocol = cur.target.type === 'serial' ? 'serial' : cur.target.protocol;
      if (!wantsPacing(protocol, chunks.length, delay)) {
        get().write(key, normalizePaste(text));
        return;
      }
      const generation = cur.generation;
      let i = 0;
      const step = () => {
        const now = tab(key);
        // Stop if the tab closed, reconnected or dropped meanwhile.
        if (!now || now.generation !== generation || now.state !== 'connected') return;
        const chunk = chunks[i++];
        if (chunk === undefined) return;
        get().write(key, chunk);
        if (i < chunks.length) setTimeout(step, delay);
        else useUi.getState().flash(t('terminal.pasted', { count: chunks.length }));
      };
      step();
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

    move(key, to) {
      const { tabs } = get();
      const from = tabs.findIndex((x) => x.key === key);
      if (from < 0 || from === to) return;
      set({ tabs: moveItem(tabs, from, to) });
    },

    moveBy(delta) {
      const { tabs, activeKey } = get();
      const from = tabs.findIndex((x) => x.key === activeKey);
      const to = from + delta;
      if (from < 0 || to < 0 || to >= tabs.length) return;
      set({ tabs: moveItem(tabs, from, to) });
    },

    splitAdd(dir, key) {
      const { tabs, activeKey, view } = get();
      if (!activeKey) return;
      const shown = shownKeys();
      if (shown.length >= MAX_PANES) {
        useUi.getState().flash(t('tabs.splitFull'));
        return;
      }
      const i = tabs.findIndex((x) => x.key === activeKey);
      const candidate =
        key ??
        Array.from({ length: tabs.length - 1 }, (_, j) => tabs[(i + j + 1) % tabs.length])
          .map((x) => x?.key)
          .find((k): k is string => k !== undefined && !shown.includes(k));
      const next = candidate ? addPane(view, activeKey, candidate, dir) : null;
      if (!next || !candidate) {
        useUi.getState().flash(t('tabs.splitNeedsTab'));
        return;
      }
      set({ view: next, activeKey: candidate });
    },

    dropTab(drag, target, zone) {
      const { view, activeKey } = get();
      if (!activeKey || !tab(drag)) return;
      if (zone === 'center' && view.keys.length < 2) {
        show(drag);
        return;
      }
      const next = dropOn(view, activeKey, drag, target, zone);
      if (next !== view) set({ view: next, activeKey: drag });
    },

    unsplit(key) {
      const { view, activeKey } = get();
      const k = resolveKey(key);
      if (view.keys.length < 2 || !k || !view.keys.includes(k)) return;
      const others = view.keys.filter((x) => x !== k);
      const nextActive =
        activeKey === k
          ? (others[Math.min(view.keys.indexOf(k), others.length - 1)] ?? null)
          : activeKey;
      set({ view: removePane(view, k), activeKey: nextActive });
    },

    singleView() {
      set((s) => ({ view: { ...s.view, keys: [] } }));
    },

    focusPane(arrow) {
      const { view, activeKey } = get();
      if (view.keys.length < 2 || !activeKey) return;
      const i = view.keys.indexOf(activeKey);
      const j = neighbor(i, view.keys.length, view.dir, arrow);
      const target = j === null ? undefined : view.keys[j];
      if (target) set({ activeKey: target });
    },

    async detach(key, at) {
      const k = resolveKey(key);
      const cur = k ? tab(k) : undefined;
      if (!cur || !canDetach() || drains.has(cur.key)) return;
      if (cur.runtimeId === null) {
        useUi.getState().flash(t('tabs.detachNeedsSession'));
        return;
      }
      if (cur.prompt) {
        useUi.getState().flash(t('tabs.detachDuringLogin'));
        return;
      }
      const id = cur.runtimeId;
      const drained = new Promise<void>((resolve) => {
        drains.set(cur.key, { data: false, event: false, done: resolve });
        setTimeout(resolve, DRAIN_TIMEOUT_MS);
      });
      try {
        await api.livePark(id);
        // Everything the backend sent before parking has arrived once both markers did.
        await drained;
        const sink = getSink(cur.key);
        const pending = sink?.takePending() ?? new Uint8Array();
        // What the terminal showed, colours included (also when its xterm was disposed).
        const screen = sink?.history.text() ?? '';
        const now = tab(cur.key) ?? cur;
        try {
          await api.windowDetach({
            runtimeId: id,
            title: now.name,
            payload: encodeHandoff(now, screen, pending),
            x: at?.x ?? null,
            y: at?.y ?? null,
          });
        } catch (e) {
          // No new window: keep the tab here and take its output back.
          if (sink && pending.length > 0) {
            sink.rx -= pending.length;
            sink.push(pending);
          }
          await reattach(cur.key, id);
          const d = describeError(e, t);
          toast({ kind: 'error', message: t('tabs.detachFailed'), detail: d.detail });
          return;
        }
        remove(now, { reopen: false, keepSession: true });
      } catch (e) {
        // Parking failed (session gone): nothing moved.
        const d = describeError(e, t);
        toast({ kind: 'error', message: t('tabs.detachFailed'), detail: d.detail });
      } finally {
        drains.delete(cur.key);
      }
    },

    adopt(handoff) {
      const snap = decodeHandoff(handoff.payload);
      if (!snap) {
        void api.liveClose(handoff.runtimeId).catch(() => {});
        toast({ kind: 'error', message: t('tabs.detachFailed') });
        return;
      }
      const key = `t${keySeq++}`;
      const sink = createSink(key, {
        dropNotice: (n) =>
          `\r\n\x1b[2m${t('terminal.dropped', { size: formatBytes(n) })}\x1b[22m\r\n`,
      });
      if (snap.screen) sink.writeLocal(snap.screen);
      if (snap.pending.length > 0) sink.push(snap.pending);
      const fresh: LiveTab = {
        ...snap.tab,
        key,
        runtimeId: handoff.runtimeId,
        generation: 1,
        prompt: null,
      };
      autoMeta.set(key, []);
      set((s) => ({ tabs: [...s.tabs, fresh] }));
      show(key);
      void reattach(key, handoff.runtimeId);
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
