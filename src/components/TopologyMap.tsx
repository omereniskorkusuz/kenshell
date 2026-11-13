/**
 * Topology map (its own tab): every open session's CDP / LLDP neighbours, MAC and ARP
 * tables merged into one hierarchical picture. Each device is its own node; every link
 * shows the port on both ends. Read hidden with read-only commands; nothing is typed
 * into a session.
 */
import {
  Boxes,
  Database,
  Maximize2,
  Minus,
  Monitor,
  Network,
  Phone,
  Plus,
  RotateCw,
  Router,
  Shield,
  Wifi,
  X,
  type LucideIcon,
} from 'lucide-react';
import {
  Fragment,
  useEffect,
  useMemo,
  useRef,
  useState,
  type PointerEvent as ReactPointerEvent,
} from 'react';
import { useT, type TKey } from '../i18n';
import { api, DEFAULT_PORTS, DEFAULT_SSH, type NetProtocol } from '../lib/ipc';
import type { LiveTab } from '../lib/sessionState';
import {
  buildGraph,
  formatMac,
  layoutGraph,
  NODE_H,
  NODE_W,
  type DeviceInput,
  type NodeKind,
  type TopoEdge,
  type TopoNode,
} from '../lib/topologyGraph';
import { useTabs } from '../stores/tabs';
import { useSettings } from '../stores/settings';
import {
  useTopology,
  withSightings,
  type Sightings,
  type SnmpEntry,
  type TopoEntry,
} from '../stores/topology';
import { useUi } from '../stores/ui';

const ICONS: Record<NodeKind, LucideIcon> = {
  router: Router,
  switch: Network,
  ap: Wifi,
  phone: Phone,
  firewall: Shield,
  host: Monitor,
  group: Boxes,
  hub: Network,
};

const PAD = 40;
const MIN_ZOOM = 0.25;
const MAX_ZOOM = 2;
const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v));

interface View {
  k: number;
  x: number;
  y: number;
}

/** Where the port labels sit: the parent's just under the card, the child's just above it;
 *  the horizontal run of the line is between them. */
const PORT_UP = 13;
/** Height of a port label row; crowded labels under a card take up to three rows. */
const PORT_ROW = 20;
/** Rough width of a port label (monospace 10px). */
const labelWidth = (text: string) => text.length * 6.2 + 12;
const PORT_DOWN = 14;

/** Rounded elbow from the bottom of the parent to the top of the child. */
function elbow(x1: number, y1: number, x2: number, y2: number): string {
  // Below the parent's port labels (up to three rows), above the child's.
  const mid = y1 + Math.min(PORT_UP + 3 * PORT_ROW, (y2 - y1) * 0.62);
  const dx = x2 - x1;
  const r = Math.min(12, Math.abs(dx) / 2, (y2 - y1) / 4);
  if (r < 1) return `M${x1},${y1}V${y2}`;
  const s = Math.sign(dx);
  return `M${x1},${y1}V${mid - r}Q${x1},${mid} ${x1 + s * r},${mid}H${x2 - s * r}Q${x2},${mid} ${x2},${mid + r}V${y2}`;
}

export default function TopologyMap() {
  const t = useT();
  // Terminal traffic changes `tabs` constantly; this text only when a session, name or
  // model changes, so the map is rebuilt only then.
  const identity = useTabs((s) => identityOf(s.tabs));
  const byTab = useTopology((s) => s.byTab);
  const hosts = useTopology((s) => s.hosts);
  const root = useTopology((s) => s.root);
  const snmp = useTopology((s) => s.snmp);
  const flow = useTopology((s) => s.flow);
  const sightings = useTopology((s) => s.sightings);
  const sessionSnmp = useTopology((s) => s.sessionSnmp);
  const refreshSecs = useSettings((s) => s.values['topology.refreshSecs']);
  const [picked, setPicked] = useState<string | null>(null);
  const [sourcesOpen, setSourcesOpen] = useState(false);
  const [view, setView] = useState<View>({ k: 1, x: PAD, y: PAD });
  const box = useRef<HTMLDivElement>(null);
  const drag = useRef<{ id: number; x: number; y: number } | null>(null);

  // Read sessions that are new or reconnected (never on a timer).
  useEffect(() => {
    useTopology.getState().refresh();
  }, [identity]);
  // Stored community, SNMP hosts and the flow collector (if switched on), once per opening.
  useEffect(() => {
    void useTopology.getState().loadSources();
  }, []);
  // The collector only counts; the map asks it what it has seen while it is on screen.
  const collecting = flow?.running === true;
  useEffect(() => {
    if (!collecting) return;
    const timer = setInterval(() => void useTopology.getState().pollFlow(), 5000);
    return () => clearInterval(timer);
  }, [collecting]);

  // Tables age out on the devices: read again every few minutes while the map is shown.
  useEffect(() => {
    if (refreshSecs <= 0) return;
    const timer = setInterval(() => useTopology.getState().refresh(true), refreshSecs * 1000);
    return () => clearInterval(timer);
  }, [refreshSecs]);

  const connected = useMemo(
    () => deviceInputs(identity, byTab, sightings, sessionSnmp),
    [identity, byTab, sightings, sessionSnmp],
  );
  const polled = useMemo(() => snmpInputs(snmp, sightings), [snmp, sightings]);
  const devices = useMemo(() => [...connected, ...polled], [connected, polled]);
  const flows = flow?.hosts;
  const graph = useMemo(() => buildGraph(devices, { hosts, flows }), [devices, hosts, flows]);
  const layout = useMemo(() => layoutGraph(graph, root), [graph, root]);
  const nodeById = useMemo(() => new Map(graph.nodes.map((n) => [n.id, n])), [graph]);

  const fit = () => {
    const el = box.current;
    if (!el) return;
    const k = clamp(
      Math.min(
        (el.clientWidth - PAD * 2) / layout.width,
        (el.clientHeight - PAD * 2) / layout.height,
      ),
      MIN_ZOOM,
      1,
    );
    setView({
      k,
      x: Math.max(PAD, (el.clientWidth - layout.width * k) / 2),
      y: PAD,
    });
  };
  // Fit again whenever the picture changes size.
  useEffect(fit, [layout.width, layout.height]);

  const zoom = (factor: number, cx?: number, cy?: number) =>
    setView((v) => {
      const el = box.current;
      const px = cx ?? (el ? el.clientWidth / 2 : 0);
      const py = cy ?? (el ? el.clientHeight / 2 : 0);
      const k = clamp(v.k * factor, MIN_ZOOM, MAX_ZOOM);
      // Keep the point under the pointer where it is.
      return { k, x: px - ((px - v.x) / v.k) * k, y: py - ((py - v.y) / v.k) * k };
    });

  const onPointerDown = (e: ReactPointerEvent<HTMLDivElement>) => {
    if (e.button !== 0 || (e.target as HTMLElement).closest('button')) return;
    drag.current = { id: e.pointerId, x: e.clientX, y: e.clientY };
    e.currentTarget.setPointerCapture(e.pointerId);
    setPicked(null);
  };
  const onPointerMove = (e: ReactPointerEvent<HTMLDivElement>) => {
    const d = drag.current;
    if (!d || d.id !== e.pointerId) return;
    const [dx, dy] = [e.clientX - d.x, e.clientY - d.y];
    drag.current = { id: d.id, x: e.clientX, y: e.clientY };
    setView((v) => ({ ...v, x: v.x + dx, y: v.y + dy }));
  };
  const endDrag = () => {
    drag.current = null;
  };

  const snmpHosts = Object.keys(snmp);
  const busy = [
    ...connected.filter((x) => byTab[x.key]?.busy).map((x) => x.key),
    ...snmpHosts.filter((h) => snmp[h]?.busy),
  ];
  const failed = [
    ...connected
      .filter((x) => byTab[x.key]?.error)
      .map((x) => ({ key: x.key, name: x.name, error: byTab[x.key]?.error ?? '' })),
    ...snmpHosts
      .filter((h) => snmp[h]?.error)
      .map((h) => ({ key: `snmp:${h}`, name: `SNMP ${h}`, error: snmp[h]?.error ?? '' })),
  ];
  const empty = devices.length === 0 && !flows?.length;
  /** Not seen for 10 minutes: drawn faded, with when it was seen last (relative to the
   *  newest read, so a render does not depend on the clock). */
  const latestRead = Math.max(0, ...Object.values(byTab).map((e) => e.readAt ?? 0));
  const stale = (n: TopoNode) => n.seenAt != null && latestRead - n.seenAt > 10 * 60_000;
  /** What a node's own device answered (sessions and SNMP), for the details panel. */
  const sourcesOf = (n: TopoNode): string[] => {
    const out = new Set<string>();
    if (n.tabKey) {
      for (const c of byTab[n.tabKey]?.discovery?.sources ?? []) out.add(c);
      for (const c of sessionSnmp[n.tabKey]?.device?.discovery.sources ?? []) out.add(c);
    }
    for (const [host, e] of Object.entries(snmp)) {
      if (e.device && (host.replace(/:\d+$/, '') === n.ip || e.device.name === n.name)) {
        for (const c of e.device.discovery.sources) out.add(c);
      }
    }
    return [...out];
  };
  const sel = picked ? nodeById.get(picked) : undefined;
  const center = (id: string) => {
    const p = layout.pos.get(id);
    return p ? { x: p.x + NODE_W / 2, y: p.y + NODE_H / 2, top: p.y, bottom: p.y + NODE_H } : null;
  };
  /** Each child leaves its parent at its own point along the card's bottom edge (ordered
   *  like the children), so every port label sits where its cable comes out. */
  /** Ports of an edge as seen from `from`. */
  const ports = (e: TopoEdge, from: string): [string | null, string | null] =>
    e.a === from ? [e.aPort, e.bPort] : [e.bPort, e.aPort];
  const rows = new Map<string, number>();
  const exits = (() => {
    const byParent = new Map<string, string[]>();
    for (const { edge, parent } of layout.tree) {
      byParent.set(parent, [...(byParent.get(parent) ?? []), edge.id]);
    }
    const out = new Map<string, number>();
    rows.clear();
    for (const [parent, ids] of byParent) {
      const p = layout.pos.get(parent);
      if (!p) continue;
      const kids = layout.tree.filter((t) => t.parent === parent);
      // Left to right as the children are placed.
      kids.sort((a, b) => (layout.pos.get(a.child)?.x ?? 0) - (layout.pos.get(b.child)?.x ?? 0));
      const n = ids.length;
      kids.forEach((k, i) => out.set(k.edge.id, p.x + (NODE_W * (i + 1)) / (n + 1)));
      // Labels that would touch go to alternating rows.
      const gap = NODE_W / (n + 1);
      const widest = Math.max(0, ...kids.map((k) => labelWidth(ports(k.edge, parent)[0] ?? '')));
      const count = Math.min(3, Math.max(1, Math.ceil(widest / gap)));
      kids.forEach((k, i) => rows.set(k.edge.id, i % count));
    }
    return out;
  })();
  const touches = (e: TopoEdge) => picked !== null && (e.a === picked || e.b === picked);
  const nameOf = (id: string) => {
    const n = nodeById.get(id);
    return n ? nodeTitle(n, t) : '';
  };

  return (
    <div className="flex h-full min-h-0 flex-col bg-app" data-testid="topology-map">
      <div className="flex shrink-0 flex-wrap items-center gap-2 border-b border-subtle bg-surface px-3 py-1.5">
        <h1 className="text-sm font-medium">{t('topoMap.title')}</h1>
        <span className="text-xs text-muted" role="status">
          {busy.length > 0
            ? t('topoMap.reading', { count: busy.length })
            : t('topoMap.summary', {
                devices: graph.nodes.filter((n) => n.kind !== 'group').length,
                links: graph.edges.length,
                sessions: connected.length,
              })}
          {polled.length > 0 && ` · ${t('topoMap.summarySnmp', { count: polled.length })}`}
          {collecting && ` · ${t('topoMap.summaryFlow', { count: flows?.length ?? 0 })}`}
        </span>
        <span className="flex-1" />
        <button
          type="button"
          className="btn btn-ghost btn-sm"
          aria-pressed={sourcesOpen}
          onClick={() => setSourcesOpen(!sourcesOpen)}
        >
          <Database size={12} aria-hidden /> {t('topoMap.sources')}
        </button>
        <label className="flex items-center gap-1.5 whitespace-nowrap text-xs text-secondary">
          <input
            type="checkbox"
            checked={hosts}
            onChange={(e) => useTopology.getState().setHosts(e.target.checked)}
          />
          <span title={t('topoMap.endDevicesHint')}>{t('topoMap.endDevices')}</span>
        </label>
        <button
          type="button"
          className="btn btn-ghost btn-sm"
          disabled={
            (devices.length === 0 && snmpHosts.length === 0 && !collecting) || busy.length > 0
          }
          onClick={() => useTopology.getState().refresh(true)}
        >
          <RotateCw size={12} aria-hidden /> {t('topoMap.refresh')}
        </button>
        <div className="flex items-center">
          <button
            type="button"
            className="icon-btn icon-btn-sm"
            aria-label={t('topoMap.zoomOut')}
            title={t('topoMap.zoomOut')}
            onClick={() => zoom(1 / 1.25)}
          >
            <Minus size={13} />
          </button>
          <span className="w-10 text-center text-xs tabular text-muted">
            {Math.round(view.k * 100)}%
          </span>
          <button
            type="button"
            className="icon-btn icon-btn-sm"
            aria-label={t('topoMap.zoomIn')}
            title={t('topoMap.zoomIn')}
            onClick={() => zoom(1.25)}
          >
            <Plus size={13} />
          </button>
          <button
            type="button"
            className="icon-btn icon-btn-sm"
            aria-label={t('topoMap.fit')}
            title={t('topoMap.fit')}
            onClick={fit}
          >
            <Maximize2 size={13} />
          </button>
        </div>
      </div>

      {failed.length > 0 && (
        <ul className="shrink-0 border-b border-subtle bg-surface px-3 py-1 text-xs text-warning">
          {failed.map((x) => (
            <li key={x.key}>{t('topoMap.failed', { name: x.name, error: x.error })}</li>
          ))}
        </ul>
      )}

      <div className="relative flex min-h-0 flex-1">
        <div
          ref={box}
          className="topo-canvas relative min-w-0 flex-1 overflow-hidden"
          onPointerDown={onPointerDown}
          onPointerMove={onPointerMove}
          onPointerUp={endDrag}
          onPointerCancel={endDrag}
          onWheel={(e) => {
            const r = e.currentTarget.getBoundingClientRect();
            if (e.ctrlKey || e.metaKey) {
              zoom(e.deltaY < 0 ? 1.1 : 1 / 1.1, e.clientX - r.left, e.clientY - r.top);
            } else {
              setView((v) => ({ ...v, x: v.x - e.deltaX, y: v.y - e.deltaY }));
            }
          }}
        >
          {empty ? (
            <p className="m-auto max-w-sm p-6 text-center text-sm text-muted">
              {t('topoMap.noSessions')}
            </p>
          ) : (
            <div
              className="absolute left-0 top-0 origin-top-left"
              style={{
                width: layout.width,
                height: layout.height,
                transform: `translate(${view.x}px, ${view.y}px) scale(${view.k})`,
              }}
            >
              <svg
                className="pointer-events-none absolute left-0 top-0 overflow-visible"
                width={layout.width}
                height={layout.height}
                aria-hidden
              >
                {layout.extra.map((e) => {
                  const [a, b] = [center(e.a), center(e.b)];
                  if (!a || !b) return null;
                  const bend = Math.max(40, Math.abs(a.x - b.x) / 4);
                  return (
                    <path
                      key={e.id}
                      className="topo-link topo-link-extra"
                      data-on={touches(e) || undefined}
                      d={`M${a.x},${a.bottom}C${a.x},${a.bottom + bend} ${b.x},${b.bottom + bend} ${b.x},${b.bottom}`}
                    />
                  );
                })}
                {layout.tree.map(({ edge, parent, child }) => {
                  const [p, c] = [center(parent), center(child)];
                  if (!p || !c) return null;
                  return (
                    <path
                      key={edge.id}
                      className="topo-link"
                      data-on={touches(edge) || undefined}
                      d={elbow(exits.get(edge.id) ?? p.x, p.bottom, c.x, c.top)}
                    />
                  );
                })}
              </svg>

              {layout.tree.map(({ edge, parent, child }) => {
                const [p, c] = [center(parent), center(child)];
                if (!p || !c) return null;
                const [up, down] = ports(edge, parent);
                if (!up && !down) return null;
                const x = exits.get(edge.id) ?? p.x;
                const title = t('topoMap.linkTitle', {
                  a: nameOf(parent),
                  aPort: up ?? '?',
                  b: nameOf(child),
                  bPort: down ?? '?',
                });
                return (
                  <Fragment key={edge.id}>
                    {up && (
                      <span
                        className="topo-port"
                        data-side="up"
                        data-on={touches(edge) || undefined}
                        style={{
                          left: x,
                          top: p.bottom + PORT_UP + (rows.get(edge.id) ?? 0) * PORT_ROW,
                        }}
                        title={title}
                      >
                        {up}
                      </span>
                    )}
                    {down && (
                      <span
                        className="topo-port"
                        data-side="down"
                        data-on={touches(edge) || undefined}
                        style={{ left: c.x, top: c.top - PORT_DOWN }}
                        title={title}
                      >
                        {down}
                      </span>
                    )}
                  </Fragment>
                );
              })}
              {layout.extra.map((e) => {
                const [a, b] = [center(e.a), center(e.b)];
                if (!a || !b) return null;
                const bend = Math.max(40, Math.abs(a.x - b.x) / 4);
                return (
                  <span
                    key={e.id}
                    className="topo-port"
                    data-on={touches(e) || undefined}
                    style={{
                      left: (a.x + b.x) / 2,
                      top: (a.bottom + b.bottom) / 2 + bend * 0.75,
                    }}
                    title={t('topoMap.linkTitle', {
                      a: nameOf(e.a),
                      aPort: e.aPort ?? '?',
                      b: nameOf(e.b),
                      bPort: e.bPort ?? '?',
                    })}
                  >
                    {e.aPort ?? '?'} ↔ {e.bPort ?? '?'}
                  </span>
                );
              })}

              {graph.nodes.map((n) => {
                const p = layout.pos.get(n.id);
                if (!p) return null;
                const Icon = ICONS[n.kind];
                return (
                  <button
                    key={n.id}
                    type="button"
                    className="topo-card"
                    data-kind={n.kind}
                    data-session={n.tabKey !== null || undefined}
                    data-source={n.source ?? undefined}
                    data-gateway={n.gateway || undefined}
                    data-stale={stale(n) || undefined}
                    title={
                      stale(n) && n.seenAt != null
                        ? t('topoMap.lastSeen', { time: new Date(n.seenAt).toLocaleString() })
                        : undefined
                    }
                    data-on={picked === n.id || undefined}
                    style={{ left: p.x, top: p.y, width: NODE_W, height: NODE_H }}
                    aria-pressed={picked === n.id}
                    onClick={() => setPicked(picked === n.id ? null : n.id)}
                    onDoubleClick={() => n.tabKey && useTabs.getState().activate(n.tabKey)}
                  >
                    <Icon size={22} className="topo-icon shrink-0" aria-hidden />
                    <span className="flex min-w-0 flex-1 flex-col text-left leading-tight">
                      <span className="mono truncate text-sm text-primary">{nodeTitle(n, t)}</span>
                      {n.kind !== 'group' && (
                        <>
                          <span className="truncate text-xs text-secondary">
                            {n.vendor ?? t(`topoMap.kind.${n.kind}` as TKey)}
                          </span>
                          {n.model && (
                            <span className="truncate text-xs text-muted">{n.model}</span>
                          )}
                          <span className="mono truncate text-xs text-muted">
                            {n.kind === 'host'
                              ? n.ip && n.mac
                                ? formatMac(n.mac)
                                : ''
                              : (n.ip ?? (n.mac ? formatMac(n.mac) : ''))}
                          </span>
                        </>
                      )}
                    </span>
                    {n.tabKey !== null && (
                      <span className="topo-live" title={t('topoMap.session')} aria-hidden />
                    )}
                    {n.gateway && <span className="topo-gw">{t('topoMap.gateway')}</span>}
                    {n.tabKey === null && n.source !== null && (
                      <span className="topo-tag" title={t(`topoMap.source.${n.source}` as TKey)}>
                        {n.source === 'snmp' ? 'SNMP' : 'FLOW'}
                      </span>
                    )}
                  </button>
                );
              })}
            </div>
          )}
        </div>

        {sourcesOpen && <Sources onClose={() => setSourcesOpen(false)} />}
        {sel && (
          <aside
            className="absolute bottom-0 right-0 top-0 z-10 flex w-72 flex-col gap-3 overflow-y-auto border-l border-subtle bg-surface p-3 text-sm"
            style={{ boxShadow: 'var(--shadow-elevated)' }}
            aria-label={t('topoMap.details')}
          >
            <div className="flex items-start gap-2">
              <div className="min-w-0 flex-1">
                <h2 className="mono truncate text-sm font-medium">{nodeTitle(sel, t)}</h2>
                <p className="text-xs text-muted">
                  {t(`topoMap.kind.${sel.kind}` as TKey)}
                  {sel.source !== null && ` · ${t(`topoMap.source.${sel.source}` as TKey)}`}
                </p>
              </div>
              <button
                type="button"
                className="icon-btn icon-btn-sm"
                aria-label={t('common.close')}
                onClick={() => setPicked(null)}
              >
                <X size={13} />
              </button>
            </div>
            <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-xs">
              {(
                [
                  ['topoMap.vendor', sel.vendor],
                  ['topoMap.model', sel.model],
                  ['topoMap.ip', sel.ip],
                  ['topoMap.mac', sel.mac ? formatMac(sel.mac) : null],
                ] as const
              ).map(
                ([k, v]) =>
                  v && (
                    <div key={k} className="contents">
                      <dt className="text-muted">{t(k)}</dt>
                      <dd className={k === 'topoMap.ip' || k === 'topoMap.mac' ? 'mono' : ''}>
                        {v}
                      </dd>
                    </div>
                  ),
              )}
            </dl>
            {sel.gateway && <p className="text-xs text-secondary">{t('topoMap.gatewayHint')}</p>}
            {sel.seenAt != null && (
              <p className="text-xs text-muted">
                {t('topoMap.lastSeen', { time: new Date(sel.seenAt).toLocaleString() })}
              </p>
            )}
            {sourcesOf(sel).length > 0 && (
              <div>
                <h3 className="label-caps mb-1">{t('topoMap.readWith')}</h3>
                <ul className="mono flex flex-col gap-0.5 text-[11px] text-secondary">
                  {sourcesOf(sel).map((c) => (
                    <li key={c}>{c}</li>
                  ))}
                </ul>
              </div>
            )}
            <div>
              <h3 className="label-caps mb-1">{t('topoMap.links')}</h3>
              <ul className="flex flex-col gap-0.5 text-xs">
                {graph.edges
                  .filter((e) => e.a === sel.id || e.b === sel.id)
                  .map((e) => {
                    const other = e.a === sel.id ? e.b : e.a;
                    const [mine, theirs] = ports(e, sel.id);
                    return (
                      <li key={e.id}>
                        <button
                          type="button"
                          className="flex w-full items-baseline gap-2 rounded-sm px-1 py-0.5 text-left hover:bg-hover"
                          onClick={() => setPicked(other)}
                        >
                          <span className="mono w-20 shrink-0 truncate">{mine ?? '?'}</span>
                          <span className="min-w-0 flex-1 truncate">{nameOf(other)}</span>
                          <span className="mono shrink-0 text-muted">{theirs ?? ''}</span>
                        </button>
                        <p className="px-1 text-[10px] text-muted">
                          {e.via
                            .map((v) =>
                              t('topoMap.via', {
                                how: t(`topoMap.how.${v.how}` as TKey),
                                by: v.by,
                              }),
                            )
                            .join(' · ')}
                        </p>
                      </li>
                    );
                  })}
              </ul>
            </div>
            <div className="flex flex-wrap gap-1">
              {sel.tabKey !== null && (
                <button
                  type="button"
                  className="btn btn-primary btn-sm"
                  onClick={() => sel.tabKey && useTabs.getState().activate(sel.tabKey)}
                >
                  {t('topoMap.goToSession')}
                </button>
              )}
              {sel.tabKey === null && sel.ip && (
                <>
                  <button
                    type="button"
                    className="btn btn-primary btn-sm"
                    onClick={() => connectTo(sel, 'ssh')}
                  >
                    {t('topology.ssh')}
                  </button>
                  <button
                    type="button"
                    className="btn btn-ghost btn-sm"
                    onClick={() => connectTo(sel, 'telnet')}
                  >
                    Telnet
                  </button>
                </>
              )}
              {sel.ip && (
                <button
                  type="button"
                  className="btn btn-ghost btn-sm"
                  onClick={() => {
                    void api.clipboardWrite(sel.ip ?? '').catch(() => {});
                    useUi.getState().flash(t('topoMap.copied', { ip: sel.ip ?? '' }));
                  }}
                >
                  {t('topoMap.copyIp')}
                </button>
              )}
              {sel.kind !== 'group' && sel.kind !== 'host' && (
                <button
                  type="button"
                  className="btn btn-ghost btn-sm"
                  onClick={() => useTopology.getState().setRoot(root === sel.id ? null : sel.id)}
                >
                  {root === sel.id ? t('topoMap.autoRoot') : t('topoMap.setRoot')}
                </button>
              )}
            </div>
          </aside>
        )}
      </div>
    </div>
  );
}

const FIELD = '\u0001';
const ROW = '\u0002';

/** What the map shows of each connected session, as one comparable string. */
function identityOf(tabs: readonly LiveTab[]): string {
  return tabs
    .filter((x) => x.state === 'connected' && x.runtimeId !== null)
    .map((x) =>
      [
        x.key,
        x.runtimeId,
        x.device.hostname ?? x.name,
        x.target.type === 'network' ? x.target.host : '',
        x.device.facts?.model ?? '',
      ].join(FIELD),
    )
    .join(ROW);
}

function deviceInputs(
  identity: string,
  byTab: Record<string, TopoEntry>,
  sightings: Record<string, Sightings>,
  sessionSnmp: Record<string, SnmpEntry>,
): DeviceInput[] {
  if (identity === '') return [];
  return identity.split(ROW).flatMap((row) => {
    const [key = '', , name = '', host = '', model = ''] = row.split(FIELD);
    const read = byTab[key]?.discovery;
    const mem = read ? withSightings(read, sightings[key]) : null;
    const out: DeviceInput[] = [
      {
        key,
        name,
        host: host || null,
        model: model || null,
        discovery: mem?.discovery ?? null,
        seen: mem?.seen,
      },
    ];
    // The same device over SNMP: merged into the session's node by name / MAC / address.
    const polled = sessionSnmp[key]?.device;
    if (polled) {
      const m = withSightings(polled.discovery, sightings[`ss:${key}`]);
      out.push({
        key: `ss:${key}`,
        name: polled.name ?? name,
        host: host || null,
        model: polled.description?.slice(0, 80) ?? null,
        discovery: m.discovery,
        seen: m.seen,
        source: 'snmp',
        interfaces: polled.interfaces,
      });
    }
    return out;
  });
}

function snmpInputs(
  snmp: Record<string, SnmpEntry>,
  sightings: Record<string, Sightings>,
): DeviceInput[] {
  return Object.entries(snmp).flatMap(([host, e]) => {
    if (!e.device) return [];
    const m = withSightings(e.device.discovery, sightings[`snmp:${host}`]);
    return [
      {
        key: `snmp:${host}`,
        name: e.device.name ?? host,
        // `host:port` → the address alone.
        host: host.replace(/:\d+$/, ''),
        model: e.device.description?.slice(0, 80) ?? null,
        discovery: m.discovery,
        seen: m.seen,
        source: 'snmp' as const,
        interfaces: e.device.interfaces,
      },
    ];
  });
}

/**
 * The other sources of the map: SNMP agents to read and the flow collector. The
 * community is typed here once and goes straight to the keychain; it is never shown again.
 */
function Sources({ onClose }: { onClose: () => void }) {
  const t = useT();
  const community = useTopology((s) => s.community);
  const snmp = useTopology((s) => s.snmp);
  const flow = useTopology((s) => s.flow);
  const flowError = useTopology((s) => s.flowError);
  const hosts = useSettings((s) => s.values['topology.snmpHosts']);
  const flowEnabled = useSettings((s) => s.values['topology.flowEnabled']);
  const sflowPort = useSettings((s) => s.values['topology.sflowPort']);
  const netflowPort = useSettings((s) => s.values['topology.netflowPort']);
  const snmpSessions = useSettings((s) => s.values['topology.snmpSessions']);
  const refreshSecs = useSettings((s) => s.values['topology.refreshSecs']);
  const [secret, setSecret] = useState('');
  const [host, setHost] = useState('');
  const [failed, setFailed] = useState(false);
  const st = useTopology.getState();
  const running = flow?.running === true;

  const save = () => {
    if (secret === '') return;
    setFailed(false);
    st.setCommunity(secret).then(
      () => setSecret(''),
      () => setFailed(true),
    );
  };
  const add = () => {
    st.addSnmpHost(host);
    setHost('');
  };
  const port = (
    key: 'topology.sflowPort' | 'topology.netflowPort',
    label: string,
    value: number,
  ) => (
    <label className="flex items-center gap-1.5 text-xs text-secondary">
      {label}
      <input
        type="number"
        className="field h-7 w-20 text-xs"
        min={0}
        max={65535}
        value={value}
        disabled={running}
        onChange={(e) => useSettings.getState().set(key, Number(e.target.value))}
      />
    </label>
  );

  return (
    <aside
      className="absolute bottom-0 left-0 top-0 z-10 flex w-80 flex-col gap-4 overflow-y-auto border-r border-subtle bg-surface p-3 text-sm"
      style={{ boxShadow: 'var(--shadow-elevated)' }}
      aria-label={t('topoMap.sources')}
    >
      <div className="flex items-center">
        <h2 className="flex-1 text-sm font-medium">{t('topoMap.sources')}</h2>
        <button
          type="button"
          className="icon-btn icon-btn-sm"
          aria-label={t('common.close')}
          onClick={onClose}
        >
          <X size={13} />
        </button>
      </div>
      <p className="text-xs text-muted">{t('topoMap.sourcesHint')}</p>
      <details className="rounded-sm border border-subtle p-2 text-xs text-secondary">
        <summary className="cursor-pointer font-medium text-primary">
          {t('topoMap.howTitle')}
        </summary>
        <ul className="mt-2 flex list-disc flex-col gap-1.5 pl-4">
          <li>{t('topoMap.howLinks')}</li>
          <li>{t('topoMap.howHosts')}</li>
          <li>{t('topoMap.howArp')}</li>
          <li>{t('topoMap.howGateway')}</li>
          <li>{t('topoMap.howMemory')}</li>
        </ul>
      </details>
      <label className="flex items-center gap-1.5 text-xs">
        {t('topoMap.refreshEvery')}
        <select
          className="field h-7 text-xs"
          value={refreshSecs}
          onChange={(e) =>
            useSettings.getState().set('topology.refreshSecs', Number(e.target.value))
          }
        >
          {[0, 60, 120, 300, 600].map((v) => (
            <option key={v} value={v}>
              {v === 0 ? t('topoMap.refreshOff') : t('topoMap.refreshMinutes', { m: v / 60 })}
            </option>
          ))}
        </select>
      </label>

      <section className="flex flex-col gap-2">
        <h3 className="label-caps">{t('topoMap.snmpTitle')}</h3>
        <p className="text-xs text-muted">{t('topoMap.snmpHint')}</p>
        <label className="flex items-center gap-1.5 text-xs">
          <input
            type="checkbox"
            checked={snmpSessions}
            onChange={(e) => {
              useSettings.getState().set('topology.snmpSessions', e.target.checked);
              st.refresh(true);
            }}
          />
          {t('topoMap.snmpSessions')}
        </label>
        <div className="flex flex-wrap items-center gap-1">
          <input
            type="password"
            className="field h-7 min-w-0 flex-1 text-xs"
            aria-label={t('topoMap.community')}
            placeholder={community ? t('topoMap.communitySaved') : t('topoMap.community')}
            autoComplete="off"
            value={secret}
            onChange={(e) => setSecret(e.target.value)}
            onKeyDown={(e) => e.key === 'Enter' && save()}
          />
          <button type="button" className="btn btn-sm" disabled={secret === ''} onClick={save}>
            {t('topoMap.save')}
          </button>
          {community && (
            <button
              type="button"
              className="btn btn-ghost btn-sm"
              onClick={() => void st.clearCommunity()}
            >
              {t('topoMap.forget')}
            </button>
          )}
        </div>
        {failed && <p className="text-xs text-danger">{t('credentials.saveFailed')}</p>}
        <ul className="flex flex-col gap-0.5">
          {hosts.map((h) => {
            const e = snmp[h];
            return (
              <li key={h} className="flex items-center gap-2 text-xs">
                <span className="mono min-w-0 flex-1 truncate" title={h}>
                  {h}
                </span>
                <span
                  className={`min-w-0 max-w-[50%] truncate ${e?.error ? 'text-warning' : 'text-muted'}`}
                  title={e?.error ?? undefined}
                >
                  {e?.busy
                    ? t('topoMap.snmpReading')
                    : (e?.error ??
                      e?.device?.name ??
                      (community ? '' : t('topoMap.snmpNeedsCommunity')))}
                </span>
                <button
                  type="button"
                  className="icon-btn icon-btn-sm"
                  aria-label={t('topoMap.removeHost', { host: h })}
                  title={t('topoMap.removeHost', { host: h })}
                  onClick={() => st.removeSnmpHost(h)}
                >
                  <X size={12} />
                </button>
              </li>
            );
          })}
        </ul>
        <div className="flex items-center gap-1">
          <input
            className="field mono h-7 min-w-0 flex-1 text-xs"
            aria-label={t('topoMap.snmpHost')}
            placeholder={t('topoMap.snmpHost')}
            spellCheck={false}
            autoComplete="off"
            value={host}
            onChange={(e) => setHost(e.target.value)}
            onKeyDown={(e) => e.key === 'Enter' && add()}
          />
          <button type="button" className="btn btn-sm" disabled={host.trim() === ''} onClick={add}>
            {t('topoMap.add')}
          </button>
        </div>
      </section>

      <section className="flex flex-col gap-2">
        <h3 className="label-caps">{t('topoMap.flowTitle')}</h3>
        <p className="text-xs text-muted">{t('topoMap.flowHint')}</p>
        <label className="flex items-center gap-1.5 text-xs">
          <input
            type="checkbox"
            checked={flowEnabled}
            onChange={(e) => void st.setFlow(e.target.checked)}
          />
          {t('topoMap.flowListen')}
        </label>
        <div className="flex flex-wrap gap-3">
          {port('topology.sflowPort', 'sFlow UDP', sflowPort)}
          {port('topology.netflowPort', 'NetFlow / IPFIX UDP', netflowPort)}
        </div>
        {flowError && <p className="text-xs text-danger">{flowError}</p>}
        {running && (
          <p className="text-xs text-secondary" role="status">
            {t('topoMap.flowStatus', {
              sflow: flow.sflowDatagrams,
              netflow: flow.netflowDatagrams,
              hosts: flow.hosts.length,
            })}
          </p>
        )}
      </section>
    </aside>
  );
}

function nodeTitle(n: TopoNode, t: ReturnType<typeof useT>): string {
  if (n.kind === 'group') return t('topoMap.group', { count: n.count });
  if (n.kind === 'hub') return t('topoMap.hub');
  return n.name;
}

/** Opens a session to a device found on the map (the map tab steps aside). */
function connectTo(n: TopoNode, protocol: NetProtocol): void {
  if (!n.ip) return;
  useTabs.getState().openTarget(
    {
      type: 'network',
      protocol,
      host: n.ip,
      port: DEFAULT_PORTS[protocol],
      username: null,
      ssh: DEFAULT_SSH,
    },
    { name: n.name || n.ip },
  );
}
