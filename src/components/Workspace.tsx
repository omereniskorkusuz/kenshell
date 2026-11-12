/** Main area: tab bar + panes (or the empty state) + side panel (overlay or push). */
import { Usb } from 'lucide-react';
import { lazy, Suspense } from 'react';
import { useT } from '../i18n';
import { isMac } from '../lib/ipc';
import { useShortcuts } from '../app/actions';
import { newSerialSession } from '../app/actions';
import { chipLabel, useConsoles } from '../stores/consoles';
import { cellRect } from '../lib/split';
import { useTabs } from '../stores/tabs';
import { useUi } from '../stores/ui';
import { LogoMark, Wordmark } from './Logo';
import { NewTabPage } from './NewTabPage';
import { Pane } from './Pane';
import { SidePanel } from './SidePanel';
import { TabBar } from './TabBar';

/** The map loads only when its tab is first opened. */
const TopologyMap = lazy(() => import('./TopologyMap'));

export function Workspace() {
  const tabs = useTabs((s) => s.tabs);
  const activeKey = useTabs((s) => s.activeKey);
  const view = useTabs((s) => s.view);
  const topologyOpen = useUi((s) => s.topologyOpen);
  const topology = useUi((s) => s.topologyActive);
  const newTabs = useUi((s) => s.newTabs);
  const newTab = useUi((s) => s.activeNewTab);
  // Something other than a session is on screen.
  const covered = topology || newTab !== null;
  const split = view.keys.length >= 2;
  const shown = split ? view.keys : activeKey ? [activeKey] : [];

  return (
    <div className="relative flex min-w-0 flex-1">
      <main className="flex min-w-0 flex-1 flex-col">
        {(tabs.length > 0 || topologyOpen || newTabs.length > 0) && <TabBar />}
        <div className="relative min-h-0 flex-1 bg-terminal">
          {tabs.map((tab) => {
            const i = shown.indexOf(tab.key);
            return (
              <Pane
                key={tab.key}
                tab={tab}
                visible={i >= 0 && !covered}
                focused={tab.key === activeKey && !covered}
                rect={split && i >= 0 ? cellRect(i, shown.length, view.dir) : null}
              />
            );
          })}
          {tabs.length === 0 && !covered && <EmptyState />}
          {newTab !== null && (
            <div className="absolute inset-0" role="tabpanel" aria-labelledby={`tab-new-${newTab}`}>
              <NewTabPage key={newTab} />
            </div>
          )}
          {topology && (
            <div className="absolute inset-0" role="tabpanel" aria-labelledby="tab-topology">
              <Suspense fallback={null}>
                <TopologyMap />
              </Suspense>
            </div>
          )}
        </div>
      </main>
      {/* Pushes (takes layout space) at ≥ 1600px or when pinned, else floats over the pane. */}
      <SidePanel />
    </div>
  );
}

function EmptyState() {
  const t = useT();
  const ports = useConsoles((s) => s.snapshot.ports);
  const mac = isMac();
  const sc = useShortcuts(mac);
  const first = ports[0];

  return (
    <div className="flex h-full" data-mode="unknown">
      <div className="mode-stripe shrink-0" aria-hidden />
      <div className="m-auto flex max-w-md flex-col items-center gap-3 px-6 text-center">
        <div className="mb-4 flex select-none flex-col items-center gap-3">
          <LogoMark size={96} />
          <Wordmark className="text-3xl" />
        </div>
        <h1 className="text-lg font-medium text-secondary">{t('shell.emptyTitle')}</h1>
        <p className="text-sm text-muted">
          {t('shell.emptyHint', { shortcut: sc.label('quickConnect.focus') })}
        </p>
        <div className="mt-2 flex flex-wrap justify-center gap-2">
          {first && (
            <button
              type="button"
              className="btn btn-primary"
              onClick={() => useTabs.getState().openConsole(first)}
            >
              <Usb size={14} aria-hidden />
              {t('shell.connectFirst', { port: first.displayName, chip: chipLabel(first) })}
            </button>
          )}
          <button type="button" className="btn" onClick={newSerialSession}>
            {t('actions.newSerialSession')}
            {sc.label('session.newSerial') && (
              <span className="kbd">{sc.label('session.newSerial')}</span>
            )}
          </button>
        </div>
        <p className="mt-2 text-xs text-muted">
          {t('shell.paletteHint', { shortcut: sc.label('palette.open') })}
        </p>
      </div>
    </div>
  );
}
