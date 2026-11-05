/**
 * Right side panel (Ctrl/Cmd+J). Below 1600px window width it overlays the terminal (the PTY
 * is not resized, so `show run` is not re-wrapped); at ≥ 1600px, or when pinned, it pushes.
 */
import { useSyncExternalStore, type KeyboardEvent } from 'react';
import { BookText, Pin, PinOff, ScanSearch, SquareCode, Wrench, X } from 'lucide-react';
import { useT, type TKey } from '../i18n';
import { LIMITS, useSettings } from '../stores/settings';
import { useUi, type SidePanelTab } from '../stores/ui';
import { NotesPanel } from './NotesPanel';
import { SnippetsPanel } from './SnippetsPanel';
import { ResizeHandle } from './ResizeHandle';

export const PUSH_MIN_WIDTH = 1600;

function subscribeResize(cb: () => void) {
  window.addEventListener('resize', cb);
  return () => window.removeEventListener('resize', cb);
}
export const useWindowWidth = () =>
  useSyncExternalStore(
    subscribeResize,
    () => window.innerWidth,
    () => 1280,
  );

/** True when the panel should take layout space instead of floating over the terminal. */
export function usePanelPushes(): boolean {
  const width = useWindowWidth();
  const pinned = useSettings((s) => s.values['ui.sidePanelPinned']);
  return pinned || width >= PUSH_MIN_WIDTH;
}

const SEGMENTS: { id: SidePanelTab; label: TKey; empty: TKey; icon: typeof BookText }[] = [
  { id: 'notes', label: 'panel.notes', empty: 'panel.notesEmpty', icon: BookText },
  { id: 'analyzer', label: 'panel.analyzer', empty: 'panel.analyzerEmpty', icon: ScanSearch },
  { id: 'snippets', label: 'panel.snippets', empty: 'panel.snippetsEmpty', icon: SquareCode },
  { id: 'tools', label: 'panel.tools', empty: 'panel.toolsEmpty', icon: Wrench },
];

export function SidePanel() {
  const t = useT();
  const open = useUi((s) => s.sidePanelOpen);
  const active = useUi((s) => s.sidePanelTab);
  const pinned = useSettings((s) => s.values['ui.sidePanelPinned']);
  const pushes = usePanelPushes();
  if (!open) return null;

  const current = SEGMENTS.find((s) => s.id === active) ?? (SEGMENTS[0] as (typeof SEGMENTS)[0]);
  const Icon = current.icon;

  const onTabsKey = (e: KeyboardEvent<HTMLDivElement>) => {
    const i = SEGMENTS.findIndex((s) => s.id === active);
    let next = -1;
    if (e.key === 'ArrowRight') next = (i + 1) % SEGMENTS.length;
    else if (e.key === 'ArrowLeft') next = (i - 1 + SEGMENTS.length) % SEGMENTS.length;
    if (next < 0) return;
    e.preventDefault();
    const seg = SEGMENTS[next] as (typeof SEGMENTS)[0];
    useUi.getState().setSidePanelTab(seg.id);
    e.currentTarget.querySelector<HTMLElement>(`[data-seg="${seg.id}"]`)?.focus();
  };

  return (
    <aside
      aria-label={t('panel.label')}
      className={`side-panel flex shrink-0 flex-col border-l border-subtle bg-surface ${
        pushes ? 'relative' : 'elevated absolute top-0 right-0 bottom-0 rounded-none'
      }`}
      style={pushes ? undefined : { zIndex: 'var(--z-panel)' }}
      onKeyDown={(e) => {
        if (e.key === 'Escape' && !pushes) {
          e.stopPropagation();
          useUi.getState().toggleSidePanel(false);
        }
      }}
    >
      <div className="flex h-9 shrink-0 items-center gap-1 border-b border-subtle px-2">
        <div
          role="tablist"
          aria-label={t('panel.label')}
          className="flex flex-1 items-center gap-0.5 rounded-md bg-app p-0.5"
          onKeyDown={onTabsKey}
        >
          {SEGMENTS.map((s) => (
            <button
              key={s.id}
              type="button"
              role="tab"
              data-seg={s.id}
              aria-selected={s.id === active}
              tabIndex={s.id === active ? 0 : -1}
              className={`h-6 flex-1 truncate rounded-sm px-1 text-xs ${
                s.id === active ? 'bg-selected text-primary' : 'text-secondary hover:text-primary'
              }`}
              onClick={() => useUi.getState().setSidePanelTab(s.id)}
            >
              {t(s.label)}
            </button>
          ))}
        </div>
        <button
          type="button"
          className="icon-btn icon-btn-sm"
          aria-pressed={pinned}
          aria-label={pinned ? t('panel.unpin') : t('panel.pin')}
          title={pinned ? t('panel.unpin') : t('panel.pin')}
          onClick={() => useSettings.getState().set('ui.sidePanelPinned', !pinned)}
        >
          {pinned ? <PinOff size={13} /> : <Pin size={13} />}
        </button>
        <button
          type="button"
          className="icon-btn icon-btn-sm"
          aria-label={t('common.close')}
          onClick={() => useUi.getState().toggleSidePanel(false)}
        >
          <X size={14} />
        </button>
      </div>
      {current.id === 'notes' ? (
        <div role="tabpanel" className="flex min-h-0 flex-1 flex-col">
          <NotesPanel />
        </div>
      ) : current.id === 'snippets' ? (
        <div role="tabpanel" className="flex min-h-0 flex-1 flex-col">
          <SnippetsPanel />
        </div>
      ) : (
        <div
          role="tabpanel"
          className="flex flex-1 flex-col items-center justify-center gap-2 p-6 text-center"
        >
          <Icon size={22} className="text-muted" aria-hidden />
          <p className="font-medium text-secondary">{t(current.label)}</p>
          <p className="max-w-60 text-sm text-muted">{t(current.empty)}</p>
        </div>
      )}
      <ResizeHandle
        side="left"
        cssVar="--side-panel-width"
        settingKey="ui.sidePanelWidth"
        min={LIMITS.sidePanelWidth.min}
        max={LIMITS.sidePanelWidth.max}
        label={t('panel.resize')}
      />
    </aside>
  );
}
