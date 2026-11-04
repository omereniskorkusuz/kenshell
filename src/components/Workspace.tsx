/** Main area: tab bar + panes (or the empty state) + side panel (overlay or push). */
import { Usb } from 'lucide-react';
import { useT } from '../i18n';
import { isMac } from '../lib/ipc';
import { formatChord } from '../lib/shortcuts';
import { newSerialSession } from '../app/actions';
import { chipLabel, useConsoles } from '../stores/consoles';
import { useTabs } from '../stores/tabs';
import { Pane } from './Pane';
import { SidePanel } from './SidePanel';
import { TabBar } from './TabBar';

export function Workspace() {
  const tabs = useTabs((s) => s.tabs);
  const activeKey = useTabs((s) => s.activeKey);

  return (
    <div className="relative flex min-w-0 flex-1">
      <main className="flex min-w-0 flex-1 flex-col">
        {tabs.length > 0 && <TabBar />}
        <div className="relative min-h-0 flex-1 bg-terminal">
          {tabs.map((tab) => (
            <Pane key={tab.key} tab={tab} visible={tab.key === activeKey} />
          ))}
          {tabs.length === 0 && <EmptyState />}
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
  const first = ports[0];

  return (
    <div className="flex h-full" data-mode="unknown">
      <div className="mode-stripe shrink-0" aria-hidden />
      <div className="m-auto flex max-w-md flex-col items-center gap-3 px-6 text-center">
        <h1 className="text-lg font-medium text-secondary">{t('shell.emptyTitle')}</h1>
        <p className="text-sm text-muted">
          {t('shell.emptyHint', { shortcut: formatChord('mod+l', mac) })}
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
            <span className="kbd">{formatChord('mod+n', mac)}</span>
          </button>
        </div>
        <p className="mt-2 text-xs text-muted">
          {t('shell.paletteHint', { shortcut: formatChord('mod+k', mac) })}
        </p>
      </div>
    </div>
  );
}
