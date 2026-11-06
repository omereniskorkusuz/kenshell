/**
 * CDP / LLDP neighbours (P4-T10): read hidden (read-only), drawn as a star around this
 * device; pick a neighbour to connect to its management address (SSH / Telnet) or copy it.
 */
import { RotateCw, X } from 'lucide-react';
import { useEffect, useRef, useState } from 'react';
import { useT } from '../i18n';
import { describeError } from '../lib/errors';
import { api, DEFAULT_PORTS, DEFAULT_SSH, type Neighbor, type NetProtocol } from '../lib/ipc';
import type { LiveTab } from '../lib/sessionState';
import { shortName, starLayout } from '../lib/topology';
import { useTabs } from '../stores/tabs';

const SIZE = 300;

export function Topology({ tab, onClose }: { tab: LiveTab; onClose: () => void }) {
  const t = useT();
  const [list, setList] = useState<Neighbor[] | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [picked, setPicked] = useState<number | null>(null);
  const alive = useRef(true);
  const family = tab.device.vendor?.family ?? null;

  const run = async () => {
    if (tab.runtimeId === null) return;
    setBusy(true);
    setError(null);
    try {
      const n = await api.neighborsDiscover(tab.runtimeId, family);
      if (alive.current) {
        setList(n);
        setPicked(null);
      }
    } catch (e) {
      if (alive.current) setError(describeError(e, t).message);
    } finally {
      if (alive.current) setBusy(false);
    }
  };

  useEffect(() => {
    alive.current = true;
    queueMicrotask(() => void run());
    return () => {
      alive.current = false;
    };
    // Runs once per opening.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const connect = (n: Neighbor, protocol: NetProtocol) => {
    if (!n.ip) return;
    useTabs.getState().openTarget(
      {
        type: 'network',
        protocol,
        host: n.ip.split('/')[0] ?? n.ip,
        port: DEFAULT_PORTS[protocol],
        username: null,
        ssh: DEFAULT_SSH,
      },
      { name: shortName(n.name, 40) },
    );
  };

  const points = starLayout(list?.length ?? 0, SIZE);
  const sel = picked !== null ? list?.[picked] : undefined;
  const me = tab.device.hostname ?? tab.name;

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex items-center gap-1 border-b border-subtle p-2">
        <span className="min-w-0 flex-1 truncate text-sm font-medium">
          {t('topology.title')} · <span className="mono">{me}</span>
        </span>
        <button
          type="button"
          className="icon-btn icon-btn-sm"
          disabled={busy || tab.state !== 'connected'}
          aria-label={t('diag.rerun')}
          title={t('diag.rerun')}
          onClick={() => void run()}
        >
          <RotateCw size={13} />
        </button>
        <button
          type="button"
          className="icon-btn icon-btn-sm"
          aria-label={t('common.close')}
          onClick={onClose}
        >
          <X size={13} />
        </button>
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto p-2">
        {busy && (
          <p className="text-sm text-secondary" role="status">
            {t('topology.reading')}
          </p>
        )}
        {error && <p className="text-sm text-danger">{error}</p>}
        {list && list.length === 0 && <p className="text-sm text-muted">{t('topology.none')}</p>}
        {list && list.length > 0 && (
          <>
            <svg
              viewBox={`0 0 ${SIZE} ${SIZE}`}
              className="topology w-full"
              role="img"
              aria-label={t('topology.graph', { count: list.length })}
            >
              {points.map((p, i) => (
                <line
                  key={`l${i}`}
                  x1={SIZE / 2}
                  y1={SIZE / 2}
                  x2={p.x}
                  y2={p.y}
                  className="topo-edge"
                  data-on={picked === i || undefined}
                />
              ))}
              <g>
                <rect
                  x={SIZE / 2 - 44}
                  y={SIZE / 2 - 13}
                  width={88}
                  height={26}
                  rx={6}
                  className="topo-node topo-self"
                />
                <text x={SIZE / 2} y={SIZE / 2 + 4} textAnchor="middle" className="topo-label">
                  {shortName(me, 14)}
                </text>
              </g>
              {list.map((n, i) => {
                const p = points[i] ?? { x: 0, y: 0 };
                return (
                  <g
                    key={`n${i}`}
                    className="topo-pick"
                    role="button"
                    tabIndex={0}
                    aria-label={`${n.name}, ${n.localInterface}`}
                    onClick={() => setPicked(i)}
                    onKeyDown={(e) => {
                      if (e.key === 'Enter' || e.key === ' ') setPicked(i);
                    }}
                  >
                    <rect
                      x={p.x - 40}
                      y={p.y - 12}
                      width={80}
                      height={24}
                      rx={6}
                      className="topo-node"
                      data-on={picked === i || undefined}
                    />
                    <text x={p.x} y={p.y + 4} textAnchor="middle" className="topo-label">
                      {shortName(n.name, 12)}
                    </text>
                  </g>
                );
              })}
            </svg>
            {sel && (
              <div className="mt-2 rounded-sm border border-subtle p-2 text-xs">
                <p className="mono text-sm">{sel.name}</p>
                <p className="text-muted">
                  {sel.localInterface} → {sel.remoteInterface ?? '?'} ·{' '}
                  {[sel.cdp && 'CDP', sel.lldp && 'LLDP'].filter(Boolean).join(' + ')}
                </p>
                {sel.platform && <p className="truncate text-secondary">{sel.platform}</p>}
                <div className="mt-2 flex flex-wrap gap-1">
                  <button
                    type="button"
                    className="btn btn-primary h-7 text-xs"
                    disabled={!sel.ip}
                    onClick={() => connect(sel, 'ssh')}
                  >
                    {t('topology.ssh')}
                  </button>
                  <button
                    type="button"
                    className="btn btn-ghost h-7 text-xs"
                    disabled={!sel.ip}
                    onClick={() => connect(sel, 'telnet')}
                  >
                    Telnet
                  </button>
                  {sel.ip && (
                    <button
                      type="button"
                      className="btn btn-ghost h-7 text-xs"
                      onClick={() => void api.clipboardWrite(sel.ip ?? '').catch(() => {})}
                    >
                      {t('terminal.copy')} {sel.ip}
                    </button>
                  )}
                  {!sel.ip && <span className="self-center text-muted">{t('topology.noIp')}</span>}
                </div>
              </div>
            )}
            <ul className="mt-2">
              {list.map((n, i) => (
                <li key={`r${i}`}>
                  <button
                    type="button"
                    className="flex w-full items-center gap-2 rounded-sm px-1 py-0.5 text-left text-xs hover:bg-hover"
                    data-on={picked === i || undefined}
                    onClick={() => setPicked(i)}
                  >
                    <span className="mono w-32 shrink-0 truncate">{n.localInterface}</span>
                    <span className="min-w-0 flex-1 truncate">{n.name}</span>
                    <span className="mono shrink-0 text-muted">{n.ip ?? ''}</span>
                  </button>
                </li>
              ))}
            </ul>
          </>
        )}
      </div>
    </div>
  );
}
