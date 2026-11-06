/**
 * show tech / crashinfo / log file analysis (P4-T09): the file is streamed by the backend
 * (never loaded whole); this view shows the summary, findings, sections (open one to read
 * it) and the most frequent syslog messages.
 */
import { ArrowLeft, FileSearch, X } from 'lucide-react';
import { useState } from 'react';
import { useT, type TKey } from '../i18n';
import { describeError } from '../lib/errors';
import { formatBytes } from '../lib/format';
import { api, type FileSummary } from '../lib/ipc';
import { FindingList } from './Findings';

export function FileAnalysis({ onClose }: { onClose: () => void }) {
  const t = useT();
  const [path, setPath] = useState<string | null>(null);
  const [summary, setSummary] = useState<FileSummary | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [section, setSection] = useState<{ title: string; text: string } | null>(null);

  const pick = async () => {
    const p = await api.pickPath('file', ['txt', 'log', 'tech', 'out']).catch(() => null);
    if (!p) return;
    setPath(p);
    setSummary(null);
    setSection(null);
    setError(null);
    setBusy(true);
    try {
      setSummary(await api.logfileAnalyze(p));
    } catch (e) {
      setError(describeError(e, t).message);
    } finally {
      setBusy(false);
    }
  };

  const open = async (title: string, start: number, end: number) => {
    if (!path) return;
    try {
      setSection({ title, text: await api.logfileLines(path, start, end) });
    } catch (e) {
      setError(describeError(e, t).message);
    }
  };

  const name = path?.split(/[\\/]/).pop() ?? '';

  if (section) {
    return (
      <div className="flex min-h-0 flex-1 flex-col">
        <div className="flex items-center gap-1 border-b border-subtle p-2">
          <button
            type="button"
            className="icon-btn icon-btn-sm"
            aria-label={t('backups.back')}
            onClick={() => setSection(null)}
          >
            <ArrowLeft size={13} />
          </button>
          <span className="mono min-w-0 flex-1 truncate text-sm">{section.title}</span>
        </div>
        <pre className="mono min-h-0 flex-1 overflow-auto p-2 text-[11px] leading-5">
          {section.text}
        </pre>
      </div>
    );
  }

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex items-center gap-1 border-b border-subtle p-2">
        <span className="min-w-0 flex-1 truncate text-sm font-medium">
          {t('fileAnalysis.title')}
        </span>
        <button
          type="button"
          className="btn btn-primary h-7 text-xs"
          disabled={busy}
          onClick={() => void pick()}
        >
          <FileSearch size={13} aria-hidden /> {t('fileAnalysis.pick')}
        </button>
        <button
          type="button"
          className="icon-btn icon-btn-sm"
          aria-label={t('common.close')}
          onClick={onClose}
        >
          <X size={13} />
        </button>
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto p-2">
        {!summary && !busy && !error && (
          <p className="text-sm text-muted">{t('fileAnalysis.hint')}</p>
        )}
        {busy && (
          <p className="text-sm text-secondary" role="status">
            {t('fileAnalysis.reading', { name })}
          </p>
        )}
        {error && <p className="text-sm text-danger">{error}</p>}
        {summary && (
          <div className="flex flex-col gap-3">
            <div>
              <p className="mono truncate text-sm" title={path ?? ''}>
                {name}
              </p>
              <p className="text-xs text-muted">
                {[
                  t(`fileAnalysis.kind.${summary.kind}` as TKey),
                  t('backups.lines', { count: summary.lines }),
                  formatBytes(summary.bytes),
                  ...summary.facts.map(([, v]) => v),
                ].join(' · ')}
              </p>
            </div>
            <FindingList findings={summary.findings} prefix="diag" />
            {summary.sections.length > 0 && (
              <section>
                <h3 className="mb-1 text-xs font-medium text-secondary">
                  {t('fileAnalysis.sections', { count: summary.sections.length })}
                </h3>
                <ul className="max-h-72 overflow-y-auto">
                  {summary.sections.map((s) => (
                    <li key={`${s.start}`}>
                      <button
                        type="button"
                        className="flex w-full items-center gap-2 rounded-sm px-1 py-0.5 text-left hover:bg-hover"
                        onClick={() => void open(s.title, s.start, s.end)}
                      >
                        <span className="mono min-w-0 flex-1 truncate text-xs">{s.title}</span>
                        <span className="shrink-0 text-[11px] text-muted">
                          {t('backups.lines', { count: s.end - s.start + 1 })}
                        </span>
                      </button>
                    </li>
                  ))}
                </ul>
              </section>
            )}
            {summary.messages.length > 0 && (
              <section>
                <h3 className="mb-1 text-xs font-medium text-secondary">
                  {t('fileAnalysis.messages')}
                </h3>
                <ul>
                  {summary.messages.map((m) => (
                    <li key={m.mnemonic} className="flex items-center gap-2 px-1 py-0.5 text-xs">
                      <span
                        className="tone-text mono w-4 text-center"
                        data-tone={
                          m.severity <= 2 ? 'error' : m.severity === 3 ? 'warning' : 'muted'
                        }
                      >
                        {m.severity}
                      </span>
                      <button
                        type="button"
                        className="mono min-w-0 flex-1 truncate text-left hover:underline"
                        title={t('fileAnalysis.firstAt', { line: m.firstLine })}
                        onClick={() =>
                          void open(m.mnemonic, Math.max(1, m.firstLine - 5), m.firstLine + 30)
                        }
                      >
                        {m.mnemonic}
                      </button>
                      <span className="tabular text-muted">{m.count}</span>
                    </li>
                  ))}
                </ul>
              </section>
            )}
          </div>
        )}
      </div>
    </div>
  );
}
