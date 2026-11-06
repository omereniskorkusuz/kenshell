/**
 * Network tools (P5-T04): subnet calculator and MAC formats + vendor (offline OUI).
 * Results update as you type; clicking a value copies it.
 */
import { useEffect, useState } from 'react';
import { useT, type TKey } from '../i18n';
import { api } from '../lib/ipc';
import { macFormats, subnet } from '../lib/nettools';
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
            void navigator.clipboard
              ?.writeText(value)
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

export function NetTools() {
  return (
    <div className="flex flex-col gap-4 p-2">
      <SubnetCalc />
      <MacTool />
    </div>
  );
}
