/**
 * "Scan my network" (P4-T14, R-T24; lazy chunk): pick one of this computer's subnets or
 * type a routed private network (up to a /16), scan it (discovery only — the backend
 * enforces the scope rules), then see a grid, a searchable list with response times and
 * names, the probably-free address blocks, and the last 20 saved scans. Right-click a
 * used address to connect, save it as a session or open its web page. CSV export.
 */
import { Copy, Download, Radar, Square, Trash2 } from 'lucide-react';
import { useCallback, useEffect, useMemo, useState, type MouseEvent } from 'react';
import { Dialog } from '../components/Dialog';
import { Menu, useMenu, type MenuEntry } from '../components/Menu';
import { useT, type TKey } from '../i18n';
import {
  api,
  DEFAULT_PORTS,
  DEFAULT_SSH,
  netscanRun,
  type NetProtocol,
  type ScanCheck,
  type ScanNetworks,
  type ScanProgress,
  type ScanResult,
  type ScanSummary,
  type ScopeReason,
} from '../lib/ipc';
import {
  blockAddresses,
  blockLabel,
  csvRows,
  CSV_HEADER,
  filterHosts,
  freeBlocks,
  GRID_ALL_BLOCKS,
  gridCells,
  ipToNum,
  rttLabel,
  type ScanHost,
} from '../lib/netscan';
import { useTabs } from '../stores/tabs';
import { useUi } from '../stores/ui';

type View = 'grid' | 'list' | 'free' | 'history';

const dateTime = (ms: number) =>
  new Date(ms).toLocaleString(document.documentElement.lang || undefined, {
    dateStyle: 'short',
    timeStyle: 'short',
  });

const seconds = (ms: number) => (ms / 1000).toFixed(ms < 10_000 ? 1 : 0);

/** Blocks a range covers: a wide one opens on the list, not the grid. */
const blocksOf = (r: ScanResult) => {
  const first = ipToNum(r.range.first);
  // Plain arithmetic: `&` would turn addresses above 128.0.0.0 negative.
  return Math.floor((ipToNum(r.range.last) - (first - (first % 256))) / 256) + 1;
};

export default function NetScan() {
  const t = useT();
  const close = () => useUi.getState().setScanOpen(false);
  const [nets, setNets] = useState<ScanNetworks | null>(null);
  const [target, setTarget] = useState('');
  const [check, setCheck] = useState<ScanCheck | { reason: ScopeReason } | null>(null);
  const [resolveDns, setResolveDns] = useState(true);
  const [progress, setProgress] = useState<ScanProgress | null>(null);
  const [result, setResult] = useState<ScanResult | null>(null);
  /** The saved scan on screen (null = the scan just run). */
  const [saved, setSaved] = useState<ScanSummary | null>(null);
  const [history, setHistory] = useState<ScanSummary[]>([]);
  const [view, setView] = useState<View>('grid');
  const [query, setQuery] = useState('');
  const [picked, setPicked] = useState<ScanHost | null>(null);
  const menu = useMenu();
  const running = progress !== null;

  const loadHistory = useCallback(
    () =>
      void api
        .netscanHistory()
        .then(setHistory)
        .catch(() => {}),
    [],
  );

  useEffect(() => {
    void api.netscanNetworks().then((n) => {
      setNets(n);
      if (n.defaultTarget) setTarget(n.defaultTarget);
    });
    loadHistory();
  }, [loadHistory]);

  // Scope check as the target changes (the backend rule; "Scan" stays off when it fails).
  useEffect(() => {
    if (!target.trim()) return;
    let live = true;
    const timer = window.setTimeout(() => {
      api
        .netscanCheck(target)
        .then((c) => live && setCheck(c))
        .catch(
          (e: { reason?: ScopeReason }) => live && setCheck({ reason: e?.reason ?? 'invalid' }),
        );
    }, 150);
    return () => {
      live = false;
      window.clearTimeout(timer);
    };
  }, [target]);

  const show = (r: ScanResult, from: ScanSummary | null) => {
    setResult(r);
    setSaved(from);
    setPicked(null);
    setQuery('');
    setView(blocksOf(r) <= GRID_ALL_BLOCKS ? 'grid' : 'list');
  };

  const scan = async () => {
    if (!check || 'reason' in check) return;
    setResult(null);
    setSaved(null);
    setPicked(null);
    setProgress({ stage: 'sweep', done: 0, total: check.count });
    try {
      show(await netscanRun(target, { resolveDns }, setProgress), null);
      loadHistory();
    } catch (e) {
      const reason = (e as { reason?: ScopeReason }).reason;
      if (reason) setCheck({ reason });
      else useUi.getState().flash(t('netscan.busy'));
    } finally {
      setProgress(null);
    }
  };

  const openSaved = (s: ScanSummary) =>
    void api
      .netscanHistoryGet(s.id)
      .then((r) => (r ? show(r, s) : loadHistory()))
      .catch(() => {});

  const removeSaved = (s: ScanSummary) =>
    void api
      .netscanHistoryDelete(s.id)
      .then(() => {
        if (saved?.id === s.id) {
          setResult(null);
          setSaved(null);
        }
        loadHistory();
      })
      .catch(() => {});

  const hosts = useMemo(() => result?.hosts ?? [], [result]);
  const free = useMemo(() => (result ? freeBlocks(result.range, hosts) : []), [result, hosts]);
  const grids = useMemo(() => (result ? gridCells(result.range, hosts) : []), [result, hosts]);
  const shown = useMemo(
    () => filterHosts(hosts, query).sort((a, b) => ipToNum(a.ip) - ipToNum(b.ip)),
    [hosts, query],
  );
  const total = result ? ipToNum(result.range.last) - ipToNum(result.range.first) + 1 : 0;

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
    if (h.own) return;
    e.preventDefault();
    setPicked(h);
    menu.openAt(e.clientX, e.clientY);
  };

  const status = (h: ScanHost) =>
    t(h.own ? 'netscan.own' : h.gateway ? 'netscan.gateway' : 'netscan.used');

  const exportCsv = async () => {
    const name = (saved?.target ?? target).replace(/[^\w.-]+/g, '_');
    const path = await api.pickPath('save', ['csv'], `scan_${name}.csv`).catch(() => null);
    if (!path) return;
    await api
      .analyzerExportCsv(path, [...CSV_HEADER], csvRows(hosts, status))
      .then(() => useUi.getState().flash(t('analyzer.exported', { path })))
      .catch(() => {});
  };

  const reason = check && 'reason' in check ? check.reason : null;
  const ok = check && !('reason' in check) ? check : null;
  const tip = (h: ScanHost) =>
    [
      h.ip,
      h.hostname,
      rttLabel(h),
      h.mac,
      h.vendor,
      h.openPorts.length ? `${t('netscan.ports')}: ${h.openPorts.join(', ')}` : null,
      status(h),
    ]
      .filter(Boolean)
      .join('\n');

  const views: View[] = result ? ['grid', 'list', 'free', 'history'] : ['history'];
  const current: View = result ? view : 'history';

  return (
    <Dialog title={t('netscan.title')} onClose={close} width="900px">
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
            placeholder={t('netscan.targetPlaceholder')}
            maxLength={64}
            spellCheck={false}
            value={target}
            onChange={(e) => setTarget(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter' && !running) void scan();
            }}
          />
          <label className="flex items-center gap-1.5 text-xs text-secondary">
            <input
              type="checkbox"
              checked={resolveDns}
              disabled={running}
              onChange={(e) => setResolveDns(e.target.checked)}
            />
            {t('netscan.resolveDns')}
          </label>
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
              disabled={!ok}
              onClick={() => void scan()}
            >
              <Radar size={14} aria-hidden /> {t('netscan.scan')}
            </button>
          )}
        </div>
        <p className="text-xs text-muted">
          {reason
            ? t(`netscan.reason.${reason}` as TKey)
            : ok
              ? [
                  ok.iface
                    ? t('netscan.addresses', { count: ok.count, iface: ok.iface })
                    : t('netscan.routed', { count: ok.count }),
                  ok.fullTcp ? null : t('netscan.pingOnly'),
                ]
                  .filter(Boolean)
                  .join(' · ')
              : ''}
        </p>
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
        <div className="flex flex-wrap items-center gap-2 border-b border-subtle pb-2">
          <div
            role="group"
            aria-label={t('netscan.title')}
            className="flex gap-0.5 rounded-md bg-app p-0.5"
          >
            {views.map((v) => (
              <button
                key={v}
                type="button"
                aria-pressed={current === v}
                className={`h-6 rounded-sm px-2 text-xs ${current === v ? 'bg-selected text-primary' : 'text-secondary'}`}
                onClick={() => setView(v)}
              >
                {t(`netscan.view.${v}` as TKey)}
                {v === 'history' && history.length > 0 ? ` (${history.length})` : ''}
              </button>
            ))}
          </div>
          {result && (
            <span className="text-xs text-secondary">
              {saved && `${t('netscan.saved', { date: dateTime(saved.startedAt) })} · `}
              {t('netscan.summary', {
                alive: hosts.length,
                total,
                secs: seconds(result.durationMs),
              })}
              {` · ${t(`netscan.method.${result.method}` as TKey)}`}
              {result.cancelled ? ` · ${t('netscan.cancelled')}` : ''}
              {saved?.truncated ? ` · ${t('netscan.truncated')}` : ''}
            </span>
          )}
          <span className="flex-1" />
          {result && (
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
        </div>
        {result && !result.local && current !== 'history' && (
          <p className="text-xs text-muted">{t('netscan.routedNote')}</p>
        )}
        {result && hosts.length === 0 && current !== 'history' && (
          <p className="text-sm text-muted">{t('netscan.noHosts')}</p>
        )}
        {current === 'grid' && result && (
          <div className="flex max-h-[50vh] flex-col gap-3 overflow-auto">
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
                      onClick={() => setPicked(c.host ?? null)}
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
                  {picked.hostname && <span className="text-secondary">· {picked.hostname}</span>}
                </p>
                <p className="text-muted">
                  {[
                    status(picked),
                    rttLabel(picked),
                    picked.mac,
                    picked.vendor,
                    picked.openPorts.join(', '),
                  ]
                    .filter(Boolean)
                    .join(' · ')}
                </p>
                {!picked.own && (
                  <div className="mt-2 flex flex-wrap gap-1">
                    {entries(picked)
                      .filter(
                        (e): e is Extract<MenuEntry, { onSelect: () => void }> => 'onSelect' in e,
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
        {current === 'list' && result && (
          <div className="flex flex-col gap-2">
            <input
              className="field h-7 w-72 text-xs"
              type="search"
              aria-label={t('netscan.search')}
              placeholder={t('netscan.search')}
              value={query}
              onChange={(e) => setQuery(e.target.value)}
            />
            <div className="max-h-[50vh] overflow-auto">
              <table className="w-full text-xs">
                <thead className="sticky top-0 bg-surface text-left text-secondary">
                  <tr>
                    {(['ip', 'rtt', 'hostname', 'mac', 'vendor', 'ports', 'status'] as const).map(
                      (c) => (
                        <th key={c} className="px-2 py-1 font-medium">
                          {t(`netscan.col.${c}` as TKey)}
                        </th>
                      ),
                    )}
                  </tr>
                </thead>
                <tbody>
                  {shown.map((h) => (
                    <tr
                      key={h.ip}
                      className="hover:bg-hover"
                      onContextMenu={(e) => openMenu(e, h)}
                      onDoubleClick={() =>
                        !h.own &&
                        connect(
                          h,
                          h.openPorts.includes(23) && !h.openPorts.includes(22) ? 'telnet' : 'ssh',
                        )
                      }
                    >
                      <td className="mono px-2 py-0.5">{h.ip}</td>
                      <td className="tabular px-2 py-0.5">{rttLabel(h) ?? ''}</td>
                      <td className="px-2 py-0.5">{h.hostname ?? ''}</td>
                      <td className="mono px-2 py-0.5">{h.mac ?? ''}</td>
                      <td className="px-2 py-0.5">{h.vendor ?? ''}</td>
                      <td className="mono px-2 py-0.5">{h.openPorts.join(', ')}</td>
                      <td className="px-2 py-0.5">{status(h)}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          </div>
        )}
        {current === 'free' && result && (
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
                    disabled={b.count > 4096}
                    onClick={() => copy(blockAddresses(b))}
                  >
                    {t('netscan.copyAll')}
                  </button>
                </li>
              ))}
            </ul>
          </div>
        )}
        {current === 'history' && (
          <div className="max-h-[50vh] overflow-auto">
            <p className="mb-2 text-xs text-muted">{t('netscan.historyNote')}</p>
            {history.length === 0 ? (
              <p className="text-sm text-muted">{t('netscan.historyEmpty')}</p>
            ) : (
              <table className="w-full text-xs">
                <thead className="sticky top-0 bg-surface text-left text-secondary">
                  <tr>
                    {(['date', 'target', 'state', 'hosts', 'duration'] as const).map((c) => (
                      <th key={c} className="px-2 py-1 font-medium">
                        {t(`netscan.hist.${c}` as TKey)}
                      </th>
                    ))}
                    <th />
                  </tr>
                </thead>
                <tbody>
                  {history.map((s) => (
                    <tr
                      key={s.id}
                      className={`hover:bg-hover ${saved?.id === s.id ? 'bg-selected' : ''}`}
                    >
                      <td className="tabular px-2 py-0.5">{dateTime(s.startedAt)}</td>
                      <td className="mono px-2 py-0.5">{s.target}</td>
                      <td className="px-2 py-0.5">
                        {t(s.cancelled ? 'netscan.hist.cancelled' : 'netscan.hist.done')}
                      </td>
                      <td className="tabular px-2 py-0.5">
                        {s.alive} / {s.total}
                      </td>
                      <td className="tabular px-2 py-0.5">
                        {t('netscan.secs', { secs: seconds(s.durationMs) })}
                      </td>
                      <td className="flex justify-end gap-1 px-2 py-0.5">
                        <button
                          type="button"
                          className="btn btn-ghost h-6 text-xs"
                          disabled={running}
                          onClick={() => openSaved(s)}
                        >
                          {t('netscan.hist.view')}
                        </button>
                        <button
                          type="button"
                          className="icon-btn icon-btn-sm"
                          aria-label={t('netscan.hist.delete')}
                          title={t('netscan.hist.delete')}
                          onClick={() => removeSaved(s)}
                        >
                          <Trash2 size={12} />
                        </button>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            )}
          </div>
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
