/**
 * Embedded TFTP server (P5-T02): pick a folder, a listen address and port, start. Devices
 * then `copy tftp://<this-ip>/<file> flash:` or `copy running-config tftp:`. Only files in
 * that folder are served; uploads and overwriting are switches.
 */
import { FolderOpen, Play, Square } from 'lucide-react';
import { useEffect, useState } from 'react';
import { useT } from '../i18n';
import { describeError } from '../lib/errors';
import { formatBytes } from '../lib/format';
import { api, type ScanNetwork } from '../lib/ipc';
import { useTftp } from '../stores/tftp';
import { toast } from '../stores/toasts';

export function TftpServer() {
  const t = useT();
  const status = useTftp((s) => s.status);
  const transfers = useTftp((s) => s.transfers);
  const [root, setRoot] = useState('');
  const [nets, setNets] = useState<ScanNetwork[]>([]);
  const [bindIp, setBindIp] = useState('0.0.0.0');
  const [port, setPort] = useState(69);
  const [allowUpload, setAllowUpload] = useState(true);
  const [overwrite, setOverwrite] = useState(false);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    void api
      .netscanNetworks()
      .then((n) => setNets(n.networks))
      .catch(() => {});
  }, []);

  const pick = async () => {
    const p = await api.pickPath('folder').catch(() => null);
    if (p) setRoot(p);
  };
  const start = async () => {
    setBusy(true);
    try {
      await useTftp.getState().start({ root, bindIp, port, allowUpload, overwrite });
    } catch (e) {
      const d = describeError(e, t);
      toast({ kind: 'error', message: d.message, detail: d.detail });
    } finally {
      setBusy(false);
    }
  };
  const shownIp = status.addr?.startsWith('0.0.0.0')
    ? (nets[0]?.ip ?? '<ip>')
    : status.addr?.split(':')[0];

  return (
    <div className="flex flex-col gap-3 p-2">
      {!status.running ? (
        <>
          <p className="text-xs text-muted">{t('tftp.hint')}</p>
          <label className="flex flex-col gap-1 text-xs text-secondary">
            {t('tftp.folder')}
            <div className="flex gap-1">
              <input
                className="field mono h-7 min-w-0 flex-1 text-xs"
                readOnly
                value={root}
                placeholder={t('tftp.pickFolder')}
              />
              <button
                type="button"
                className="icon-btn icon-btn-sm"
                aria-label={t('tftp.pickFolder')}
                onClick={() => void pick()}
              >
                <FolderOpen size={13} />
              </button>
            </div>
          </label>
          <div className="flex gap-2">
            <label className="flex min-w-0 flex-1 flex-col gap-1 text-xs text-secondary">
              {t('tftp.listen')}
              <select
                className="field h-7 text-xs"
                value={bindIp}
                onChange={(e) => setBindIp(e.target.value)}
              >
                <option value="0.0.0.0">{t('tftp.allInterfaces')}</option>
                {nets.map((n) => (
                  <option key={n.ip} value={n.ip}>
                    {n.iface} · {n.ip}
                  </option>
                ))}
              </select>
            </label>
            <label className="flex w-20 flex-col gap-1 text-xs text-secondary">
              {t('tftp.port')}
              <input
                className="field mono h-7 text-xs"
                inputMode="numeric"
                value={port}
                onChange={(e) =>
                  setPort(
                    Math.max(1, Math.min(65535, Number(e.target.value.replace(/\D/g, '')) || 69)),
                  )
                }
              />
            </label>
          </div>
          <label className="flex items-center gap-2 text-xs">
            <input
              type="checkbox"
              checked={allowUpload}
              onChange={(e) => setAllowUpload(e.target.checked)}
            />
            {t('tftp.allowUpload')}
          </label>
          <label className="flex items-center gap-2 text-xs">
            <input
              type="checkbox"
              checked={overwrite}
              disabled={!allowUpload}
              onChange={(e) => setOverwrite(e.target.checked)}
            />
            {t('tftp.overwrite')}
          </label>
          <button
            type="button"
            className="btn btn-primary h-8 self-start"
            disabled={!root || busy}
            onClick={() => void start()}
          >
            <Play size={13} aria-hidden /> {t('tftp.start')}
          </button>
        </>
      ) : (
        <>
          <div className="rounded-sm border border-subtle p-2 text-xs">
            <p className="font-medium">{t('tftp.running', { addr: status.addr ?? '' })}</p>
            <p className="mono truncate text-muted" title={status.root ?? ''}>
              {status.root}
            </p>
            <p className="mt-1 text-secondary">{t('tftp.example')}</p>
            <p className="mono select-all">copy tftp://{shownIp}/&lt;file&gt; flash:</p>
            <p className="mono select-all">copy running-config tftp://{shownIp}/sw1-confg</p>
          </div>
          <button
            type="button"
            className="btn btn-ghost h-8 self-start"
            onClick={() => void useTftp.getState().stop()}
          >
            <Square size={13} aria-hidden /> {t('tftp.stop')}
          </button>
        </>
      )}
      {transfers.length > 0 && (
        <ul className="flex flex-col gap-1">
          {transfers.map((x) => (
            <li key={x.id} className="text-xs">
              <div className="flex items-center gap-1">
                <span aria-hidden>{x.upload ? '↑' : '↓'}</span>
                <span className="mono min-w-0 flex-1 truncate">{x.file}</span>
                <span className="text-muted">{x.peer.split(':')[0]}</span>
                <span
                  className="tone-text"
                  data-tone={
                    x.state === 'failed' ? 'error' : x.state === 'done' ? 'success' : 'muted'
                  }
                >
                  {x.state === 'running'
                    ? formatBytes(x.bytes)
                    : x.state === 'done'
                      ? formatBytes(x.bytes)
                      : t('transfer.state.failed')}
                </span>
              </div>
              {x.error && (
                <p className="truncate text-muted" title={x.error}>
                  {x.error}
                </p>
              )}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
