import { PanelLeft, Radar, Settings } from 'lucide-react';
import { useT } from '../i18n';
import { isMac } from '../lib/ipc';
import { formatChord } from '../lib/shortcuts';
import { useSettings } from '../stores/settings';
import { useUi } from '../stores/ui';
import { runAction } from '../app/actions';
import { QuickConnect } from './QuickConnect';

export function TopBar() {
  const t = useT();
  const mac = isMac();
  const collapsed = useSettings((s) => s.values['ui.sidebarCollapsed']);

  return (
    <header
      className="flex shrink-0 items-center gap-3 border-b border-subtle bg-surface px-2"
      style={{ height: 'var(--top-bar-height)' }}
    >
      <button
        type="button"
        className="icon-btn"
        aria-pressed={!collapsed}
        aria-label={t('actions.toggleSidebar')}
        title={`${t('actions.toggleSidebar')} (${formatChord('mod+b', mac)})`}
        onClick={() => runAction('sidebar.toggle')}
      >
        <PanelLeft size={16} />
      </button>
      <span className="font-semibold tracking-tight">{t('app.name')}</span>
      <QuickConnect />
      <div className="ml-auto flex items-center gap-1">
        <button
          type="button"
          className="btn btn-ghost btn-sm text-secondary"
          title={t('shell.scanNetwork')}
          onClick={() => useUi.getState().setScanOpen(true)}
        >
          <Radar size={14} aria-hidden />{' '}
          <span className="hidden lg:inline">{t('shell.scanNetwork')}</span>
        </button>
        <button
          type="button"
          className="btn btn-ghost btn-sm text-secondary"
          aria-label={t('shell.commandPalette')}
          title={t('shell.commandPalette')}
          onClick={() => useUi.getState().openPalette()}
        >
          <span className="kbd">{formatChord('mod+k', mac)}</span>
        </button>
        <button
          type="button"
          className="icon-btn"
          aria-label={t('shell.settings')}
          title={`${t('shell.settings')} (${formatChord('mod+,', mac)})`}
          onClick={() => useUi.getState().setSettingsOpen(true)}
        >
          <Settings size={16} />
        </button>
      </div>
    </header>
  );
}
