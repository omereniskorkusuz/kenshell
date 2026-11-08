/**
 * "Scan my network" (P4-T14, lazy chunk): pick one of this computer's own subnets (default:
 * the active interface's /24), scan it (discovery only — the backend enforces the scope
 * rules), then see a grid, a list and the probably-free address blocks. Right-click a used
 * address to connect, save it as a session or open its web page. CSV export.
 */
import { Copy, Download, Radar, Square } from 'lucide-react';
import { useEffect, useMemo, useState, type MouseEvent } from 'react';
import { Dialog } from '../components/Dialog';
import { Menu, useMenu, type MenuEntry } from '../components/Menu';
import { useT, type TKey } from '../i18n';
import {
  api,
  DEFAULT_PORTS,
  DEFAULT_SSH,
  netscanRun,
  type NetProtocol,
  type ScanNetworks,
  type ScanProgress,
  type ScanResult,
  type ScopeReason,
} from '../lib/ipc';
import {
  blockAddresses,
  blockLabel,
  csvRows,
  CSV_HEADER,
  freeBlocks,
  gridCells,
  ipToNum,
  type ScanHost,
} from '../lib/netscan';
import { useTabs } from '../stores/tabs';
import { useUi } from '../stores/ui';

type View = 'grid' | 'list' | 'free';

export default function NetScan() {
  const t = useT();
  const close = () => useUi.getState().setScanOpen(false);
  const [nets, setNets] = useState<ScanNetworks | null>(null);
  const [target, setTarget] = useState('');
  const [check, setCheck] = useState<
    { count: number; iface: string } | { reason: ScopeReason } | null
  >(null);
  const [progress, setProgress] = useState<ScanProgress | null>(null);
  const [result, setResult] = useState<ScanResult | null>(null);
  const [view, setView] = useState<View>('grid');
  const [picked, setPicked] = useState<ScanHost | null>(null);
  const menu = useMenu();
  const running = progress !== null;

  useEffect(() => {
    void api.netscanNetworks().then((n) => {
      setNets(n);
      if (n.defaultTarget) setTarget(n.defaultTarget);
    });
  }, []);

  // Scope check as the target changes (the backend rule; "Scan" stays off when it fails).
  useEffect(() => {
    if (!target.trim()) return;
    let live = true;
    const timer = window.setTimeout(() => {
      api
        .netscanCheck(target)
        .then((c) => live && setCheck({ count: c.count, iface: c.iface }))
        .catch(
          (e: { reason?: ScopeReason }) => live && setCheck({ reason: e?.reason ?? 'invalid' }),
        );
    }, 150);
    return () => {
      live = false;
      window.clearTimeout(timer);
    };
  }, [target]);

  const scan = async () => {
    if (!check || 'reason' in check) return;
    setResult(null);
    setPicked(null);
    setProgress({ stage: 'sweep', done: 0, total: check.count });
    try {
      setResult(await netscanRun(target, setProgress));
    } catch (e) {
      const reason = (e as { reason?: ScopeReason }).reason;
      if (reason) setCheck({ reason });
      else useUi.getState().flash(t('netscan.busy'));
    } finally {
      setProgress(null);
    }
  };

  const hosts = useMemo(() => result?.hosts ?? [], [result]);
  const used = useMemo(() => hosts.filter((h) => h.alive), [hosts]);
  const free = useMemo(() => freeBlocks(hosts), [hosts]);
  const grids = useMemo(() => gridCells(hosts), [hosts]);
  const freeCount = hosts.length - used.length;

  const copy = (text: string) =>
    void api
      .clipboardWrite(text)
      .then(() => useUi.getState().flash(t('netscan.copied')))
      .catch(() => {});

  const connect = (h: ScanHost, protocol: NetProtocol) => {
    useTabs.getState().openTarget(
      {
        type: 'network',
        protocol,
        host: h.ip,
        port: DEFAULT_PORTS[protocol],
        username: null,
        ssh: DEFAULT_SSH,
      },
      { name: h.hostname?.split('.')[0] ?? h.ip },
    );
    close();
  };

  const entries = (h: ScanHost): MenuEntry[] => [
    {
      label: t('netscan.ssh'),
      ...(h.openPorts.includes(22) ? { hint: ':22' } : {}),
      onSelect: () => connect(h, 'ssh'),
    },
    {
      label: 'Telnet',
      ...(h.openPorts.includes(23) ? { hint: ':23' } : {}),
      onSelect: () => connect(h, 'telnet'),
    },
    {
      label: t('netscan.saveSession'),
      onSelect: () => {
        close();
        useUi.getState().openEditor({
          sessionId: null,
          initial: {
            name: h.hostname?.split('.')[0] ?? h.ip,
            protocol: 'ssh',
            host: h.ip,
            port: 22,
          },
        });
      },
    },
    { kind: 'separator' },
    {
      label: t('netscan.openWeb'),
      disabled: !h.openPorts.includes(80) && !h.openPorts.includes(443),
      onSelect: () => void api.openDeviceWeb(h.ip, h.openPorts.includes(443)).catch(() => {}),
    },
    { label: t('netscan.copyIp'), onSelect: () => copy(h.ip) },
  ];

  const openMenu = (e: MouseEvent, h: ScanHost) => {
    if (!h.alive || h.own) return;
    e.preventDefault();
    setPicked(h);
    menu.openAt(e.clientX, e.clientY);
  };

  const status = (h: ScanHost) =>
    t(
      h.own
        ? 'netscan.own'
        : h.gateway
          ? 'netscan.gateway'
          : h.alive
            ? 'netscan.used'
            : 'netscan.free',
    );

  const exportCsv = async () => {
    const path = await api
      .pickPath('save', ['csv'], `scan_${target.replace(/[^\w.-]+/g, '_')}.csv`)
      .catch(() => null);
    if (!path) return;
    await api
      .analyzerExportCsv(path, [...CSV_HEADER], csvRows(hosts, status))
      .then(() => useUi.getState().flash(t('analyzer.exported', { path })))
      .catch(() => {});
  };

  const reason = check && 'reason' in check ? check.reason : null;
  const tip = (h: ScanHost) =>
    [
      h.ip,
      h.hostname,
      h.mac,
      h.vendor,
      h.openPorts.length ? `${t('netscan.ports')}: ${h.openPorts.join(', ')}` : null,
      status(h),
    ]
      .filter(Boolean)
      .join('\n');

  return (
    <Dialog title={t('netscan.title')} onClose={close} width="880px">
      <div className="flex flex-col gap-3 p-4">
        <div className="flex flex-wrap items-center gap-2">
          <select
            className="field h-8 w-56 text-sm"
            aria-label={t('netscan.network')}
            value={
              nets?.networks.find((n) => n.defaultTarget === target || n.cidr === target)?.cidr ??
              ''
            }
            onChange={(e) => {
              const n = nets?.networks.find((x) => x.cidr === e.target.value);
              if (n) setTarget(n.defaultTarget);
            }}
          >
            <option value="" disabled>
              {t('netscan.custom')}
            </option>
            {nets?.networks.map((n) => (
              <option key={n.cidr} value={n.cidr}>
                {n.iface} · {n.cidr}
              </option>
            ))}
          </select>
          <input
            className="field mono h-8 w-56 text-sm"
            aria-label={t('netscan.target')}
            spellCheck={false}
            value={target}
            onChange={(e) => setTarget(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter' && !running) void scan();
            }}
          />
          {running ? (
            <button
              type="button"
              className="btn btn-ghost h-8"
              onClick={() => void api.netscanCancel()}
            >
              <Square size={13} aria-hidden /> {t('netscan.stop')}
            </button>
          ) : (
            <button
              type="button"
              className="btn btn-primary h-8"
              disabled={!check || reason !== null}
              onClick={() => void scan()}
            >
              <Radar size={14} aria-hidden /> {t('netscan.scan')}
            </button>
          )}
          <span className="text-xs text-muted">
            {reason
              ? t(`netscan.reason.${reason}` as TKey)
              : check && 'count' in check
                ? t('netscan.addresses', { count: check.count, iface: check.iface })
                : ''}
          </span>
        </div>
        {nets && nets.networks.length === 0 && (
          <p className="text-sm text-muted">{t('netscan.noNetworks')}</p>
        )}
        <p className="text-xs text-muted">
          {t('netscan.scopeNote', { ports: nets?.ports.join(', ') ?? '' })}
        </p>
        {progress && (
          <div role="status">
            <div className="mb-1 flex justify-between text-xs text-secondary">
              <span>{t(`netscan.stage.${progress.stage}` as TKey)}</span>
              <span className="tabular">
                {progress.done}/{progress.total}
              </span>
            </div>
            <div className="h-1 overflow-hidden rounded-full bg-hover">
              <div
                className="h-full bg-accent transition-[width]"
                style={{ width: `${(progress.done / Math.max(1, progress.total)) * 100}%` }}
              />
            </div>
          </div>
        )}
        {result && (
          <>
            <div className="flex items-center gap-2 border-b border-subtle pb-2">
              <div
                role="group"
                aria-label={t('netscan.title')}
                className="flex gap-0.5 rounded-md bg-app p-0.5"
              >
                {(['grid', 'list', 'free'] as const).map((v) => (
                  <button
                    key={v}
                    type="button"
                    aria-pressed={view === v}
                    className={`h-6 rounded-sm px-2 text-xs ${view === v ? 'bg-selected text-primary' : 'text-secondary'}`}
                    onClick={() => setView(v)}
                  >
                    {t(`netscan.view.${v}` as TKey)}
                  </button>
                ))}
              </div>
              <span className="text-xs text-secondary">
                {t('netscan.summary', { used: used.length, free: freeCount })}
                {result.cancelled ? ` · ${t('netscan.cancelled')}` : ''}
                {!result.icmpAvailable ? ` · ${t('netscan.noIcmp')}` : ''}
              </span>
              <span className="flex-1" />
              <button
                type="button"
                className="icon-btn icon-btn-sm"
                aria-label={t('analyzer.exportCsv')}
                title={t('analyzer.exportCsv')}
                onClick={() => void exportCsv()}
              >
                <Download size={13} />
              </button>
            </div>
            {view === 'grid' && (
              <div className="flex flex-col gap-3">
                <div className="flex flex-wrap gap-3 text-[11px] text-secondary">
                  {(['used', 'free', 'own', 'gateway'] as const).map((k) => (
                    <span key={k} className="flex items-center gap-1">
                      <span className="scan-cell inline-block h-3 w-3" data-kind={k} aria-hidden />{' '}
                      {t(`netscan.${k}` as TKey)}
                    </span>
                  ))}
                </div>
                {grids.map((g) => (
                  <div key={g.base}>
                    <p className="mono mb-1 text-xs text-muted">{g.base}/24</p>
                    <div
                      className="scan-grid grid gap-px"
                      style={{ gridTemplateColumns: 'repeat(32, minmax(0, 1fr))' }}
                    >
                      {g.cells.map((c) => (
                        <button
                          key={c.ip}
                          type="button"
                          className="scan-cell aspect-square"
                          data-kind={c.kind}
                          data-picked={picked?.ip === c.ip || undefined}
                          disabled={c.kind === 'outside'}
                          title={c.host ? tip(c.host) : c.ip}
                          aria-label={c.host ? `${c.ip}, ${status(c.host)}` : c.ip}
                          onClick={() => c.host && setPicked(c.host)}
                          onContextMenu={(e) => c.host && openMenu(e, c.host)}
                        />
                      ))}
                    </div>
                  </div>
                ))}
                {picked && (
                  <div className="rounded-sm border border-subtle p-2 text-xs">
                    <p className="mono text-sm">
                      {picked.ip}{' '}
                      {picked.hostname && (
                        <span className="text-secondary">· {picked.hostname}</span>
                      )}
                    </p>
                    <p className="text-muted">
                      {[status(picked), picked.mac, picked.vendor, picked.openPorts.join(', ')]
                        .filter(Boolean)
                        .join(' · ')}
                    </p>
                    {picked.alive && !picked.own && (
                      <div className="mt-2 flex flex-wrap gap-1">
                        {entries(picked)
                          .filter(
                            (e): e is Extract<MenuEntry, { onSelect: () => void }> =>
                              'onSelect' in e,
                          )
                          .map((e) => (
                            <button
                              key={e.label}
                              type="button"
                              className="btn btn-ghost h-7 text-xs"
                              disabled={e.disabled}
                              onClick={e.onSelect}
                            >
                              {e.label}
                            </button>
                          ))}
                      </div>
                    )}
                  </div>
                )}
              </div>
            )}
            {view === 'list' && (
              <div className="max-h-[50vh] overflow-auto">
                <table className="w-full text-xs">
                  <thead className="sticky top-0 bg-surface text-left text-secondary">
                    <tr>
                      {(['ip', 'status', 'hostname', 'mac', 'vendor', 'ports'] as const).map(
                        (c) => (
                          <th key={c} className="px-2 py-1 font-medium">
                            {t(`netscan.col.${c}` as TKey)}
                          </th>
                        ),
                      )}
                    </tr>
                  </thead>
                  <tbody>
                    {[...used]
                      .sort((a, b) => ipToNum(a.ip) - ipToNum(b.ip))
                      .map((h) => (
                        <tr
                          key={h.ip}
                          className="hover:bg-hover"
                          onContextMenu={(e) => openMenu(e, h)}
                          onDoubleClick={() =>
                            !h.own &&
                            connect(
                              h,
                              h.openPorts.includes(23) && !h.openPorts.includes(22)
                                ? 'telnet'
                                : 'ssh',
                            )
                          }
                        >
                          <td className="mono px-2 py-0.5">{h.ip}</td>
                          <td className="px-2 py-0.5">{status(h)}</td>
                          <td className="px-2 py-0.5">{h.hostname ?? ''}</td>
                          <td className="mono px-2 py-0.5">{h.mac ?? ''}</td>
                          <td className="px-2 py-0.5">{h.vendor ?? ''}</td>
                          <td className="mono px-2 py-0.5">{h.openPorts.join(', ')}</td>
                        </tr>
                      ))}
                  </tbody>
                </table>
              </div>
            )}
            {view === 'free' && (
              <div className="max-h-[50vh] overflow-auto">
                <p className="mb-2 text-xs text-muted">{t('netscan.freeNote')}</p>
                <ul className="flex flex-col gap-0.5">
                  {free.map((b) => (
                    <li
                      key={b.first}
                      className="flex items-center gap-2 rounded-sm px-1 py-0.5 hover:bg-hover"
                    >
                      <span className="mono min-w-0 flex-1 truncate text-sm">{blockLabel(b)}</span>
                      <span className="text-xs text-muted">
                        {t('netscan.blockCount', { count: b.count })}
                      </span>
                      <button
                        type="button"
                        className="btn btn-ghost h-6 text-xs"
                        onClick={() => copy(blockLabel(b))}
                      >
                        <Copy size={11} aria-hidden /> {t('netscan.copyRange')}
                      </button>
                      <button
                        type="button"
                        className="btn btn-ghost h-6 text-xs"
                        onClick={() => copy(blockAddresses(b))}
                      >
                        {t('netscan.copyAll')}
                      </button>
                    </li>
                  ))}
                </ul>
              </div>
            )}
          </>
        )}
      </div>
      {menu.at && picked && (
        <Menu
          x={menu.at.x}
          y={menu.at.y}
          entries={entries(picked)}
          onClose={menu.close}
          label={picked.ip}
        />
      )}
    </Dialog>
  );
}
