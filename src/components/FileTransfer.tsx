/**
 * File transfer (P5-T01), in the side panel's Tools tab: SFTP browser and SCP get/put on
 * the active SSH tab's own connection (no second login). Network devices usually offer
 * SCP only; the SFTP view says so when the device has no SFTP server.
 */
import {
  ArrowUp,
  Download,
  Folder,
  FolderPlus,
  Pencil,
  RefreshCw,
  Trash2,
  Upload,
  X,
} from 'lucide-react';
import { useCallback, useEffect, useRef, useState } from 'react';
import { useT } from '../i18n';
import { describeError } from '../lib/errors';
import { formatBytes } from '../lib/format';
import { api, transfer, type SftpEntry, type TransferProgress } from '../lib/ipc';
import { useActiveTab } from '../stores/tabs';
import { toast } from '../stores/toasts';
import { useUi } from '../stores/ui';

interface Job {
  id: number;
  label: string;
  progress: TransferProgress;
  state: 'running' | 'done' | 'failed' | 'cancelled';
  error?: string;
}

let nextJob = 1;

const parent = (path: string) => {
  const p = path.replace(/\/+$/, '').split('/').slice(0, -1).join('/');
  return p === '' ? '/' : p;
};
const join = (dir: string, name: string) =>
  dir.endsWith('/') ? `${dir}${name}` : `${dir}/${name}`;
const baseName = (p: string) =>
  p
    .split(/[\\/:]/)
    .filter(Boolean)
    .pop() ?? 'file';

export function FileTransfer() {
  const t = useT();
  const tab = useActiveTab();
  const ssh = tab?.target.type === 'network' && tab.target.protocol === 'ssh';
  const ready = ssh && tab?.state === 'connected' && tab.runtimeId !== null;
  const [mode, setMode] = useState<'sftp' | 'scp'>('scp');
  const [jobs, setJobs] = useState<Job[]>([]);

  const fail = useCallback(
    (e: unknown) => {
      const d = describeError(e, t);
      toast({ kind: 'error', message: d.message, detail: d.detail });
    },
    [t],
  );

  /** Runs a transfer with a progress row; returns when it ends. */
  const track = async (
    label: string,
    run: (id: number, onProgress: (p: TransferProgress) => void) => Promise<unknown>,
  ) => {
    const id = nextJob++;
    setJobs((j) =>
      [{ id, label, progress: { done: 0, total: 0 }, state: 'running' as const }, ...j].slice(
        0,
        20,
      ),
    );
    const update = (patch: Partial<Job>) =>
      setJobs((j) => j.map((x) => (x.id === id ? { ...x, ...patch } : x)));
    try {
      await run(id, (progress) => update({ progress }));
      update({ state: 'done' });
      useUi.getState().flash(t('transfer.done', { name: label }));
    } catch (e) {
      const d = describeError(e, t);
      update({ state: d.message.includes('cancelled') ? 'cancelled' : 'failed', error: d.message });
    }
  };

  if (!tab || !ssh)
    return <p className="p-4 text-center text-sm text-muted">{t('transfer.needsSsh')}</p>;
  if (!ready)
    return <p className="p-4 text-center text-sm text-muted">{t('transfer.connectFirst')}</p>;
  const runtimeId = tab.runtimeId as number;

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex items-center gap-1 border-b border-subtle p-2">
        <div
          role="group"
          aria-label={t('transfer.title')}
          className="flex gap-0.5 rounded-md bg-app p-0.5"
        >
          {(['scp', 'sftp'] as const).map((m) => (
            <button
              key={m}
              type="button"
              aria-pressed={mode === m}
              className={`h-6 rounded-sm px-2 text-xs ${mode === m ? 'bg-selected text-primary' : 'text-secondary'}`}
              onClick={() => setMode(m)}
            >
              {m.toUpperCase()}
            </button>
          ))}
        </div>
        <span className="truncate text-xs text-muted">{t('transfer.sameConnection')}</span>
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto">
        {mode === 'scp' ? (
          <ScpForm runtimeId={runtimeId} track={track} fail={fail} />
        ) : (
          <SftpBrowser key={tab.key} runtimeId={runtimeId} track={track} fail={fail} />
        )}
      </div>
      {jobs.length > 0 && (
        <ul className="max-h-40 overflow-y-auto border-t border-subtle p-2">
          {jobs.map((j) => (
            <li key={j.id} className="mb-1.5 text-xs">
              <div className="flex items-center gap-1">
                <span className="mono min-w-0 flex-1 truncate">{j.label}</span>
                <span className="shrink-0 text-muted">
                  {j.state === 'running'
                    ? `${formatBytes(j.progress.done)}${j.progress.total ? ` / ${formatBytes(j.progress.total)}` : ''}`
                    : t(`transfer.state.${j.state}`)}
                </span>
                {j.state === 'running' && (
                  <button
                    type="button"
                    className="icon-btn icon-btn-sm"
                    aria-label={t('transfer.cancel')}
                    onClick={() => void transfer.cancel(j.id)}
                  >
                    <X size={12} />
                  </button>
                )}
              </div>
              {j.state === 'running' && (
                <div className="mt-0.5 h-1 overflow-hidden rounded-full bg-hover">
                  <div
                    className="h-full bg-accent transition-[width]"
                    style={{
                      width: `${j.progress.total ? (j.progress.done / j.progress.total) * 100 : 0}%`,
                    }}
                  />
                </div>
              )}
              {j.error && j.state === 'failed' && (
                <p className="tone-text truncate" data-tone="error" title={j.error}>
                  {j.error}
                </p>
              )}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

type Track = (
  label: string,
  run: (id: number, onProgress: (p: TransferProgress) => void) => Promise<unknown>,
) => Promise<void>;

function ScpForm({
  runtimeId,
  track,
  fail,
}: {
  runtimeId: number;
  track: Track;
  fail: (e: unknown) => void;
}) {
  const t = useT();
  const [remote, setRemote] = useState('flash:startup-config');
  const [dest, setDest] = useState('flash:');

  const get = async () => {
    const local = await api.pickPath('save', [], baseName(remote)).catch(() => null);
    if (!local) return;
    await track(baseName(remote), (id, p) =>
      transfer.scpDownload(runtimeId, id, remote, local, p),
    ).catch(fail);
  };
  const put = async () => {
    const local = await api.pickPath('file').catch(() => null);
    if (!local) return;
    await track(baseName(local), (id, p) =>
      transfer.scpUpload(runtimeId, id, local, dest, p),
    ).catch(fail);
  };

  return (
    <div className="flex flex-col gap-3 p-2">
      <p className="text-xs text-muted">{t('transfer.scpHint')}</p>
      <label className="flex flex-col gap-1 text-xs text-secondary">
        {t('transfer.remoteFile')}
        <div className="flex gap-1">
          <input
            className="field mono h-7 min-w-0 flex-1 text-sm"
            spellCheck={false}
            value={remote}
            onChange={(e) => setRemote(e.target.value)}
          />
          <button
            type="button"
            className="btn btn-primary h-7 text-xs"
            disabled={!remote.trim()}
            onClick={() => void get()}
          >
            <Download size={12} aria-hidden /> {t('transfer.download')}
          </button>
        </div>
      </label>
      <label className="flex flex-col gap-1 text-xs text-secondary">
        {t('transfer.remoteDir')}
        <div className="flex gap-1">
          <input
            className="field mono h-7 min-w-0 flex-1 text-sm"
            spellCheck={false}
            value={dest}
            onChange={(e) => setDest(e.target.value)}
          />
          <button
            type="button"
            className="btn btn-ghost h-7 text-xs"
            disabled={!dest.trim()}
            onClick={() => void put()}
          >
            <Upload size={12} aria-hidden /> {t('transfer.uploadFile')}
          </button>
        </div>
      </label>
    </div>
  );
}

function SftpBrowser({
  runtimeId,
  track,
  fail,
}: {
  runtimeId: number;
  track: Track;
  fail: (e: unknown) => void;
}) {
  const t = useT();
  const [sftp, setSftp] = useState<number | null>(null);
  const [path, setPath] = useState('/');
  const [list, setList] = useState<SftpEntry[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [renaming, setRenaming] = useState<{ path: string; name: string } | null>(null);
  const opened = useRef<number | null>(null);

  useEffect(() => {
    let live = true;
    transfer
      .sftpOpen(runtimeId)
      .then((r) => {
        if (!live) return void transfer.sftpClose(r.sftp);
        opened.current = r.sftp;
        setSftp(r.sftp);
        setPath(r.home);
      })
      .catch((e: unknown) => live && setError(describeError(e, t).message));
    return () => {
      live = false;
      if (opened.current !== null) void transfer.sftpClose(opened.current);
    };
  }, [runtimeId, t]);

  const load = useCallback(() => {
    if (sftp === null) return;
    transfer
      .sftpList(sftp, path)
      .then(setList)
      .catch((e: unknown) => setError(describeError(e, t).message));
  }, [sftp, path, t]);
  useEffect(load, [load]);

  if (error) return <p className="p-4 text-sm text-muted">{error}</p>;
  if (sftp === null || !list)
    return <p className="p-4 text-sm text-muted">{t('transfer.opening')}</p>;

  const act = (fn: () => Promise<unknown>) => void fn().then(load).catch(fail);
  const download = async (e: SftpEntry) => {
    const local = await api.pickPath('save', [], e.name).catch(() => null);
    if (!local) return;
    await track(e.name, (id, p) => transfer.sftpDownload(sftp, id, e.path, local, p));
  };
  const upload = async () => {
    const local = await api.pickPath('file').catch(() => null);
    if (!local) return;
    await track(baseName(local), (id, p) => transfer.sftpUpload(sftp, id, local, path, p));
    load();
  };

  return (
    <div className="flex flex-col">
      <div className="flex items-center gap-1 px-2 py-1.5">
        <button
          type="button"
          className="icon-btn icon-btn-sm"
          disabled={path === '/'}
          aria-label={t('transfer.up')}
          onClick={() => setPath(parent(path))}
        >
          <ArrowUp size={13} />
        </button>
        <span className="mono min-w-0 flex-1 truncate text-xs" title={path}>
          {path}
        </span>
        <button
          type="button"
          className="icon-btn icon-btn-sm"
          aria-label={t('transfer.refresh')}
          onClick={load}
        >
          <RefreshCw size={13} />
        </button>
        <button
          type="button"
          className="icon-btn icon-btn-sm"
          aria-label={t('transfer.newFolder')}
          onClick={() =>
            act(() => transfer.sftpMkdir(sftp, join(path, t('transfer.newFolderName'))))
          }
        >
          <FolderPlus size={13} />
        </button>
        <button
          type="button"
          className="icon-btn icon-btn-sm"
          aria-label={t('transfer.uploadFile')}
          title={t('transfer.uploadFile')}
          onClick={() => void upload()}
        >
          <Upload size={13} />
        </button>
      </div>
      {list.length === 0 && (
        <p className="p-4 text-center text-sm text-muted">{t('transfer.empty')}</p>
      )}
      <ul>
        {list.map((e) => (
          <li key={e.path} className="group flex items-center gap-2 px-2 py-0.5 hover:bg-hover">
            {e.isDir ? (
              <Folder size={13} className="shrink-0 text-muted" aria-hidden />
            ) : (
              <span className="w-[13px] shrink-0" />
            )}
            {renaming?.path === e.path ? (
              <input
                className="field mono h-6 min-w-0 flex-1 text-xs"
                autoFocus
                value={renaming.name}
                aria-label={t('transfer.rename')}
                onChange={(ev) => setRenaming({ path: e.path, name: ev.target.value })}
                onKeyDown={(ev) => {
                  if (ev.key === 'Escape') setRenaming(null);
                  if (ev.key === 'Enter' && renaming.name.trim()) {
                    setRenaming(null);
                    act(() => transfer.sftpRename(sftp, e.path, join(path, renaming.name.trim())));
                  }
                }}
              />
            ) : (
              <button
                type="button"
                className="mono min-w-0 flex-1 truncate text-left text-xs"
                onDoubleClick={() => (e.isDir ? setPath(e.path) : void download(e))}
                onClick={() => e.isDir && setPath(e.path)}
              >
                {e.name}
              </button>
            )}
            <span className="shrink-0 text-[11px] text-muted">
              {e.isDir ? '' : formatBytes(e.size)}
            </span>
            <span className="flex shrink-0 opacity-60 group-hover:opacity-100">
              {!e.isDir && (
                <button
                  type="button"
                  className="icon-btn icon-btn-sm"
                  aria-label={t('transfer.download')}
                  onClick={() => void download(e)}
                >
                  <Download size={12} />
                </button>
              )}
              <button
                type="button"
                className="icon-btn icon-btn-sm"
                aria-label={t('transfer.rename')}
                onClick={() => setRenaming({ path: e.path, name: e.name })}
              >
                <Pencil size={12} />
              </button>
              <button
                type="button"
                className="icon-btn icon-btn-sm"
                aria-label={t('transfer.delete')}
                onClick={() => act(() => transfer.sftpRemove(sftp, e.path, e.isDir))}
              >
                <Trash2 size={12} />
              </button>
            </span>
          </li>
        ))}
      </ul>
    </div>
  );
}
