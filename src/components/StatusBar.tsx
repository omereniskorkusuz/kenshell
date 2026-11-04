/**
 * Status bar: log state, RX/TX of the active tab, encoding, grid size, session summary and a
 * transient message slot. Byte counters are polled (2×/s) from the tab's sink so the data
 * path never triggers React renders.
 */
import { useEffect, useState } from 'react';
import { useT } from '../i18n';
import { formatBytes } from '../lib/format';
import { isProd } from '../lib/tree';
import { getSink } from '../terminal/sink';
import { useTabs } from '../stores/tabs';
import { useUi } from '../stores/ui';

interface Counters {
  rx: number;
  tx: number;
  cols: number;
  rows: number;
}

function useActiveCounters(key: string | null): Counters | null {
  const [c, setC] = useState<Counters | null>(null);
  useEffect(() => {
    if (!key) return;
    const read = () => {
      const s = getSink(key);
      setC((prev) => {
        if (!s) return null;
        if (
          prev &&
          prev.rx === s.rx &&
          prev.tx === s.tx &&
          prev.cols === s.cols &&
          prev.rows === s.rows
        ) {
          return prev;
        }
        return { rx: s.rx, tx: s.tx, cols: s.cols, rows: s.rows };
      });
    };
    const first = setTimeout(read, 0);
    const id = setInterval(read, 500);
    return () => {
      clearTimeout(first);
      clearInterval(id);
    };
  }, [key]);
  return key ? c : null;
}

export function StatusBar() {
  const t = useT();
  const activeKey = useTabs((s) => s.activeKey);
  const tabs = useTabs((s) => s.tabs);
  const message = useUi((s) => s.statusMessage);
  const counters = useActiveCounters(activeKey);
  const prodCount = tabs.filter((x) => isProd(x.envTag)).length;

  return (
    <footer
      className="flex shrink-0 items-center gap-4 border-t border-subtle bg-surface px-3 text-xs text-muted"
      style={{ height: 'var(--status-bar-height)' }}
    >
      <span>{t('status.logOff')}</span>
      {counters && (
        <>
          <span className="mono tabular" title={t('status.rxTxHint')}>
            RX {formatBytes(counters.rx)} · TX {formatBytes(counters.tx)}
          </span>
          <span>UTF-8</span>
          {counters.cols > 0 && (
            <span className="mono tabular" title={t('status.gridHint')}>
              {counters.cols}×{counters.rows}
            </span>
          )}
        </>
      )}
      {message && (
        <span className="truncate text-secondary" key={message.id}>
          {message.text}
        </span>
      )}
      <span className="ml-auto shrink-0">
        {tabs.length === 0
          ? t('shell.statusReady')
          : t('unit.session', { count: tabs.length }) +
            (prodCount > 0 ? ` · ${t('status.prodCount', { count: prodCount })}` : '')}
      </span>
    </footer>
  );
}
