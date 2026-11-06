/**
 * Config backups (P4-T08): take running/startup backups (read hidden, stored encrypted —
 * ADR-016), import a file, compare two backups, a backup with the running config now, or
 * running ↔ startup, side by side. Delete is immediate with undo (no confirm dialogs).
 */
import { ArrowLeft, Download, FileUp, GitCompare, Trash2, X } from 'lucide-react';
import { useCallback, useEffect, useState } from 'react';
import { useT } from '../i18n';
import { describeError } from '../lib/errors';
import { formatBytes } from '../lib/format';
import { api, type BackupInfo, type ConfigDiff } from '../lib/ipc';
import type { LiveTab } from '../lib/sessionState';
import { toast } from '../stores/toasts';
import { useUi } from '../stores/ui';
import { DiffView } from './DiffView';

interface Compare {
  diff: ConfigDiff;
  left: string;
  right: string;
}

export function Backups({ tab, onClose }: { tab: LiveTab; onClose: () => void }) {
  const t = useT();
  const sessionId = tab.sessionId;
  const deviceId = tab.device.deviceId;
  const owned = sessionId !== null || deviceId !== null;
  const connected = tab.state === 'connected' && tab.runtimeId !== null;
  const [list, setList] = useState<BackupInfo[]>([]);
  const [selected, setSelected] = useState<number[]>([]);
  const [busy, setBusy] = useState<string | null>(null);
  const [compare, setCompare] = useState<Compare | null>(null);
  const [viewing, setViewing] = useState<{ title: string; text: string } | null>(null);

  const fail = useCallback(
    (e: unknown) => {
      const d = describeError(e, t);
      toast({ kind: 'error', message: d.message, detail: d.detail });
    },
    [t],
  );
  const load = useCallback(() => {
    if (!owned) return;
    api
      .backupsList(sessionId, deviceId)
      .then(setList)
      .catch(() => setList([]));
  }, [owned, sessionId, deviceId]);
  useEffect(load, [load]);

  const when = (b: BackupInfo) =>
    new Date(b.createdAt).toLocaleString(undefined, { dateStyle: 'short', timeStyle: 'short' });
  const label = (b: BackupInfo) => `${when(b)} · ${t(`backups.src.${b.source}`)}`;

  const run = async (what: string, fn: () => Promise<void>) => {
    setBusy(what);
    try {
      await fn();
    } catch (e) {
      fail(e);
    } finally {
      setBusy(null);
    }
  };

  const take = (source: 'running' | 'startup') =>
    run(source, async () => {
      if (tab.runtimeId === null) return;
      const b = await api.backupTake(
        tab.runtimeId,
        sessionId,
        deviceId,
        tab.device.hostname,
        source,
      );
      useUi.getState().flash(t(b.unchanged ? 'backups.unchanged' : 'backups.saved'));
      load();
    });

  const importFile = () =>
    run('import', async () => {
      const path = await api.pickPath('file', ['txt', 'cfg', 'conf', 'log']);
      if (!path) return;
      const text = await api.configFileRead(path);
      await api.backupImport(sessionId, deviceId, tab.device.hostname, text);
      load();
    });

  const remove = (b: BackupInfo) =>
    run('delete', async () => {
      // Kept in memory for undo; the stored copy is gone at once.
      const text = await api.backupText(b.id);
      await api.backupDelete(b.id);
      setSelected((s) => s.filter((x) => x !== b.id));
      load();
      toast({
        message: t('backups.deleted', { name: label(b) }),
        actions: [
          {
            label: t('common.undo'),
            primary: true,
            run: () =>
              void api
                .backupImport(b.sessionId, b.deviceId, b.hostname, text, b.source)
                .then(load)
                .catch(fail),
          },
        ],
      });
    });

  const exportOne = (b: BackupInfo) =>
    run('export', async () => {
      const name = `${b.hostname ?? 'config'}_${b.source}_${new Date(b.createdAt).toISOString().slice(0, 16).replace(/[:T]/g, '-')}.txt`;
      const path = await api.pickPath('save', ['txt'], name);
      if (!path) return;
      await api.backupExport(b.id, path);
      useUi.getState().flash(t('backups.exported', { path }));
    });

  const compareSelected = () =>
    run('compare', async () => {
      const picked = list
        .filter((b) => selected.includes(b.id))
        .sort((x, y) => x.createdAt - y.createdAt);
      const [older, newer] = picked;
      if (!older || !newer) return;
      const [l, r] = await Promise.all([api.backupText(older.id), api.backupText(newer.id)]);
      setCompare({ diff: await api.configDiff(l, r), left: label(older), right: label(newer) });
    });

  const compareWithRunning = () =>
    run('compare', async () => {
      const b = list.find((x) => x.id === selected[0]);
      if (!b || tab.runtimeId === null) return;
      setCompare({
        diff: await api.backupDiffLive(b.id, tab.runtimeId),
        left: label(b),
        right: t('backups.runningNow'),
      });
    });

  const runningVsStartup = () =>
    run('compare', async () => {
      if (tab.runtimeId === null) return;
      setCompare({
        diff: await api.configLiveDiff(tab.runtimeId),
        left: 'startup-config',
        right: 'running-config',
      });
    });

  const header = (title: string, back?: () => void) => (
    <div className="flex items-center gap-1 border-b border-subtle p-2">
      {back && (
        <button
          type="button"
          className="icon-btn icon-btn-sm"
          aria-label={t('backups.back')}
          onClick={back}
        >
          <ArrowLeft size={13} />
        </button>
      )}
      <span className="min-w-0 flex-1 truncate text-sm font-medium">{title}</span>
      <button
        type="button"
        className="icon-btn icon-btn-sm"
        aria-label={t('common.close')}
        onClick={onClose}
      >
        <X size={13} />
      </button>
    </div>
  );

  if (compare) {
    return (
      <div className="flex min-h-0 flex-1 flex-col">
        {header(t('backups.compareTitle'), () => setCompare(null))}
        <DiffView diff={compare.diff} leftTitle={compare.left} rightTitle={compare.right} />
      </div>
    );
  }
  if (viewing) {
    return (
      <div className="flex min-h-0 flex-1 flex-col">
        {header(viewing.title, () => setViewing(null))}
        <pre className="mono min-h-0 flex-1 overflow-auto p-2 text-[11px] leading-5">
          {viewing.text}
        </pre>
      </div>
    );
  }

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      {header(`${t('backups.title')} · ${tab.device.hostname ?? tab.name}`)}
      {!owned ? (
        <p className="p-4 text-center text-sm text-muted">{t('backups.needsOwner')}</p>
      ) : (
        <>
          <div className="flex flex-wrap items-center gap-1 border-b border-subtle p-2">
            <button
              type="button"
              className="btn btn-primary h-7 text-xs"
              disabled={!connected || busy !== null}
              onClick={() => void take('running')}
            >
              {busy === 'running' ? t('backups.reading') : t('backups.takeRunning')}
            </button>
            <button
              type="button"
              className="btn btn-ghost h-7 text-xs"
              disabled={!connected || busy !== null}
              onClick={() => void take('startup')}
            >
              {t('backups.takeStartup')}
            </button>
            <button
              type="button"
              className="icon-btn icon-btn-sm"
              disabled={busy !== null}
              aria-label={t('backups.import')}
              title={t('backups.import')}
              onClick={() => void importFile()}
            >
              <FileUp size={13} />
            </button>
            <span className="flex-1" />
            <button
              type="button"
              className="btn btn-ghost h-7 text-xs"
              disabled={!connected || busy !== null}
              onClick={() => void runningVsStartup()}
            >
              {t('backups.runningVsStartup')}
            </button>
          </div>
          <div className="flex flex-wrap items-center gap-1 px-2 py-1.5 text-xs text-muted">
            <span className="w-full">{t('backups.selectHint')}</span>
            <button
              type="button"
              className="btn btn-ghost h-6 text-xs"
              disabled={selected.length !== 2 || busy !== null}
              onClick={() => void compareSelected()}
            >
              <GitCompare size={12} aria-hidden /> {t('backups.compare')}
            </button>
            <button
              type="button"
              className="btn btn-ghost h-6 text-xs"
              disabled={selected.length !== 1 || !connected || busy !== null}
              onClick={() => void compareWithRunning()}
            >
              {t('backups.compareRunning')}
            </button>
          </div>
          <ul className="min-h-0 flex-1 overflow-y-auto p-1">
            {list.length === 0 && (
              <li className="p-4 text-center text-sm text-muted">{t('backups.none')}</li>
            )}
            {list.map((b) => {
              const on = selected.includes(b.id);
              return (
                <li
                  key={b.id}
                  className="group flex items-center gap-2 rounded-sm px-1 py-1 hover:bg-hover"
                >
                  <input
                    type="checkbox"
                    aria-label={label(b)}
                    checked={on}
                    onChange={() =>
                      setSelected((s) =>
                        on ? s.filter((x) => x !== b.id) : [...s, b.id].slice(-2),
                      )
                    }
                  />
                  <button
                    type="button"
                    className="min-w-0 flex-1 text-left"
                    onClick={() =>
                      void run('view', async () =>
                        setViewing({ title: label(b), text: await api.backupText(b.id) }),
                      )
                    }
                  >
                    <span className="block truncate text-sm">{label(b)}</span>
                    <span className="block truncate text-[11px] text-muted">
                      {[b.hostname, t('backups.lines', { count: b.lines }), formatBytes(b.size)]
                        .filter(Boolean)
                        .join(' · ')}
                    </span>
                  </button>
                  <button
                    type="button"
                    className="icon-btn icon-btn-sm opacity-70 group-hover:opacity-100"
                    aria-label={t('backups.export')}
                    title={t('backups.export')}
                    onClick={() => void exportOne(b)}
                  >
                    <Download size={13} />
                  </button>
                  <button
                    type="button"
                    className="icon-btn icon-btn-sm opacity-70 group-hover:opacity-100"
                    aria-label={t('backups.delete')}
                    title={t('backups.delete')}
                    onClick={() => void remove(b)}
                  >
                    <Trash2 size={13} />
                  </button>
                </li>
              );
            })}
          </ul>
          <p className="border-t border-subtle px-2 py-1.5 text-[11px] text-muted">
            {t('backups.encrypted')}
          </p>
        </>
      )}
    </div>
  );
}
