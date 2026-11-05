/**
 * Central action registry. Every user-facing command lives here once; the command palette
 * lists them, the global keyboard handler triggers them and menus show their shortcut hint.
 */
import { pagingDisableCommand } from '../lib/paging';
import { t, type TKey } from '../i18n';
import { canDetach, DEFAULT_SERIAL } from '../lib/ipc';
import type { Arrow } from '../lib/split';
import { useConsoles } from '../stores/consoles';
import { useLibrary } from '../stores/library';
import { getSetting, useSettings } from '../stores/settings';
import { useTabs } from '../stores/tabs';
import { useUi } from '../stores/ui';

export type ActionGroup = 'actions' | 'settings';

export interface AppAction {
  id: string;
  title: TKey;
  group: ActionGroup;
  /** Shortcut notation, see lib/shortcuts.ts ("mod+shift+b"). */
  shortcut?: string;
  /** Extra search words (English + Turkish, so either language finds it). */
  keywords?: string[];
  /** Not listed in the palette (e.g. "go to tab 3"). */
  hidden?: boolean;
  enabled?: () => boolean;
  run: () => void;
}

const activeTab = () => {
  const { tabs, activeKey } = useTabs.getState();
  return tabs.find((x) => x.key === activeKey) ?? null;
};
const hasActiveTab = () => activeTab() !== null;
const isSerialTab = () => activeTab()?.target.type === 'serial';

export function newSerialSession(): void {
  useUi.getState().openEditor({
    sessionId: null,
    initial: { protocol: 'serial', serial: { ...DEFAULT_SERIAL, port: '' } },
  });
}

export function connectDetectedConsole(): void {
  const ports = useConsoles.getState().snapshot.ports;
  if (ports.length === 1 && ports[0]) useTabs.getState().openConsole(ports[0]);
  else if (ports.length === 0) useUi.getState().flash(t('consoles.none'));
  else useUi.getState().openPalette('', 'consoles');
}

function toggleSidebar(): void {
  const s = useSettings.getState();
  s.set('ui.sidebarCollapsed', !s.values['ui.sidebarCollapsed']);
}

const inSplit = () => useTabs.getState().view.keys.length >= 2;

const paneFocus: AppAction[] = (['left', 'right', 'up', 'down'] as Arrow[]).map((arrow) => ({
  id: `pane.focus.${arrow}`,
  title: 'actions.focusPane' as TKey,
  group: 'actions' as const,
  shortcut: `mod+alt+arrow${arrow}`,
  hidden: true,
  enabled: inSplit,
  run: () => useTabs.getState().focusPane(arrow),
}));

const tabSelectors: AppAction[] = Array.from({ length: 9 }, (_, i) => ({
  id: `tab.select${i + 1}`,
  title: 'actions.selectTab' as TKey,
  group: 'actions' as const,
  shortcut: `mod+${i + 1}`,
  hidden: true,
  run: () => useTabs.getState().selectIndex(i),
}));

export const ACTIONS: readonly AppAction[] = [
  {
    id: 'palette.open',
    title: 'actions.commandPalette',
    group: 'actions',
    shortcut: 'mod+k',
    hidden: true,
    run: () => {
      const ui = useUi.getState();
      if (ui.paletteOpen) ui.closePalette();
      else ui.openPalette();
    },
  },
  {
    id: 'quickConnect.focus',
    title: 'actions.quickConnect',
    group: 'actions',
    shortcut: 'mod+l',
    keywords: ['connect', 'bağlan'],
    run: () => useUi.getState().focusQuickConnect(),
  },
  {
    id: 'session.newSerial',
    title: 'actions.newSerialSession',
    group: 'actions',
    shortcut: 'mod+n',
    keywords: ['serial', 'console', 'seri', 'konsol', 'add', 'ekle'],
    run: newSerialSession,
  },
  {
    id: 'session.open',
    title: 'actions.openSession',
    group: 'actions',
    keywords: ['session', 'oturum'],
    run: () => useUi.getState().openPalette('@'),
  },
  {
    id: 'console.connect',
    title: 'actions.connectConsole',
    group: 'actions',
    keywords: ['usb', 'console', 'konsol', 'serial', 'seri'],
    run: connectDetectedConsole,
  },
  {
    id: 'console.rescan',
    title: 'actions.rescanConsoles',
    group: 'actions',
    keywords: ['usb', 'refresh', 'yenile', 'driver', 'sürücü'],
    run: () => void useConsoles.getState().rescan(),
  },
  {
    id: 'folder.new',
    title: 'actions.newFolder',
    group: 'actions',
    keywords: ['folder', 'klasör', 'group', 'grup'],
    run: () => void useLibrary.getState().createFolder(null),
  },
  {
    id: 'tab.close',
    title: 'actions.closeTab',
    group: 'actions',
    shortcut: 'mod+w',
    enabled: hasActiveTab,
    run: () => useTabs.getState().close(),
  },
  {
    id: 'tab.reopen',
    title: 'actions.reopenTab',
    group: 'actions',
    shortcut: 'mod+shift+t',
    enabled: () => useTabs.getState().closed.length > 0,
    run: () => useTabs.getState().reopenClosed(),
  },
  {
    id: 'tab.next',
    title: 'actions.nextTab',
    group: 'actions',
    shortcut: 'ctrl+tab',
    enabled: () => useTabs.getState().tabs.length > 1,
    run: () => useTabs.getState().next(),
  },
  {
    id: 'tab.prev',
    title: 'actions.prevTab',
    group: 'actions',
    shortcut: 'ctrl+shift+tab',
    enabled: () => useTabs.getState().tabs.length > 1,
    run: () => useTabs.getState().prev(),
  },
  {
    id: 'tab.moveLeft',
    title: 'actions.moveTabLeft',
    group: 'actions',
    shortcut: 'mod+shift+pageup',
    keywords: ['reorder', 'sırala', 'tab', 'sekme'],
    enabled: () => useTabs.getState().tabs.length > 1,
    run: () => useTabs.getState().moveBy(-1),
  },
  {
    id: 'tab.moveRight',
    title: 'actions.moveTabRight',
    group: 'actions',
    shortcut: 'mod+shift+pagedown',
    keywords: ['reorder', 'sırala', 'tab', 'sekme'],
    enabled: () => useTabs.getState().tabs.length > 1,
    run: () => useTabs.getState().moveBy(1),
  },
  {
    id: 'pane.splitRight',
    title: 'actions.splitRight',
    group: 'actions',
    // Terminator's split keys; Ctrl+D stays EOF in the terminal on Windows/Linux.
    shortcut: 'mod+shift+e',
    keywords: ['split', 'böl', 'pane', 'bölme', 'side by side', 'yan yana'],
    enabled: hasActiveTab,
    run: () => useTabs.getState().splitAdd('cols'),
  },
  {
    id: 'pane.splitDown',
    title: 'actions.splitDown',
    group: 'actions',
    shortcut: 'mod+shift+o',
    keywords: ['split', 'böl', 'pane', 'bölme'],
    enabled: hasActiveTab,
    run: () => useTabs.getState().splitAdd('rows'),
  },
  {
    id: 'pane.single',
    title: 'actions.singleView',
    group: 'actions',
    shortcut: 'mod+shift+x',
    keywords: ['split', 'böl', 'unsplit', 'single', 'tek'],
    enabled: inSplit,
    run: () => useTabs.getState().singleView(),
  },
  {
    id: 'terminal.disablePaging',
    title: 'actions.disablePaging',
    group: 'actions',
    keywords: ['more', 'paging', 'sayfa', 'terminal length', 'screen-length'],
    enabled: () => {
      const tab = activeTab();
      return (
        tab?.state === 'connected' && pagingDisableCommand(tab.device.vendor?.id ?? null) !== null
      );
    },
    run: () => {
      const tab = activeTab();
      const cmd = tab ? pagingDisableCommand(tab.device.vendor?.id ?? null) : null;
      if (tab && cmd) useTabs.getState().write(tab.key, `${cmd}\r`);
    },
  },
  {
    id: 'broadcast.toggle',
    title: 'actions.toggleBroadcast',
    group: 'actions',
    shortcut: 'mod+alt+b',
    keywords: ['broadcast', 'yayın', 'multi', 'çoklu', 'all', 'hepsi'],
    enabled: hasActiveTab,
    run: () => useTabs.getState().toggleBroadcast(),
  },
  {
    id: 'broadcast.all',
    title: 'actions.broadcastAll',
    group: 'actions',
    keywords: ['broadcast', 'yayın', 'multi', 'çoklu'],
    enabled: () => useTabs.getState().tabs.length > 1,
    run: () => useTabs.getState().broadcastAll(),
  },
  {
    id: 'broadcast.stop',
    title: 'actions.stopBroadcast',
    group: 'actions',
    keywords: ['broadcast', 'yayın'],
    enabled: () => useTabs.getState().broadcast.length > 0,
    run: () => useTabs.getState().stopBroadcast(),
  },
  {
    id: 'tab.detach',
    title: 'actions.detachTab',
    group: 'actions',
    shortcut: 'mod+shift+n',
    keywords: ['window', 'pencere', 'detach', 'ayır', 'tear off'],
    enabled: () => canDetach() && activeTab()?.runtimeId != null,
    run: () => void useTabs.getState().detach(),
  },
  {
    id: 'session.break',
    title: 'actions.sendBreak',
    group: 'actions',
    shortcut: 'mod+shift+b',
    keywords: ['break', 'rommon', 'boot', 'kesme'],
    enabled: () => isSerialTab() && activeTab()?.state === 'connected',
    run: () => useTabs.getState().sendBreak(),
  },
  {
    id: 'session.reconnect',
    title: 'actions.reconnect',
    group: 'actions',
    shortcut: 'mod+shift+r',
    enabled: () => activeTab()?.state === 'disconnected',
    run: () => useTabs.getState().reconnect(),
  },
  {
    id: 'library.search',
    title: 'actions.searchSessions',
    group: 'actions',
    shortcut: 'mod+shift+f',
    keywords: ['search', 'find', 'session', 'ara', 'oturum'],
    run: () => {
      const st = useSettings.getState();
      if (st.values['ui.sidebarCollapsed']) st.set('ui.sidebarCollapsed', false);
      useUi.getState().focusSearch();
    },
  },
  {
    id: 'terminal.find',
    title: 'actions.find',
    group: 'actions',
    shortcut: 'mod+f',
    keywords: ['search', 'find', 'ara', 'bul', 'scrollback'],
    enabled: hasActiveTab,
    run: () => {
      const tab = activeTab();
      if (tab) useUi.getState().openFind(tab.key);
    },
  },
  {
    id: 'terminal.timestamps',
    title: 'actions.toggleTimestamps',
    group: 'actions',
    keywords: ['timestamp', 'time', 'zaman', 'damga'],
    run: () => {
      const s = useSettings.getState();
      s.set('terminal.timestamps', !s.values['terminal.timestamps']);
    },
  },
  {
    id: 'sidebar.toggle',
    title: 'actions.toggleSidebar',
    group: 'actions',
    shortcut: 'mod+b',
    run: toggleSidebar,
  },
  {
    id: 'sidePanel.toggle',
    title: 'actions.toggleSidePanel',
    group: 'actions',
    shortcut: 'mod+j',
    keywords: ['notes', 'notlar', 'analyzer', 'snippets', 'tools', 'araçlar'],
    run: () => useUi.getState().toggleSidePanel(),
  },
  {
    id: 'theme.dark',
    title: 'actions.themeDark',
    group: 'settings',
    keywords: ['theme', 'tema', 'dark', 'koyu'],
    enabled: () => getSetting('appearance.theme') !== 'dark',
    run: () => useSettings.getState().set('appearance.theme', 'dark'),
  },
  {
    id: 'theme.light',
    title: 'actions.themeLight',
    group: 'settings',
    keywords: ['theme', 'tema', 'light', 'açık'],
    enabled: () => getSetting('appearance.theme') !== 'light',
    run: () => useSettings.getState().set('appearance.theme', 'light'),
  },
  {
    id: 'theme.system',
    title: 'actions.themeSystem',
    group: 'settings',
    keywords: ['theme', 'tema', 'system', 'sistem'],
    enabled: () => getSetting('appearance.theme') !== 'system',
    run: () => useSettings.getState().set('appearance.theme', 'system'),
  },
  {
    id: 'language.tr',
    title: 'actions.languageTr',
    group: 'settings',
    keywords: ['language', 'dil', 'turkish', 'türkçe'],
    run: () => {
      useSettings.getState().set('general.language', 'tr');
    },
  },
  {
    id: 'language.en',
    title: 'actions.languageEn',
    group: 'settings',
    keywords: ['language', 'dil', 'english', 'ingilizce'],
    run: () => {
      useSettings.getState().set('general.language', 'en');
    },
  },
  {
    id: 'settings.open',
    title: 'actions.openSettings',
    group: 'settings',
    shortcut: 'mod+,',
    keywords: ['preferences', 'tercihler', 'ayarlar'],
    run: () => useUi.getState().setSettingsOpen(true),
  },
  ...paneFocus,
  ...tabSelectors,
];

export const actionById = (id: string): AppAction | undefined => ACTIONS.find((a) => a.id === id);

export const isEnabled = (a: AppAction): boolean => a.enabled?.() ?? true;

export function runAction(id: string): void {
  const a = actionById(id);
  if (a && isEnabled(a)) a.run();
}
