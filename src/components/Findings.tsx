/** Findings with severity icons (diagnostics P4-T06, config audit P4-T07). */
import { AlertOctagon, AlertTriangle, CheckCircle2, Info } from 'lucide-react';
import { useT, type TKey } from '../i18n';
import type { Finding, Severity } from '../lib/ipc';

const ICON: Record<Severity, typeof Info> = {
  critical: AlertOctagon,
  warning: AlertTriangle,
  info: Info,
  ok: CheckCircle2,
};
const TONE: Record<Severity, string> = {
  critical: 'error',
  warning: 'warning',
  info: 'muted',
  ok: 'success',
};

/** `prefix`: i18n namespace of the codes (`diag`, `cfgRule`); `allGood` is shared. */
export function useFindingText(prefix: string) {
  const t = useT();
  return (f: Finding) =>
    t((f.code === 'allGood' ? 'diag.allGood' : `${prefix}.${f.code}`) as TKey, f.params);
}

export function FindingList({ findings, prefix }: { findings: Finding[]; prefix: string }) {
  const t = useT();
  const text = useFindingText(prefix);
  return (
    <ul className="flex flex-col gap-1">
      {findings.map((f, i) => {
        const Icon = ICON[f.severity];
        return (
          <li key={i} className="flex items-start gap-2 rounded-sm px-1 py-1 hover:bg-hover">
            <Icon
              size={14}
              className="mt-0.5 shrink-0 tone-text"
              data-tone={TONE[f.severity]}
              aria-label={t(`diag.sev.${f.severity}` as TKey)}
            />
            <div className="min-w-0 flex-1">
              <p className="text-sm">{text(f)}</p>
              {f.command && <p className="mono truncate text-[11px] text-muted">{f.command}</p>}
            </div>
          </li>
        );
      })}
    </ul>
  );
}
