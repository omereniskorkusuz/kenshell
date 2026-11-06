/**
 * Same read-only command on several devices (P4-T11): pick connected tabs (the broadcast
 * group is preselected), run a show command hidden on each at once, and see one merged
 * table (when a parser knows the output) or each device's output.
 */
import { Download, Play, X } from 'lucide-react';
import { useMemo, useState } from 'react';
import { useT, type TKey } from '../i18n';
import { describeError } from '../lib/errors';
import { api } from '../lib/ipc';
import { isReadOnly, mergeTables, type DeviceResult } from '../lib/multiRun';
import { useTabs } from '../stores/tabs';
import { useUi } from '../stores/ui';
import { VirtualTable } from './VirtualTable';

interface Run {
  command: string;
  results: (DeviceResult & { text: string | null; error: string | null })[];
}

export function MultiRun({ onClose }: { onClose: () => void }) {
  const t = useT();
  const tabs = useTabs((s) => s.tabs);
  const connected = useMemo(
    () => tabs.filter((x) => x.state === 'connected' && x.runtimeId !== null),
    [tabs],
  );
  const [picked, setPicked] = useState<string[]>(() => {
    const group = useTabs.getState().broadcast;
    return group.length > 1 ? group : connected.map((x) => x.key);
  });
  const [command, setCommand] = useState('');
  const [busy, setBusy] = useState(false);
  const [run, setRun] = useState<Run | null>(null);
  const valid = isReadOnly(command);
  const chosen = connected.filter((x) => picked.includes(x.key));

  const go = async () => {
    if (!valid || chosen.length === 0) return;
    setBusy(true);
    const cmd = command.trim();
    const results = await Promise.all(
      chosen.map(async (tab) => {
        const device = tab.device.hostname ?? tab.name;
        try {
          const text = await api.liveCapture(tab.runtimeId as number, cmd, 120);
          const table = await api
            .analyzeOutput(text, tab.device.vendor?.family ?? null, cmd)
            .catch(() => null);
          return { device, text, table, error: null };
        } catch (e) {
          return { device, text: null, table: null, error: describeError(e, t).message };
        }
      }),
    );
    setRun({ command: cmd, results });
    setBusy(false);
  };

  const merged = useMemo(() => (run ? mergeTables(run.results) : null), [run]);
  const label = (key: string) => {
    const k = `analyzer.col.${key}` as TKey;
    const v = t(k);
    return v === k ? (key === 'device' ? t('topology.device') : key) : v;
  };

  const exportCsv = async () => {
    if (!merged) return;
    const path = await api
      .pickPath('save', ['csv'], `${merged.command.replace(/\s+/g, '_')}_devices.csv`)
      .catch(() => null);
    if (!path) return;
    await api
      .analyzerExportCsv(
        path,
        merged.columns.map((c) => label(c.key)),
        merged.rows,
      )
      .then(() => useUi.getState().flash(t('analyzer.exported', { path })))
      .catch(() => {});
  };

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex items-center gap-1 border-b border-subtle p-2">
        <span className="min-w-0 flex-1 truncate text-sm font-medium">{t('multi.title')}</span>
        {merged && (
          <button
            type="button"
            className="icon-btn icon-btn-sm"
            aria-label={t('analyzer.exportCsv')}
            title={t('analyzer.exportCsv')}
            onClick={() => void exportCsv()}
          >
            <Download size={13} />
          </button>
        )}
        <button
          type="button"
          className="icon-btn icon-btn-sm"
          aria-label={t('common.close')}
          onClick={onClose}
        >
          <X size={13} />
        </button>
      </div>
      <div className="flex flex-col gap-2 border-b border-subtle p-2">
        {connected.length === 0 ? (
          <p className="text-sm text-muted">{t('multi.noTabs')}</p>
        ) : (
          <fieldset className="flex flex-wrap gap-x-3 gap-y-1">
            <legend className="mb-1 text-xs text-secondary">{t('multi.devices')}</legend>
            {connected.map((x) => (
              <label key={x.key} className="flex items-center gap-1.5 text-xs">
                <input
                  type="checkbox"
                  checked={picked.includes(x.key)}
                  onChange={(e) =>
                    setPicked((p) =>
                      e.target.checked ? [...p, x.key] : p.filter((k) => k !== x.key),
                    )
                  }
                />
                <span className="mono">{x.device.hostname ?? x.name}</span>
              </label>
            ))}
          </fieldset>
        )}
        <form
          className="flex gap-1"
          onSubmit={(e) => {
            e.preventDefault();
            void go();
          }}
        >
          <input
            className="field mono h-7 min-w-0 flex-1 text-sm"
            placeholder="show version"
            aria-label={t('multi.command')}
            spellCheck={false}
            value={command}
            onChange={(e) => setCommand(e.target.value)}
          />
          <button
            type="submit"
            className="btn btn-primary h-7 text-xs"
            disabled={!valid || chosen.length === 0 || busy}
          >
            <Play size={12} aria-hidden />{' '}
            {busy ? t('multi.running') : t('multi.run', { count: chosen.length })}
          </button>
        </form>
        {command.trim() !== '' && !valid && (
          <p className="text-xs text-muted">{t('multi.readOnlyOnly')}</p>
        )}
      </div>
      {run && merged && (
        <VirtualTable
          label={run.command}
          columns={merged.columns.map((c) => ({
            label: label(c.key),
            width: c.key === 'device' ? 110 : 100,
          }))}
          rows={merged.rows}
          sort={null}
          onSort={() => {}}
        />
      )}
      {run && (
        <div
          className={`overflow-y-auto p-2 ${merged ? 'max-h-48 border-t border-subtle' : 'min-h-0 flex-1'}`}
        >
          {run.results.map((r) => (
            <details key={r.device} open={!merged && run.results.length <= 3}>
              <summary className="cursor-default text-xs">
                <span className="mono">{r.device}</span>
                {r.error && <span className="ml-2 text-danger">{r.error}</span>}
              </summary>
              {r.text && (
                <pre className="mono max-h-64 overflow-auto rounded-sm bg-terminal p-2 text-[11px]">
                  {r.text}
                </pre>
              )}
            </details>
          ))}
        </div>
      )}
    </div>
  );
}
