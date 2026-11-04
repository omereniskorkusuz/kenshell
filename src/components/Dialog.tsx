/**
 * Modal sheet for forms/views (session editor, settings). Not a confirmation dialog: the app
 * never asks "are you sure?". Esc and the backdrop close it; Tab is kept inside.
 */
import { useEffect, useId, useRef, useState, type KeyboardEvent, type ReactNode } from 'react';
import { createPortal } from 'react-dom';
import { X } from 'lucide-react';
import { useT } from '../i18n';

interface Props {
  title: string;
  onClose: () => void;
  children: ReactNode;
  footer?: ReactNode;
  width?: string;
}

const FOCUSABLE =
  'a[href],button:not([disabled]),input:not([disabled]),select:not([disabled]),textarea:not([disabled]),[tabindex]:not([tabindex="-1"])';

export function Dialog({ title, onClose, children, footer, width = 'var(--editor-width)' }: Props) {
  const t = useT();
  const ref = useRef<HTMLDivElement>(null);
  const titleId = useId();
  const [opener] = useState(() => document.activeElement as HTMLElement | null);

  useEffect(() => {
    const el = ref.current;
    const first =
      el?.querySelector<HTMLElement>('[data-autofocus]') ??
      el?.querySelector<HTMLElement>(FOCUSABLE);
    first?.focus();
    return () => {
      if (opener?.isConnected) opener.focus();
    };
  }, [opener]);

  const onKeyDown = (e: KeyboardEvent) => {
    if (e.key === 'Escape') {
      e.stopPropagation();
      onClose();
      return;
    }
    if (e.key !== 'Tab') return;
    const items = Array.from(ref.current?.querySelectorAll<HTMLElement>(FOCUSABLE) ?? []).filter(
      (x) => x.offsetParent !== null,
    );
    if (items.length === 0) return;
    const first = items[0] as HTMLElement;
    const last = items[items.length - 1] as HTMLElement;
    if (e.shiftKey && document.activeElement === first) {
      e.preventDefault();
      last.focus();
    } else if (!e.shiftKey && document.activeElement === last) {
      e.preventDefault();
      first.focus();
    }
  };

  return createPortal(
    <div
      className="fixed inset-0 flex items-start justify-center bg-app/60 pt-[8vh]"
      style={{ zIndex: 'var(--z-overlay)' }}
      onPointerDown={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div
        ref={ref}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        className="elevated flex max-h-[84vh] max-w-[calc(100vw-32px)] flex-col"
        style={{ width }}
        onKeyDown={onKeyDown}
      >
        <header className="flex h-10 shrink-0 items-center gap-2 border-b border-subtle pr-2 pl-4">
          <h2 id={titleId} className="flex-1 truncate font-medium">
            {title}
          </h2>
          <button
            type="button"
            className="icon-btn"
            aria-label={t('common.close')}
            onClick={onClose}
          >
            <X size={16} />
          </button>
        </header>
        <div className="scroll-thin min-h-0 flex-1 overflow-y-auto">{children}</div>
        {footer && (
          <footer className="flex shrink-0 items-center justify-end gap-2 border-t border-subtle px-4 py-3">
            {footer}
          </footer>
        )}
      </div>
    </div>,
    document.body,
  );
}
