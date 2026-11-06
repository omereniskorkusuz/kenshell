/**
 * System Diagnostics (P4-T06), in the analyzer panel: runs the vendor's read-only command
 * set hidden from the terminal (one by one, at the device's prompt), then lists findings
 * by severity. Never sends a configuration command (the backend refuses anything but
 * show/display).
 */
import { ClipboardCopy, RotateCw, X } from 'lucide-react';
import { useEffect, useRef, useState } from 'react';
import { useT, type TKey } from '../i18n';
import { describeError } from '../lib/errors';
import { api, type CommandOutput, type Finding } from '../lib/ipc';
import type { LiveTab } from '../lib/sessionState';
import { useUi } from '../stores/ui';
import { FindingList, useFindingText } from './Findings';

/** Per-command time limit (a slow `show interfaces` on a big chassis). */
const TIMEOUT_SECS = 90;

export function Diagnostics({ tab, onClose }: { tab: LiveTab; onClose: () => void }) {
  const t = useT();
  const [progress, setProgress] = useState<{ done: number; total: number; command: string } | null>(
    null,
  );
  const [findings, setFindings] = useState<Finding[] | null>(null);
  const [outputs, setOutputs] = useState<CommandOutput[]>([]);
  const [error, setError] = useState<string | null>(null);
  const cancelled = useRef(false);
  const family = tab.device.vendor?.family ?? null;
  const runtimeId = tab.runtimeId;

  const run = async () => {
    if (runtimeId === null) return;
    cancelled.current = false;
    setFindings(null);
    setError(null);
    setOutputs([]);
    try {
      const commands = await api.diagnosticsCommands(family);
      const collected: CommandOutput[] = [];
      for (const [i, command] of commands.entries()) {
        if (cancelled.current) return;
        setProgress({ done: i, total: commands.length, command });
        const text = await api.liveCapture(runtimeId, command, TIMEOUT_SECS).catch(() => null);
        collected.push({ command, text });
        setOutputs([...collected]);
      }
      if (cancelled.current) return;
      setFindings(await api.diagnosticsEvaluate(family, collected));
    } catch (e) {
      setError(describeError(e, t).message);
    } finally {
      setProgress(null);
    }
  };

  useEffect(() => {
    queueMicrotask(() => void run());
    return () => {
      cancelled.current = true;
    };
    // Runs once per opening (the tab is fixed while the view is open).
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const message = useFindingText('diag');
  const copyReport = () => {
    const lines = [
      `${t('diag.title')} · ${tab.device.hostname ?? tab.name} · ${new Date().toLocaleString()}`,
      ...(findings ?? []).map((f) => `[${t(`diag.sev.${f.severity}` as TKey)}] ${message(f)}`),
    ];
    void api
      .clipboardWrite(lines.join('\n'))
      .then(() => useUi.getState().flash(t('diag.copied')))
      .catch(() => {});
  };

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex items-center gap-1 border-b border-subtle p-2">
        <span className="min-w-0 flex-1 truncate text-sm font-medium">
          {t('diag.title')} · <span className="mono">{tab.device.hostname ?? tab.name}</span>
        </span>
        <button
          type="button"
          className="icon-btn icon-btn-sm"
          disabled={progress !== null || tab.state !== 'connected'}
          aria-label={t('diag.rerun')}
          title={t('diag.rerun')}
          onClick={() => void run()}
        >
          <RotateCw size={13} />
        </button>
        <button
          type="button"
          className="icon-btn icon-btn-sm"
          disabled={!findings}
          aria-label={t('diag.copy')}
          title={t('diag.copy')}
          onClick={copyReport}
        >
          <ClipboardCopy size={13} />
        </button>
        <button
          type="button"
          className="icon-btn icon-btn-sm"
          aria-label={t('common.close')}
          onClick={() => {
            cancelled.current = true;
            onClose();
          }}
        >
          <X size={13} />
        </button>
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto p-2">
        <p className="mb-2 text-xs text-muted">{t('diag.readOnly')}</p>
        {progress && (
          <div className="mb-2" role="status">
            <div className="mb-1 flex justify-between text-xs text-secondary">
              <span className="mono truncate">{progress.command}</span>
              <span>
                {progress.done + 1}/{progress.total}
              </span>
            </div>
            <div className="h-1 overflow-hidden rounded-full bg-hover">
              <div
                className="h-full bg-accent transition-[width]"
                style={{ width: `${((progress.done + 0.5) / progress.total) * 100}%` }}
              />
            </div>
          </div>
        )}
        {error && <p className="text-sm text-danger">{error}</p>}
        {findings && <FindingList findings={findings} prefix="diag" />}
        {outputs.length > 0 && (
          <details className="mt-3">
            <summary className="cursor-default text-xs text-secondary">{t('diag.outputs')}</summary>
            {outputs.map((o) => (
              <details key={o.command} className="mt-1">
                <summary className="mono cursor-default text-xs">{o.command}</summary>
                <pre className="mono max-h-60 overflow-auto rounded-sm bg-terminal p-2 text-[11px]">
                  {o.text ?? t('diag.noOutput')}
                </pre>
              </details>
            ))}
          </details>
        )}
      </div>
    </div>
  );
}
