/** Tab strip: one tab per live session; 2px mode-colored top edge + env letter. */
import { Plus, X } from 'lucide-react';
import type { KeyboardEvent } from 'react';
import { useT } from '../i18n';
import { isMac } from '../lib/ipc';
import { tabStatus } from '../lib/sessionState';
import { formatChord } from '../lib/shortcuts';
import { envKind, envLabel, envLetter } from '../lib/tree';
import { useTabs } from '../stores/tabs';
import { useUi } from '../stores/ui';

export function TabBar() {
  const t = useT();
  const tabs = useTabs((s) => s.tabs);
  const activeKey = useTabs((s) => s.activeKey);
  const mac = isMac();

  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    const i = tabs.findIndex((x) => x.key === activeKey);
    let next: number | null = null;
    if (e.key === 'ArrowRight') next = (i + 1) % tabs.length;
    else if (e.key === 'ArrowLeft') next = (i - 1 + tabs.length) % tabs.length;
    else if (e.key === 'Home') next = 0;
    else if (e.key === 'End') next = tabs.length - 1;
    else if (e.key === 'Delete' && activeKey) {
      useTabs.getState().close(activeKey);
      e.preventDefault();
      return;
    }
    const target = next === null ? undefined : tabs[next];
    if (!target) return;
    e.preventDefault();
    const key = target.key;
    useTabs.getState().activate(key);
    e.currentTarget.querySelector<HTMLElement>(`[data-tab="${key}"]`)?.focus();
  };

  return (
    <div className="flex shrink-0 items-stretch border-b border-subtle bg-surface">
      <div
        role="tablist"
        aria-label={t('tabs.label')}
        className="scroll-thin flex min-w-0 overflow-x-auto"
        onKeyDown={onKeyDown}
      >
        {tabs.map((tab) => {
          const selected = tab.key === activeKey;
          const status = tabStatus(tab, t);
          const env = envKind(tab.envTag);
          return (
            <div
              key={tab.key}
              role="tab"
              id={`tab-${tab.key}`}
              data-tab={tab.key}
              aria-selected={selected}
              aria-controls={`pane-${tab.key}`}
              aria-label={`${tab.name}, ${status.text}`}
              tabIndex={selected ? 0 : -1}
              data-mode={tab.mode}
              data-env={env ?? undefined}
              data-state={tab.state}
              className="tab cursor-default"
              title={`${tab.name} — ${status.text}`}
              onClick={() => useTabs.getState().activate(tab.key)}
              onAuxClick={(e) => {
                if (e.button === 1) useTabs.getState().close(tab.key);
              }}
            >
              <span className="status-dot" data-tone={status.tone} aria-hidden />
              <span className="tab-title mono min-w-0 flex-1 truncate">{tab.name}</span>
              {env && (
                <span className="env-letter" title={envLabel(tab.envTag) ?? ''}>
                  {envLetter(tab.envTag)}
                </span>
              )}
              <button
                type="button"
                tabIndex={-1}
                className="icon-btn icon-btn-sm"
                aria-label={t('tabs.closeTab', { name: tab.name })}
                title={`${t('actions.closeTab')} (${formatChord('mod+w', mac)})`}
                onClick={(e) => {
                  e.stopPropagation();
                  useTabs.getState().close(tab.key);
                }}
              >
                <X size={12} />
              </button>
            </div>
          );
        })}
      </div>
      <button
        type="button"
        className="icon-btn m-0.5 self-center"
        aria-label={t('quick.label')}
        title={`${t('quick.label')} (${formatChord('mod+l', mac)})`}
        onClick={() => useUi.getState().focusQuickConnect()}
      >
        <Plus size={14} />
      </button>
    </div>
  );
}
