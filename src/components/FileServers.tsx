/**
 * Embedded file servers (P5-T02 TFTP, P5-T03 FTP / SCP): pick a protocol, a folder, a
 * listen address and port, start. Devices then `copy tftp|ftp|scp://<this-ip>/<file>
 * flash:` or upload their config. Only files in that folder are served; uploads and
 * overwriting are switches. FTP / SCP use one login (random password unless given).
 */
import { Copy, FolderOpen, Play, Square } from 'lucide-react';
import { useEffect, useState } from 'react';
import { useT } from '../i18n';
import { describeError } from '../lib/errors';
import { formatBytes } from '../lib/format';
import { api, type ScanNetwork, type ServerKind, type ServerStatus } from '../lib/ipc';
import { useServers } from '../stores/servers';
import { toast } from '../stores/toasts';

const KINDS: { kind: ServerKind; label: string; port: number }[] = [
  { kind: 'tftp', label: 'TFTP', port: 69 },
  { kind: 'ftp', label: 'FTP', port: 21 },
  { kind: 'scp', label: 'SCP', port: 22 },
];

export const serverLabel = (k: ServerKind) => KINDS.find((x) => x.kind === k)?.label ?? k;

function copyText(text: string) {
  void api.clipboardWrite(text).catch(() => {});
}

export function FileServers() {
  const t = useT();
  const servers = useServers((s) => s.servers);
  const transfers = useServers((s) => s.transfers);
  const [kind, setKind] = useState<ServerKind>('tftp');
  const [root, setRoot] = useState('');
  const [nets, setNets] = useState<ScanNetwork[]>([]);
  const [bindIp, setBindIp] = useState('0.0.0.0');
  const [ports, setPorts] = useState<Record<ServerKind, number>>({ tftp: 69, ftp: 21, scp: 22 });
  const [user, setUser] = useState('kenshell');
  const [password, setPassword] = useState('');
  const [allowUpload, setAllowUpload] = useState(true);
  const [overwrite, setOverwrite] = useState(false);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    void api
      .netscanNetworks()
      .then((n) => setNets(n.networks))
      .catch(() => {});
  }, []);

  const current = servers.find((s) => s.kind === kind);
  const login = kind !== 'tftp';

  const pick = async () => {
    const p = await api.pickPath('folder').catch(() => null);
    if (p) setRoot(p);
  };
  const start = async () => {
    setBusy(true);
    try {
      await useServers.getState().start({
        kind,
        root,
        bindIp,
        port: ports[kind],
        allowUpload,
        overwrite,
        user: login ? user : null,
        password: login && password ? password : null,
      });
    } catch (e) {
      const d = describeError(e, t);
      toast({ kind: 'error', message: d.message, detail: d.detail });
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="flex flex-col gap-3 p-2">
      <div className="flex gap-1" role="group" aria-label={t('servers.title')}>
        {KINDS.map((k) => {
          const on = servers.some((s) => s.kind === k.kind);
          return (
            <button
              key={k.kind}
              type="button"
              aria-pressed={kind === k.kind}
              className={`btn h-7 flex-1 ${kind === k.kind ? 'btn-primary' : 'btn-ghost'}`}
              onClick={() => setKind(k.kind)}
            >
              {k.label}
              {on && (
                <span className="tone-text" data-tone="success" aria-hidden>
                  ●
                </span>
              )}
            </button>
          );
        })}
      </div>
      {current ? (
        <Running status={current} nets={nets} />
      ) : (
        <>
          <p className="text-xs text-muted">
            {t('servers.hint')}
            {kind === 'scp' && ` ${t('servers.scpHint')}`}
          </p>
          <label className="flex flex-col gap-1 text-xs text-secondary">
            {t('servers.folder')}
            <div className="flex gap-1">
              <input
                className="field mono h-7 min-w-0 flex-1 text-xs"
                readOnly
                value={root}
                placeholder={t('servers.pickFolder')}
              />
              <button
                type="button"
                className="icon-btn icon-btn-sm"
                aria-label={t('servers.pickFolder')}
                onClick={() => void pick()}
              >
                <FolderOpen size={13} />
              </button>
            </div>
          </label>
          <div className="flex gap-2">
            <label className="flex min-w-0 flex-1 flex-col gap-1 text-xs text-secondary">
              {t('servers.listen')}
              <select
                className="field h-7 text-xs"
                value={bindIp}
                onChange={(e) => setBindIp(e.target.value)}
              >
                <option value="0.0.0.0">{t('servers.allInterfaces')}</option>
                {nets.map((n) => (
                  <option key={n.ip} value={n.ip}>
                    {n.iface} · {n.ip}
                  </option>
                ))}
              </select>
            </label>
            <label className="flex w-20 flex-col gap-1 text-xs text-secondary">
              {t('servers.port')}
              <input
                className="field mono h-7 text-xs"
                inputMode="numeric"
                value={ports[kind]}
                onChange={(e) => {
                  const n = Number(e.target.value.replace(/\D/g, ''));
                  setPorts({ ...ports, [kind]: Math.max(1, Math.min(65535, n || 1)) });
                }}
              />
            </label>
          </div>
          {login && (
            <div className="flex gap-2">
              <label className="flex min-w-0 flex-1 flex-col gap-1 text-xs text-secondary">
                {t('servers.user')}
                <input
                  className="field mono h-7 text-xs"
                  value={user}
                  autoComplete="off"
                  spellCheck={false}
                  onChange={(e) => setUser(e.target.value)}
                />
              </label>
              <label className="flex min-w-0 flex-1 flex-col gap-1 text-xs text-secondary">
                {t('servers.password')}
                <input
                  className="field mono h-7 text-xs"
                  type="password"
                  value={password}
                  autoComplete="new-password"
                  placeholder={t('servers.passwordAuto')}
                  onChange={(e) => setPassword(e.target.value)}
                />
              </label>
            </div>
          )}
          <label className="flex items-center gap-2 text-xs">
            <input
              type="checkbox"
              checked={allowUpload}
              onChange={(e) => setAllowUpload(e.target.checked)}
            />
            {t('servers.allowUpload')}
          </label>
          <label className="flex items-center gap-2 text-xs">
            <input
              type="checkbox"
              checked={overwrite}
              disabled={!allowUpload}
              onChange={(e) => setOverwrite(e.target.checked)}
            />
            {t('servers.overwrite')}
          </label>
          <button
            type="button"
            className="btn btn-primary h-8 self-start"
            disabled={!root || busy}
            onClick={() => void start()}
          >
            <Play size={13} aria-hidden /> {busy ? t('servers.starting') : t('servers.start')}
          </button>
        </>
      )}
      {transfers.length > 0 && (
        <ul className="flex flex-col gap-1">
          {transfers.map((x) => (
            <li key={x.key} className="text-xs">
              <div className="flex items-center gap-1">
                <span aria-hidden>{x.upload ? '↑' : '↓'}</span>
                <span className="text-muted">{serverLabel(x.server)}</span>
                <span className="mono min-w-0 flex-1 truncate" title={x.file}>
                  {x.file}
                </span>
                <span className="text-muted">{x.peer.replace(/:\d+$/, '')}</span>
                <span
                  className="tone-text"
                  data-tone={
                    x.state === 'failed' ? 'error' : x.state === 'done' ? 'success' : 'muted'
                  }
                >
                  {x.state === 'failed' ? t('transfer.state.failed') : formatBytes(x.bytes)}
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

function Running({ status, nets }: { status: ServerStatus; nets: ScanNetwork[] }) {
  const t = useT();
  const [host, port] = [
    status.addr.replace(/:\d+$/, ''),
    Number(status.addr.match(/:(\d+)$/)?.[1] ?? 0),
  ];
  const ip = host === '0.0.0.0' ? (nets[0]?.ip ?? '<ip>') : host;
  const defaultPort = KINDS.find((k) => k.kind === status.kind)?.port;
  const at = port === defaultPort || status.kind === 'tftp' ? ip : `${ip}:${port}`;
  const examples =
    status.kind === 'tftp'
      ? [`copy tftp://${ip}/<file> flash:`, `copy running-config tftp://${ip}/sw1-confg`]
      : status.kind === 'ftp'
        ? [
            `copy ftp://${status.user}:${status.password}@${at}/<file> flash:`,
            `copy running-config ftp://${status.user}:${status.password}@${at}/sw1-confg`,
          ]
        : [
            `copy scp://${status.user}@${at}/<file> flash:`,
            `copy running-config scp://${status.user}@${at}/sw1-confg`,
          ];
  return (
    <>
      <div className="flex flex-col gap-1 rounded-sm border border-subtle p-2 text-xs">
        <p className="font-medium">
          {t('servers.running', { kind: serverLabel(status.kind), addr: status.addr })}
        </p>
        <p className="mono truncate text-muted" title={status.root}>
          {status.root}
        </p>
        {status.user && (
          <p className="mono select-all">
            {t('servers.login', { user: status.user, password: status.password ?? '' })}
          </p>
        )}
        {status.fingerprints.map((f) => (
          <p key={f} className="mono truncate text-muted" title={f}>
            {t('servers.fingerprint')}: {f}
          </p>
        ))}
        <p className="mt-1 text-secondary">{t('servers.example')}</p>
        {examples.map((cmd) => (
          <div key={cmd} className="flex items-center gap-1">
            <span className="mono min-w-0 flex-1 select-all break-all">{cmd}</span>
            <button
              type="button"
              className="icon-btn icon-btn-sm"
              aria-label={t('terminal.copy')}
              onClick={() => copyText(cmd)}
            >
              <Copy size={12} />
            </button>
          </div>
        ))}
      </div>
      <button
        type="button"
        className="btn btn-ghost h-8 self-start"
        onClick={() => void useServers.getState().stop(status.kind)}
      >
        <Square size={13} aria-hidden /> {t('servers.stop')}
      </button>
    </>
  );
}
