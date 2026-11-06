/**
 * Analyzer panel (P4-T01, lazy chunk): the last command's output (or the selection) of the
 * active tab as a sortable, filterable table, exported as CSV. When a command the analyzer
 * knows finishes in the active tab, its table appears automatically.
 */
import { Download, ScanSearch, TextSelect } from 'lucide-react';
import { useCallback, useEffect, useMemo, useState } from 'react';
import { useT, type TKey } from '../i18n';
import { filterRows, shownColumns, sortRows, statusTone, type SortState } from '../lib/analyzer';
import { describeError } from '../lib/errors';
import { api, type AnalyzerTable } from '../lib/ipc';
import { getTerminal } from '../terminal/registry';
import { useAnalyzer } from '../stores/analyzer';
import { useActiveTab } from '../stores/tabs';
import { toast } from '../stores/toasts';
import { useUi } from '../stores/ui';
import { VirtualTable } from './VirtualTable';

const WIDTH: Record<string, number> = { iface: 150, ip: 120, status: 90, number: 70, text: 80 };

export default function AnalyzerPanel() {
  const t = useT();
  const tab = useActiveTab();
  const done = useAnalyzer((s) => s.done);
  const parsers = useAnalyzer((s) => s.parsers);
  const [table, setTable] = useState<AnalyzerTable | null>(null);
  const [miss, setMiss] = useState<string | null>(null);
  const [query, setQuery] = useState('');
  const [sort, setSort] = useState<SortState | null>(null);

  useEffect(() => {
    void useAnalyzer.getState().loadParsers();
  }, []);

  const tabKey = tab?.key ?? null;
  const family = tab?.device.vendor?.family ?? null;
  const run = useCallback(
    async (source: 'last' | 'selection') => {
      if (!tabKey) return;
      const h = getTerminal(tabKey);
      if (!h) return;
      const input =
        source === 'last'
          ? h.lastOutput()
          : h.selection()
            ? { command: null, text: h.selection() }
            : null;
      if (!input) {
        setMiss(t('analyzer.noOutput'));
        return;
      }
      try {
        const result = await api.analyzeOutput(input.text, family, input.command);
        setTable(result);
        setSort(null);
        setMiss(result ? null : t('analyzer.unknown', { command: input.command ?? '' }));
      } catch (e) {
        const d = describeError(e, t);
        toast({ kind: 'error', message: d.message, detail: d.detail });
      }
    },
    [tabKey, family, t],
  );

  // A known command finished in the active tab: show it.
  useEffect(() => {
    if (done && done.tabKey === tabKey) queueMicrotask(() => void run('last'));
  }, [done, tabKey, run]);

  const shown = useMemo(() => {
    if (!table) return null;
    const cols = shownColumns(table.rows, table.columns.length);
    const rows = sortRows(filterRows(table.rows, query), table.columns, sort).map((r) =>
      cols.map((c) => r[c] ?? ''),
    );
    return { cols, rows };
  }, [table, query, sort]);

  const label = (key: string) => {
    const k = `analyzer.col.${key}` as TKey;
    const v = t(k);
    return v === k ? key : v;
  };

  const exportCsv = async () => {
    if (!table || !shown) return;
    const name = `${table.command.replace(/\s+/g, '_')}.csv`;
    const path = await api.pickPath('save', ['csv'], name).catch(() => null);
    if (!path) return;
    try {
      await api.analyzerExportCsv(
        path,
        shown.cols.map((c) => label(table.columns[c]?.key ?? '')),
        shown.rows,
      );
      useUi.getState().flash(t('analyzer.exported', { path }));
    } catch (e) {
      const d = describeError(e, t);
      toast({ kind: 'error', message: d.message, detail: d.detail });
    }
  };

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex items-center gap-1 border-b border-subtle p-2">
        <button
          type="button"
          className="btn btn-ghost h-7 text-xs"
          disabled={!tab}
          onClick={() => void run('last')}
        >
          <ScanSearch size={13} aria-hidden /> {t('analyzer.analyzeLast')}
        </button>
        <button
          type="button"
          className="icon-btn icon-btn-sm"
          disabled={!tab}
          aria-label={t('analyzer.analyzeSelection')}
          title={t('analyzer.analyzeSelection')}
          onClick={() => void run('selection')}
        >
          <TextSelect size={13} />
        </button>
        <span className="flex-1" />
        <button
          type="button"
          className="icon-btn icon-btn-sm"
          disabled={!shown?.rows.length}
          aria-label={t('analyzer.exportCsv')}
          title={t('analyzer.exportCsv')}
          onClick={() => void exportCsv()}
        >
          <Download size={13} />
        </button>
      </div>
      {table && shown ? (
        <>
          <div className="flex items-center gap-2 px-2 py-1.5">
            <span className="mono min-w-0 flex-1 truncate text-xs text-secondary">
              {table.command}
            </span>
            <span className="shrink-0 text-xs text-muted">
              {t('analyzer.rows', { count: shown.rows.length })}
              {table.truncated ? ` · ${t('analyzer.truncated')}` : ''}
            </span>
          </div>
          <div className="px-2 pb-1.5">
            <input
              className="field h-7 w-full text-sm"
              type="search"
              placeholder={t('analyzer.filter')}
              aria-label={t('analyzer.filter')}
              value={query}
              onChange={(e) => setQuery(e.target.value)}
            />
          </div>
          <VirtualTable
            label={table.command}
            columns={shown.cols.map((c) => {
              const col = table.columns[c];
              return { label: label(col?.key ?? ''), width: WIDTH[col?.kind ?? 'text'] ?? 80 };
            })}
            rows={shown.rows}
            sort={sort}
            onSort={(col) =>
              setSort((s) =>
                s?.col === col ? (s.dir === 1 ? { col, dir: -1 } : null) : { col, dir: 1 },
              )
            }
            cell={(v, c) => {
              const col = table.columns[shown.cols[c] ?? 0];
              const kind = col?.kind;
              // `show version`: the field column holds keys.
              if (col?.key === 'field') {
                const k = `analyzer.field.${v}` as TKey;
                const label = t(k);
                return label === k ? v : label;
              }
              const tone = kind === 'status' ? statusTone(v) : null;
              return tone ? (
                <span className="tone-text" data-tone={tone}>
                  {v}
                </span>
              ) : (
                v
              );
            }}
          />
        </>
      ) : (
        <div className="flex flex-1 flex-col items-center justify-center gap-2 p-6 text-center">
          <ScanSearch size={22} className="text-muted" aria-hidden />
          <p className="max-w-64 text-sm text-muted">{miss ?? t('analyzer.hint')}</p>
          {parsers && parsers.length > 0 && (
            <p className="max-w-64 text-xs text-muted">
              {t('analyzer.supported')}{' '}
              <span className="mono">{parsers.map((p) => p.command).join(', ')}</span>
            </p>
          )}
        </div>
      )}
    </div>
  );
}
