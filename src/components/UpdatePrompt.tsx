/**
 * "New version available" card on the main window (top right, under the top bar).
 * Shows the new and current version and the release notes; "Update now" downloads and
 * restarts with a progress bar, "Later" hides it until the next start.
 */
import { X } from 'lucide-react';
import { useT } from '../i18n';
import { installUpdate } from '../app/updates';
import { useUpdate } from '../stores/update';
import { LogoMark } from './Logo';

export function UpdatePrompt() {
  const t = useT();
  const info = useUpdate((s) => s.info);
  const dismissed = useUpdate((s) => s.dismissed);
  const phase = useUpdate((s) => s.phase);
  const percent = useUpdate((s) => s.percent);
  const error = useUpdate((s) => s.error);
  if (!info || dismissed) return null;
  const busy = phase === 'downloading';

  return (
    <section
      role="dialog"
      aria-modal="false"
      aria-labelledby="update-title"
      className="update-card fixed right-3 flex w-80 flex-col gap-3 rounded-lg border border-strong bg-elevated p-4 text-sm"
      style={{
        // Top right, under the top bar: toasts own the bottom-right corner.
        top: 'calc(var(--top-bar-height) + 12px)',
        zIndex: 'var(--z-toast)',
        boxShadow: 'var(--shadow-elevated)',
      }}
    >
      <div className="flex items-start gap-3">
        <LogoMark size={36} className="shrink-0" />
        <div className="min-w-0 flex-1">
          <h2 id="update-title" className="font-medium text-primary">
            {t('updates.promptTitle')}
          </h2>
          <p className="text-xs text-muted">
            {t('updates.promptVersions', { version: info.version, current: info.current })}
          </p>
        </div>
        {!busy && (
          <button
            type="button"
            className="icon-btn icon-btn-sm"
            aria-label={t('updates.later')}
            title={t('updates.later')}
            onClick={() => useUpdate.getState().later()}
          >
            <X size={14} />
          </button>
        )}
      </div>
      {info.notes && (
        <p className="scroll-thin max-h-28 overflow-y-auto whitespace-pre-wrap text-xs text-secondary">
          {info.notes}
        </p>
      )}
      {busy && (
        <div className="flex flex-col gap-1">
          <div
            className="h-1.5 overflow-hidden rounded-full bg-hover"
            role="progressbar"
            aria-label={t('updates.downloadingBytes')}
            aria-valuemin={0}
            aria-valuemax={100}
            aria-valuenow={percent ?? undefined}
          >
            <div
              className="h-full bg-brand transition-[width]"
              style={{ width: `${percent ?? 100}%` }}
            />
          </div>
          <p className="text-xs text-muted">
            {percent === null
              ? t('updates.downloadingBytes')
              : t('updates.downloading', { percent })}
          </p>
        </div>
      )}
      {phase === 'error' && error && <p className="text-xs text-danger">{error}</p>}
      {!busy && (
        <div className="flex justify-end gap-2">
          <button
            type="button"
            className="btn btn-ghost btn-sm"
            onClick={() => useUpdate.getState().later()}
          >
            {t('updates.later')}
          </button>
          <button
            type="button"
            className="btn btn-primary btn-sm"
            onClick={() => void installUpdate()}
          >
            {phase === 'error' ? t('updates.retry') : t('updates.updateNow')}
          </button>
        </div>
      )}
    </section>
  );
}
