/**
 * Network tools: subnet calculator and MAC formats + vendor (offline OUI, P5-T04);
 * ping / traceroute from this computer (P5-T05). Results update as you type; clicking
 * a value copies it.
 */
import { Play, Square } from 'lucide-react';
import { useEffect, useMemo, useState } from 'react';
import { useT, type TKey } from '../i18n';
import { pingStats, traceHops } from '../lib/diag';
import { describeError } from '../lib/errors';
import { api } from '../lib/ipc';
import { macFormats, subnet } from '../lib/nettools';
import { useDiag } from '../stores/diag';
import { toast } from '../stores/toasts';

function Value({ label, value }: { label: string; value: string }) {
  const t = useT();
  return (
    <>
      <dt className="text-muted">{label}</dt>
      <dd>
        <button
          type="button"
          className="mono break-all text-left hover:underline"
          title={t('nettools.clickToCopy')}
          onClick={() => {
            void api
              .clipboardWrite(value)
              .then(() => toast({ kind: 'info', message: t('netscan.copied') }))
              .catch(() => {});
          }}
        >
          {value}
        </button>
      </dd>
    </>
  );
}

function SubnetCalc() {
  const t = useT();
  const [input, setInput] = useState('');
  const s = input.trim() ? subnet(input) : null;
  return (
    <section className="flex flex-col gap-2">
      <h3 className="text-xs font-medium text-secondary">{t('nettools.subnet')}</h3>
      <input
        className="field mono h-7 text-xs"
        value={input}
        spellCheck={false}
        aria-label={t('nettools.subnet')}
        placeholder={t('nettools.subnetPlaceholder')}
        onChange={(e) => setInput(e.target.value)}
      />
      {input.trim() && !s && <p className="text-xs text-muted">{t('nettools.subnetInvalid')}</p>}
      {s && (
        <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-3 gap-y-0.5 text-xs">
          <Value label={t('nettools.network')} value={`${s.network}/${s.prefix}`} />
          <Value label={t('nettools.mask')} value={s.mask} />
          <Value label={t('nettools.wildcard')} value={s.wildcard} />
          <Value label={t('nettools.broadcast')} value={s.broadcast} />
          <Value label={t('nettools.range')} value={`${s.firstHost} – ${s.lastHost}`} />
          <Value label={t('nettools.hosts')} value={s.hosts.toLocaleString()} />
          <Value label={t('nettools.total')} value={s.total.toLocaleString()} />
          <Value label={t('nettools.binary')} value={s.binaryMask} />
          <dt className="text-muted">{t('nettools.scope')}</dt>
          <dd>{t(`nettools.scopes.${s.scope}` as TKey)}</dd>
        </dl>
      )}
    </section>
  );
}

function MacTool() {
  const t = useT();
  const [input, setInput] = useState('');
  const [lookup, setLookup] = useState<{
    mac: string;
    vendor: string | null;
    tableSize: number;
  } | null>(null);
  const m = input.trim() ? macFormats(input) : null;
  const bare = m?.bare;
  // Only the answer for the MAC shown now counts.
  const vendor = lookup && lookup.mac === bare ? lookup : null;

  useEffect(() => {
    if (!bare) return;
    let live = true;
    void api
      .ouiLookup(bare)
      .then((v) => {
        if (live) setLookup({ mac: bare, ...v });
      })
      .catch(() => {});
    return () => {
      live = false;
    };
  }, [bare]);

  return (
    <section className="flex flex-col gap-2">
      <h3 className="text-xs font-medium text-secondary">{t('nettools.mac')}</h3>
      <input
        className="field mono h-7 text-xs"
        value={input}
        spellCheck={false}
        aria-label={t('nettools.mac')}
        placeholder={t('nettools.macPlaceholder')}
        onChange={(e) => setInput(e.target.value)}
      />
      {input.trim() && !m && <p className="text-xs text-muted">{t('nettools.macInvalid')}</p>}
      {m && (
        <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-3 gap-y-0.5 text-xs">
          <Value label={t('nettools.formats.cisco')} value={m.cisco} />
          <Value label={t('nettools.formats.colon')} value={m.colon} />
          <Value label={t('nettools.formats.dash')} value={m.dash} />
          <Value label={t('nettools.formats.bare')} value={m.bare} />
          <dt className="text-muted">{t('nettools.vendor')}</dt>
          <dd>{vendor?.vendor ?? t('nettools.vendorUnknown')}</dd>
          <dt className="text-muted" aria-hidden />
          <dd className="text-muted">
            {m.multicast
              ? t('nettools.group')
              : m.local
                ? t('nettools.local')
                : t('nettools.universal')}
          </dd>
        </dl>
      )}
      {vendor && vendor.tableSize < 1000 && (
        <p className="text-xs text-muted">{t('nettools.ouiSample')}</p>
      )}
    </section>
  );
}

const COUNTS = [4, 10, 100] as const;
const ms = (n: number) => (n < 10 ? n.toFixed(2) : n.toFixed(0));

function DiagTool() {
  const t = useT();
  const { kind, target, count, resolve, lines, running, exit, error } = useDiag();
  const stats = useMemo(() => (kind === 'ping' ? pingStats(lines) : null), [kind, lines]);
  const hops = useMemo(() => (kind === 'trace' ? traceHops(lines) : []), [kind, lines]);
  const set = useDiag.getState().setOptions;
  const busy = running !== null;
  return (
    <section className="flex flex-col gap-2">
      <h3 className="text-xs font-medium text-secondary">{t('nettools.diag')}</h3>
      <div className="flex gap-1" role="tablist" aria-label={t('nettools.diag')}>
        {(['ping', 'trace'] as const).map((k) => (
          <button
            key={k}
            type="button"
            role="tab"
            aria-selected={kind === k}
            disabled={busy}
            className={`btn h-7 flex-1 ${kind === k ? 'btn-primary' : 'btn-ghost'}`}
            onClick={() => set({ kind: k })}
          >
            {t(`nettools.${k}`)}
          </button>
        ))}
      </div>
      <form
        className="flex gap-1"
        onSubmit={(e) => {
          e.preventDefault();
          if (busy) useDiag.getState().stop();
          else if (target.trim()) void useDiag.getState().start();
        }}
      >
        <input
          className="field mono h-7 min-w-0 flex-1 text-xs"
          value={target}
          spellCheck={false}
          autoCapitalize="off"
          aria-label={t('nettools.target')}
          placeholder={t('nettools.target')}
          onChange={(e) => set({ target: e.target.value })}
        />
        <button
          type="submit"
          className={`btn h-7 ${busy ? 'btn-ghost' : 'btn-primary'}`}
          disabled={!busy && !target.trim()}
        >
          {busy ? <Square size={12} aria-hidden /> : <Play size={12} aria-hidden />}
          {busy ? t('nettools.stopRun') : t('nettools.run')}
        </button>
      </form>
      <div className="flex items-center gap-3 text-xs">
        {kind === 'ping' ? (
          <label className="flex items-center gap-1 text-secondary">
            {t('nettools.count')}
            <select
              className="field h-6 text-xs"
              value={count ?? 0}
              disabled={busy}
              onChange={(e) => set({ count: Number(e.target.value) || null })}
            >
              {COUNTS.map((c) => (
                <option key={c} value={c}>
                  {c}
                </option>
              ))}
              <option value={0}>{t('nettools.continuous')}</option>
            </select>
          </label>
        ) : (
          <label className="flex items-center gap-1">
            <input
              type="checkbox"
              checked={resolve}
              disabled={busy}
              onChange={(e) => set({ resolve: e.target.checked })}
            />
            {t('nettools.resolveNames')}
          </label>
        )}
      </div>
      {error !== null && (
        <p className="text-xs tone-text" data-tone="error">
          {describeError(error, t).message}
        </p>
      )}
      {stats && (
        <p className="mono text-xs" aria-live="polite">
          {t('nettools.replies', { count: stats.replies })} ·{' '}
          {t('nettools.rtt', { min: ms(stats.min), avg: ms(stats.avg), max: ms(stats.max) })}
        </p>
      )}
      {hops.length > 0 && (
        <table className="w-full text-xs">
          <tbody>
            {hops.map((h) => (
              <tr key={h.hop} className="align-top">
                <td className="w-6 pr-2 text-right text-muted">{h.hop}</td>
                <td className="mono break-all">
                  {h.text || <span className="text-muted">{t('nettools.noReply')}</span>}
                </td>
                <td className="mono pl-2 text-right whitespace-nowrap text-secondary">
                  {h.rtts.length ? `${ms(Math.min(...h.rtts))} ms` : '*'}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      {lines.length > 0 && (
        <details open={kind === 'ping'}>
          <summary className="cursor-pointer text-xs text-muted">{t('nettools.output')}</summary>
          <pre className="mono mt-1 max-h-60 overflow-auto rounded-sm border border-subtle p-2 text-[11px] leading-snug">
            {lines.join('\n')}
          </pre>
        </details>
      )}
      {!busy && exit !== undefined && (
        <p className="text-xs text-muted">
          {exit === null ? t('nettools.stopped') : t('nettools.finished', { code: exit })}
        </p>
      )}
    </section>
  );
}

export function NetTools() {
  return (
    <div className="flex flex-col gap-4 p-2">
      <DiagTool />
      <SubnetCalc />
      <MacTool />
    </div>
  );
}
