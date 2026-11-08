/**
 * App shell (Concept "Quiet Console"): top bar · sidebar | tabs + panes | side panel · status
 * bar, plus overlays (palette, settings, session editor, toasts) and the aria-live region.
 */
import { lazy, Suspense, useEffect } from 'react';
import { useT } from '../i18n';
import { api, appReady } from '../lib/ipc';
import { describeError } from '../lib/errors';
import { LiveRegion } from '../components/LiveRegion';
import { SessionEditor } from '../components/SessionEditor';
import { Sidebar } from '../components/Sidebar';
import { StatusBar } from '../components/StatusBar';
import { Toasts } from '../components/Toasts';
import { TopBar } from '../components/TopBar';
import { Workspace } from '../components/Workspace';
import { loadTerminalView } from '../components/Pane';
import { useConsoles } from '../stores/consoles';
import { useLibrary } from '../stores/library';
import { useNotes } from '../stores/notes';
import { useTabs } from '../stores/tabs';
import { setPersistErrorHandler, useSettings } from '../stores/settings';
import { toast } from '../stores/toasts';
import { useUi } from '../stores/ui';
import { installSettingsEffects } from './effects';
import { installShortcuts } from './keyboard';
import { scheduleUpdateCheck } from './updates';

/** Windows opened by moving a tab out (P2-T19) show only their sessions: no library. */
const DETACHED = typeof location !== 'undefined' && location.hash === '#detached';

const loadPalette = () => import('../palette/CommandPalette');
const loadSettings = () => import('../settings/SettingsView');
const CommandPalette = lazy(loadPalette);
const SettingsView = lazy(loadSettings);
const NetScan = lazy(() => import('../netscan/NetScan'));
const Replay = lazy(() => import('../terminal/Replay'));

/** Warm the lazy chunks after first paint so the first ⌘K / tab open is instant. */
function preloadChunks(): void {
  const idle =
    typeof requestIdleCallback === 'function'
      ? (cb: () => void) => requestIdleCallback(cb, { timeout: 2000 })
      : (cb: () => void) => setTimeout(cb, 800);
  idle(() => {
    void loadTerminalView();
    void loadPalette();
  });
}

export function App() {
  const t = useT();
  const paletteOpen = useUi((s) => s.paletteOpen);
  const settingsOpen = useUi((s) => s.settingsOpen);
  const scanOpen = useUi((s) => s.scanOpen);
  const replay = useUi((s) => s.replay);
  const editor = useUi((s) => s.editor);

  useEffect(() => {
    // First commit done → show the (initially hidden) window. rAF does not fire while a
    // WKWebView window is hidden, so this must not wait for an animation frame.
    void appReady().then((bench) => {
      if (bench) void import('./bench').then((m) => m.openBenchTabs(bench));
    });

    setPersistErrorHandler((e) => {
      const d = describeError(e, t);
      toast({ kind: 'error', message: d.message, detail: d.detail });
    });
    const disposeEffects = installSettingsEffects();
    const disposeKeys = installShortcuts();
    const disposeUpdates = scheduleUpdateCheck();
    void useSettings.getState().load();
    void useLibrary.getState().load();
    void useNotes.getState().loadPinned();
    void useConsoles.getState().load();
    // A moved tab waits for this window in the backend (null in the main window).
    void api
      .windowAdopt()
      .then((h) => {
        if (h) useTabs.getState().adopt(h);
      })
      .catch(() => {});
    let unlisten: (() => void) | null = null;
    let cancelled = false;
    void useConsoles
      .getState()
      .subscribe()
      .then((fn) => {
        if (cancelled) fn();
        else unlisten = fn;
      })
      .catch(() => {});
    preloadChunks();

    return () => {
      cancelled = true;
      unlisten?.();
      disposeEffects();
      disposeKeys();
      disposeUpdates();
    };
    // Boot once; `t` is a stable module function.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return (
    <div className="flex h-full flex-col">
      <TopBar />
      <div className="flex min-h-0 flex-1">
        {!DETACHED && <Sidebar />}
        <Workspace />
      </div>
      <StatusBar />
      <Toasts />
      <LiveRegion />
      <Suspense fallback={null}>
        {paletteOpen && <CommandPalette />}
        {settingsOpen && <SettingsView />}
        {scanOpen && <NetScan />}
        {replay && <Replay key={replay} path={replay} />}
      </Suspense>
      {editor && <SessionEditor key={editor.sessionId ?? 'new'} state={editor} />}
    </div>
  );
}
