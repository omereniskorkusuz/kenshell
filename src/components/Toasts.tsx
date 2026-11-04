/**
 * Toast stack: bottom-right above the status bar, silent, 8 s by default. Hover or keyboard
 * focus pauses the timer; reduced motion disables the entrance animation (token).
 */
import { useEffect, useRef, useState } from 'react';
import { X } from 'lucide-react';
import { useT } from '../i18n';
import { useToasts, type Toast } from '../stores/toasts';

export function Toasts() {
  const toasts = useToasts((s) => s.toasts);
  return (
    <div
      aria-live="polite"
      aria-relevant="additions"
      className="pointer-events-none fixed right-3 flex flex-col items-end gap-2"
      style={{ bottom: 'calc(var(--status-bar-height) + 12px)', zIndex: 'var(--z-toast)' }}
    >
      {toasts.map((x) => (
        <ToastView key={x.id} toast={x} />
      ))}
    </div>
  );
}

function ToastView({ toast }: { toast: Toast }) {
  const t = useT();
  const [paused, setPaused] = useState(false);
  const remaining = useRef(toast.durationMs);
  const startedAt = useRef(0);

  useEffect(() => {
    if (paused) return;
    startedAt.current = Date.now();
    const id = setTimeout(() => useToasts.getState().dismiss(toast.id), remaining.current);
    return () => {
      clearTimeout(id);
      remaining.current = Math.max(1500, remaining.current - (Date.now() - startedAt.current));
    };
  }, [paused, toast.id]);

  const dismiss = () => useToasts.getState().dismiss(toast.id);

  return (
    <div
      role={toast.kind === 'error' ? 'alert' : 'status'}
      className="toast elevated pointer-events-auto flex items-start gap-2 py-2 pr-1 pl-3 text-sm"
      style={{ width: 'var(--toast-width)', maxWidth: 'calc(100vw - 24px)' }}
      title={toast.detail ?? undefined}
      onMouseEnter={() => setPaused(true)}
      onMouseLeave={() => setPaused(false)}
      onFocus={() => setPaused(true)}
      onBlur={(e) => {
        if (!e.currentTarget.contains(e.relatedTarget as Node | null)) setPaused(false);
      }}
      onKeyDown={(e) => {
        if (e.key === 'Escape') dismiss();
      }}
    >
      {toast.kind === 'error' && (
        <span className="status-dot mt-1.5" data-tone="error" aria-hidden />
      )}
      <div className="min-w-0 flex-1 py-0.5">
        <p className="break-words text-primary">{toast.message}</p>
        {toast.actions.length > 0 && (
          <div className="mt-2 flex flex-wrap gap-1">
            {toast.actions.map((a) => (
              <button
                key={a.label}
                type="button"
                className={`btn btn-sm ${a.primary ? 'btn-primary' : ''}`}
                onClick={() => {
                  dismiss();
                  a.run();
                }}
              >
                {a.label}
              </button>
            ))}
          </div>
        )}
      </div>
      <button
        type="button"
        className="icon-btn icon-btn-sm"
        aria-label={t('common.dismiss')}
        onClick={dismiss}
      >
        <X size={12} />
      </button>
    </div>
  );
}
