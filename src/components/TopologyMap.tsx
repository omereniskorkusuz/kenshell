/**
 * Topology map (its own tab): every open session's CDP / LLDP neighbours, MAC and ARP
 * tables merged into one hierarchical picture. Each device is its own node; every link
 * shows the port on both ends. Read hidden with read-only commands; nothing is typed
 * into a session.
 */
import {
  Boxes,
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
import { useTopology, type TopoEntry } from '../stores/topology';
import { useUi } from '../stores/ui';

const ICONS: Record<NodeKind, LucideIcon> = {
  router: Router,
  switch: Network,
  ap: Wifi,
  phone: Phone,
  firewall: Shield,
  host: Monitor,
  group: Boxes,
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

/** Rounded elbow from the bottom of the parent to the top of the child. */
function elbow(x1: number, y1: number, x2: number, y2: number): string {
  const mid = y1 + (y2 - y1) * 0.4;
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
  const [picked, setPicked] = useState<string | null>(null);
  const [view, setView] = useState<View>({ k: 1, x: PAD, y: PAD });
  const box = useRef<HTMLDivElement>(null);
  const drag = useRef<{ id: number; x: number; y: number } | null>(null);

  // Read sessions that are new or reconnected (never on a timer).
  useEffect(() => {
    useTopology.getState().refresh();
  }, [identity]);
  const devices = useMemo(() => deviceInputs(identity, byTab), [identity, byTab]);
  const connected = devices;
  const graph = useMemo(() => buildGraph(devices, { hosts }), [devices, hosts]);
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

  const busy = connected.filter((x) => byTab[x.key]?.busy);
  const failed = connected.filter((x) => byTab[x.key]?.error);
  const sel = picked ? nodeById.get(picked) : undefined;
  const center = (id: string) => {
    const p = layout.pos.get(id);
    return p ? { x: p.x + NODE_W / 2, y: p.y + NODE_H / 2, top: p.y, bottom: p.y + NODE_H } : null;
  };
  const touches = (e: TopoEdge) => picked !== null && (e.a === picked || e.b === picked);
  const nameOf = (id: string) => {
    const n = nodeById.get(id);
    return n ? nodeTitle(n, t) : '';
  };
  /** Ports of an edge as seen from `from`. */
  const ports = (e: TopoEdge, from: string): [string | null, string | null] =>
    e.a === from ? [e.aPort, e.bPort] : [e.bPort, e.aPort];

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
        </span>
        <span className="flex-1" />
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
          disabled={connected.length === 0 || busy.length > 0}
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
            <li key={x.key}>
              {t('topoMap.failed', { name: x.name, error: byTab[x.key]?.error ?? '' })}
            </li>
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
          {connected.length === 0 ? (
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
                      d={elbow(p.x, p.bottom, c.x, c.top)}
                    />
                  );
                })}
              </svg>

              {layout.tree.map(({ edge, parent, child }) => {
                const [p, c] = [center(parent), center(child)];
                if (!p || !c) return null;
                const [up, down] = ports(edge, parent);
                if (!up && !down) return null;
                const mid = p.bottom + (c.top - p.bottom) * 0.4;
                return (
                  <span
                    key={edge.id}
                    className="topo-port"
                    data-on={touches(edge) || undefined}
                    style={{ left: c.x, top: mid + (c.top - mid) / 2 }}
                    title={t('topoMap.linkTitle', {
                      a: nameOf(parent),
                      aPort: up ?? '?',
                      b: nameOf(child),
                      bPort: down ?? '?',
                    })}
                  >
                    {up ?? '?'}
                    {down ? ` → ${down}` : ''}
                  </span>
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
                  </button>
                );
              })}
            </div>
          )}
        </div>

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
                  {sel.tabKey !== null && ` · ${t('topoMap.session')}`}
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

function deviceInputs(identity: string, byTab: Record<string, TopoEntry>): DeviceInput[] {
  if (identity === '') return [];
  return identity.split(ROW).map((row) => {
    const [key = '', , name = '', host = '', model = ''] = row.split(FIELD);
    return {
      key,
      name,
      host: host || null,
      model: model || null,
      discovery: byTab[key]?.discovery ?? null,
    };
  });
}

function nodeTitle(n: TopoNode, t: ReturnType<typeof useT>): string {
  return n.kind === 'group' ? t('topoMap.group', { count: n.count }) : n.name;
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
