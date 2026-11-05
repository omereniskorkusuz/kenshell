/**
 * Tab strip: one tab per live session; 2px mode-colored top edge + env letter.
 *
 * Dragging a tab (pointer events, not HTML5 drag-and-drop, which the webview's file-drop
 * handling can swallow): along the strip → reorder; onto a pane → split view (edge) or
 * replace (center); out of the window → move to a new window (P2-T19).
 */
import { Plus, Radio, X } from 'lucide-react';
import {
  useEffect,
  useRef,
  useState,
  type KeyboardEvent,
  type PointerEvent as ReactPointerEvent,
} from 'react';
import { useT } from '../i18n';
import { canDetach, isMac } from '../lib/ipc';
import { tabStatus } from '../lib/sessionState';
import { formatChord } from '../lib/shortcuts';
import { zoneAt, type DropZone } from '../lib/split';
import { envKind, envLabel, envLetter } from '../lib/tree';
import { useTabs } from '../stores/tabs';
import { useUi, type TabDrag } from '../stores/ui';
import { Menu, useMenu, type MenuEntry } from './Menu';

/** Pointer travel before a press becomes a drag. */
const DRAG_START_PX = 6;

interface DragState {
  key: string;
  pointerId: number;
  startX: number;
  startY: number;
  active: boolean;
}

/** The visible pane under a point and the drop zone inside it. */
function paneAt(x: number, y: number): { key: string; zone: DropZone } | null {
  for (const el of document.querySelectorAll<HTMLElement>('.pane[data-pane]:not([hidden])')) {
    const r = el.getBoundingClientRect();
    if (x >= r.left && x <= r.right && y >= r.top && y <= r.bottom && el.dataset.pane) {
      return { key: el.dataset.pane, zone: zoneAt(r, x, y) };
    }
  }
  return null;
}

function sameDrag(a: TabDrag | null, b: TabDrag): boolean {
  return a?.key === b.key && a.pane === b.pane && a.zone === b.zone && a.outside === b.outside;
}

export function TabBar() {
  const t = useT();
  const tabs = useTabs((s) => s.tabs);
  const activeKey = useTabs((s) => s.activeKey);
  const view = useTabs((s) => s.view);
  const broadcast = useTabs((s) => s.broadcast);
  const mac = isMac();
  const listRef = useRef<HTMLDivElement>(null);
  const drag = useRef<DragState | null>(null);
  const suppressClick = useRef(false);
  const menu = useMenu();
  const [menuKey, setMenuKey] = useState<string | null>(null);

  const setDrag = (next: TabDrag | null) => {
    const ui = useUi.getState();
    if (next === null ? ui.tabDrag === null : sameDrag(ui.tabDrag, next)) return;
    if (next?.outside && !ui.tabDrag?.outside) ui.flash(t('tabs.dropOutside'));
    ui.setTabDrag(next);
  };

  // Window listeners for the press in progress (moves can leave the strip at once).
  const listening = useRef<(() => void) | null>(null);
  const endDrag = () => {
    listening.current?.();
    listening.current = null;
    drag.current = null;
    setDrag(null);
  };
  useEffect(() => () => listening.current?.(), []);

  const onPointerDown = (e: ReactPointerEvent<HTMLDivElement>, key: string) => {
    if (e.button !== 0 || (e.target as HTMLElement).closest('button')) return;
    listening.current?.();
    drag.current = {
      key,
      pointerId: e.pointerId,
      startX: e.clientX,
      startY: e.clientY,
      active: false,
    };
    const move = (ev: PointerEvent) => onPointerMove(ev);
    const up = (ev: PointerEvent) => onPointerUp(ev);
    window.addEventListener('pointermove', move);
    window.addEventListener('pointerup', up);
    window.addEventListener('pointercancel', endDrag);
    listening.current = () => {
      window.removeEventListener('pointermove', move);
      window.removeEventListener('pointerup', up);
      window.removeEventListener('pointercancel', endDrag);
    };
  };

  const onPointerMove = (e: PointerEvent) => {
    const d = drag.current;
    if (!d || e.pointerId !== d.pointerId) return;
    if (!d.active) {
      if (Math.hypot(e.clientX - d.startX, e.clientY - d.startY) < DRAG_START_PX) return;
      d.active = true;
      // Keep receiving moves outside the window. Captured by the strip, which never moves
      // (tab nodes are re-ordered while dragging); captured events still bubble to window.
      try {
        listRef.current?.setPointerCapture(e.pointerId);
      } catch {
        // Pointer already released.
      }
    }
    const strip = listRef.current?.getBoundingClientRect();
    const { clientX: x, clientY: y } = e;
    const outside = x < 0 || y < 0 || x > window.innerWidth || y > window.innerHeight;
    if (outside) {
      setDrag({ key: d.key, pane: null, zone: null, outside: canDetach() });
      return;
    }
    if (strip && y <= strip.bottom + 6) {
      const others = Array.from(
        listRef.current?.querySelectorAll<HTMLElement>('[data-tab]') ?? [],
      ).filter((el) => el.dataset.tab !== d.key);
      let to = others.findIndex((el) => {
        const r = el.getBoundingClientRect();
        return x < r.left + r.width / 2;
      });
      if (to < 0) to = others.length;
      useTabs.getState().move(d.key, to);
      setDrag({ key: d.key, pane: null, zone: null, outside: false });
      return;
    }
    const pane = paneAt(x, y);
    setDrag({ key: d.key, pane: pane?.key ?? null, zone: pane?.zone ?? null, outside: false });
  };

  const onPointerUp = (e: PointerEvent) => {
    const d = drag.current;
    if (!d || e.pointerId !== d.pointerId) return;
    const target = useUi.getState().tabDrag;
    endDrag();
    if (!d.active) return;
    // The click that may follow this release is not a tab selection (cleared right after,
    // because with the strip capturing the pointer the click may not reach a tab at all).
    suppressClick.current = true;
    setTimeout(() => (suppressClick.current = false), 0);
    if (target?.outside) {
      void useTabs.getState().detach(d.key, { x: e.screenX, y: e.screenY });
    } else if (target?.pane && target.zone) {
      useTabs.getState().dropTab(d.key, target.pane, target.zone);
    }
  };

  const menuEntries = (key: string): MenuEntry[] => {
    const st = useTabs.getState();
    const other = key === activeKey ? undefined : key;
    const entries: MenuEntry[] = [
      {
        label: t('actions.splitRight'),
        hint: formatChord('mod+shift+e', mac),
        onSelect: () => st.splitAdd('cols', other),
      },
      {
        label: t('actions.splitDown'),
        hint: formatChord('mod+shift+o', mac),
        onSelect: () => st.splitAdd('rows', other),
      },
    ];
    if (view.keys.includes(key)) {
      entries.push({ label: t('tabs.removeFromSplit'), onSelect: () => st.unsplit(key) });
    }
    if (canDetach()) {
      entries.push({
        label: t('actions.detachTab'),
        hint: formatChord('mod+shift+n', mac),
        onSelect: () => void st.detach(key),
      });
    }
    entries.push(
      { kind: 'separator' },
      {
        label: t('actions.toggleBroadcast'),
        hint: formatChord('mod+alt+b', mac),
        checked: st.broadcast.includes(key),
        onSelect: () => st.toggleBroadcast(key),
      },
      { label: t('actions.broadcastAll'), onSelect: () => st.broadcastAll() },
      ...(st.broadcast.length > 0
        ? [{ label: t('actions.stopBroadcast'), onSelect: () => st.stopBroadcast() }]
        : []),
      { kind: 'separator' },
      {
        label: t('actions.closeTab'),
        hint: formatChord('mod+w', mac),
        onSelect: () => st.close(key),
      },
    );
    return entries;
  };

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
        ref={listRef}
        role="tablist"
        aria-label={t('tabs.label')}
        aria-description={t('tabs.dragHint')}
        className="scroll-thin flex min-w-0 overflow-x-auto"
        onKeyDown={onKeyDown}
        onLostPointerCapture={() => {
          if (drag.current?.active) endDrag();
        }}
      >
        {tabs.map((tab) => {
          const selected = tab.key === activeKey;
          const status = tabStatus(tab, t);
          const env = envKind(tab.envTag);
          const inView = view.keys.includes(tab.key);
          const casting = broadcast.includes(tab.key);
          return (
            <div
              key={tab.key}
              role="tab"
              id={`tab-${tab.key}`}
              data-tab={tab.key}
              aria-selected={selected}
              aria-controls={`pane-${tab.key}`}
              aria-label={`${tab.name}, ${status.text}${inView ? `, ${t('tabs.inView')}` : ''}${
                casting ? `, ${t('tabs.broadcasting')}` : ''
              }`}
              tabIndex={selected ? 0 : -1}
              data-mode={tab.mode}
              data-vendor={tab.device.vendor?.family}
              data-env={env ?? undefined}
              data-state={tab.state}
              data-in-view={inView || undefined}
              data-broadcast={casting || undefined}
              className="tab cursor-default"
              title={`${tab.name} — ${status.text}`}
              onPointerDown={(e) => onPointerDown(e, tab.key)}
              onClick={() => {
                if (suppressClick.current) {
                  suppressClick.current = false;
                  return;
                }
                useTabs.getState().activate(tab.key);
              }}
              onContextMenu={(e) => {
                e.preventDefault();
                setMenuKey(tab.key);
                menu.openAt(e.clientX, e.clientY);
              }}
              onAuxClick={(e) => {
                if (e.button === 1) useTabs.getState().close(tab.key);
              }}
            >
              <span className="status-dot" data-tone={status.tone} aria-hidden />
              {casting && (
                <Radio
                  size={12}
                  className="shrink-0 text-warning"
                  aria-hidden
                  data-testid="broadcast-mark"
                />
              )}
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
      {menu.at && menuKey && (
        <Menu
          x={menu.at.x}
          y={menu.at.y}
          entries={menuEntries(menuKey)}
          onClose={menu.close}
          label={t('tabs.menu')}
        />
      )}
    </div>
  );
}
