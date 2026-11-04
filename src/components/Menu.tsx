/**
 * Small accessible menu (context menu / dropdown): arrow keys, Home/End, Enter, Esc,
 * closes on outside click. Positioned at a point and kept inside the viewport.
 */
import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type KeyboardEvent,
  type ReactNode,
} from 'react';
import { createPortal } from 'react-dom';

export type MenuEntry =
  | {
      kind?: 'item';
      label: string;
      icon?: ReactNode;
      hint?: string;
      disabled?: boolean;
      danger?: boolean;
      checked?: boolean;
      onSelect: () => void;
    }
  | { kind: 'separator' }
  | { kind: 'label'; label: string };

interface Props {
  x: number;
  y: number;
  entries: MenuEntry[];
  onClose: () => void;
  label: string;
}

export function Menu({ x, y, entries, onClose, label }: Props) {
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState({ left: x, top: y });

  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const r = el.getBoundingClientRect();
    const left = Math.max(4, Math.min(x, window.innerWidth - r.width - 4));
    const top = y + r.height > window.innerHeight - 4 ? Math.max(4, y - r.height) : y;
    setPos({ left, top });
    el.querySelector<HTMLElement>('[role^="menuitem"]:not([aria-disabled="true"])')?.focus();
  }, [x, y]);

  // Return focus to whatever opened the menu.
  const [opener] = useState(() => document.activeElement as HTMLElement | null);
  useEffect(() => {
    const menu = ref.current;
    return () => {
      const ae = document.activeElement;
      if (opener?.isConnected && (!ae || ae === document.body || menu?.contains(ae))) {
        opener.focus();
      }
    };
  }, [opener]);

  useEffect(() => {
    const onDown = (e: PointerEvent) => {
      if (!ref.current?.contains(e.target as Node)) onClose();
    };
    const onBlur = () => onClose();
    window.addEventListener('pointerdown', onDown, true);
    window.addEventListener('blur', onBlur);
    window.addEventListener('resize', onBlur);
    return () => {
      window.removeEventListener('pointerdown', onDown, true);
      window.removeEventListener('blur', onBlur);
      window.removeEventListener('resize', onBlur);
    };
  }, [onClose]);

  const onKeyDown = (e: KeyboardEvent) => {
    const items = Array.from(
      ref.current?.querySelectorAll<HTMLElement>(
        '[role^="menuitem"]:not([aria-disabled="true"])',
      ) ?? [],
    );
    const i = items.indexOf(document.activeElement as HTMLElement);
    const focus = (n: number) => items[(n + items.length) % items.length]?.focus();
    switch (e.key) {
      case 'ArrowDown':
        focus(i + 1);
        break;
      case 'ArrowUp':
        focus(i - 1);
        break;
      case 'Home':
        focus(0);
        break;
      case 'End':
        focus(items.length - 1);
        break;
      case 'Escape':
      case 'Tab':
        onClose();
        break;
      default:
        return;
    }
    e.preventDefault();
    e.stopPropagation();
  };

  return createPortal(
    <div
      ref={ref}
      role="menu"
      aria-label={label}
      className="elevated scroll-thin fixed max-h-[70vh] min-w-44 overflow-y-auto py-1 text-sm"
      style={{ left: pos.left, top: pos.top, zIndex: 'var(--z-palette)' }}
      onKeyDown={onKeyDown}
      onContextMenu={(e) => e.preventDefault()}
    >
      {entries.map((entry, i) => {
        if (entry.kind === 'separator') {
          return <div key={i} role="separator" className="my-1 border-t border-subtle" />;
        }
        if (entry.kind === 'label') {
          return (
            <div key={i} className="label-caps px-3 pt-2 pb-1">
              {entry.label}
            </div>
          );
        }
        return (
          <button
            key={i}
            type="button"
            role={entry.checked === undefined ? 'menuitem' : 'menuitemradio'}
            aria-checked={entry.checked}
            aria-disabled={entry.disabled || undefined}
            tabIndex={-1}
            className={`mx-1 flex h-7 w-[calc(100%-8px)] items-center gap-2 rounded-sm px-2 text-left hover:bg-hover focus:bg-hover focus:shadow-none ${
              entry.disabled ? 'text-muted' : entry.danger ? 'text-danger' : 'text-primary'
            }`}
            onClick={() => {
              if (entry.disabled) return;
              onClose();
              entry.onSelect();
            }}
          >
            <span className="flex w-4 justify-center text-secondary">
              {entry.checked ? '✓' : entry.icon}
            </span>
            <span className="flex-1 truncate">{entry.label}</span>
            {entry.hint && <span className="kbd border-0">{entry.hint}</span>}
          </button>
        );
      })}
    </div>,
    document.body,
  );
}

/** Helper state for "open a menu at the pointer / under an element". */
export function useMenu() {
  const [at, setAt] = useState<{ x: number; y: number } | null>(null);
  const openAt = useCallback((x: number, y: number) => setAt({ x, y }), []);
  const openBelow = useCallback((el: Element) => {
    const r = el.getBoundingClientRect();
    setAt({ x: r.left, y: r.bottom + 2 });
  }, []);
  const close = useCallback(() => setAt(null), []);
  return { at, openAt, openBelow, close };
}
