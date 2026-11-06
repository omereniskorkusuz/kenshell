/** Side-by-side config diff (P4-T08): changes with context, or every line. */
import { useMemo, useState } from 'react';
import { useT } from '../i18n';
import { foldDiff } from '../lib/diffView';
import type { ConfigDiff } from '../lib/ipc';

export function DiffView({
  diff,
  leftTitle,
  rightTitle,
}: {
  diff: ConfigDiff;
  leftTitle: string;
  rightTitle: string;
}) {
  const t = useT();
  const [all, setAll] = useState(false);
  const rows = useMemo(
    () => (all ? diff.rows.map((row) => ({ type: 'row' as const, row })) : foldDiff(diff.rows)),
    [diff, all],
  );
  const identical = diff.added + diff.removed + diff.changed === 0;
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex items-center gap-2 px-2 py-1.5 text-xs">
        <span className="tone-text" data-tone="success">
          +{diff.added}
        </span>
        <span className="tone-text" data-tone="danger">
          −{diff.removed}
        </span>
        <span className="tone-text" data-tone="warning">
          ~{diff.changed}
        </span>
        {diff.coarse && <span className="text-muted">{t('backups.coarse')}</span>}
        <span className="flex-1" />
        <label className="flex items-center gap-1 text-secondary">
          <input type="checkbox" checked={all} onChange={(e) => setAll(e.target.checked)} />
          {t('backups.allLines')}
        </label>
      </div>
      <div className="grid grid-cols-2 border-y border-subtle text-[11px] text-secondary">
        <span className="mono truncate px-2 py-1">{leftTitle}</span>
        <span className="mono truncate border-l border-subtle px-2 py-1">{rightTitle}</span>
      </div>
      {identical ? (
        <p className="p-4 text-center text-sm text-muted">{t('backups.identical')}</p>
      ) : (
        <div className="mono min-h-0 flex-1 overflow-auto text-[11px] leading-5">
          {rows.map((d, i) =>
            d.type === 'fold' ? (
              <div key={i} className="diff-fold px-2 text-center text-muted">
                {t('backups.folded', { count: d.count })}
              </div>
            ) : (
              <div key={i} className="grid grid-cols-2" data-kind={d.row.kind}>
                <div className="diff-cell diff-left flex min-w-0 gap-2 overflow-hidden px-1">
                  <span className="w-8 shrink-0 text-right text-muted">{d.row.leftNo ?? ''}</span>
                  <span className="min-w-0 break-all whitespace-pre-wrap">{d.row.left ?? ''}</span>
                </div>
                <div className="diff-cell diff-right flex min-w-0 gap-2 overflow-hidden border-l border-subtle px-1">
                  <span className="w-8 shrink-0 text-right text-muted">{d.row.rightNo ?? ''}</span>
                  <span className="min-w-0 break-all whitespace-pre-wrap">{d.row.right ?? ''}</span>
                </div>
              </div>
            ),
          )}
        </div>
      )}
    </div>
  );
}
