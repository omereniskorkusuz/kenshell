/** Left sidebar: saved sessions, detected consoles, recent connections. Resizable 180–320px. */
import { useEffect, useMemo, useState } from 'react';
import { useT, getLocale } from '../i18n';
import { envKind, envLabel, envLetter, recentSessions } from '../lib/tree';
import { useLibrary } from '../stores/library';
import { LIMITS, useSettings } from '../stores/settings';
import { useTabs } from '../stores/tabs';
import { ConsoleList } from './ConsoleList';
import { ResizeHandle } from './ResizeHandle';
import { SessionTree } from './SessionTree';

export function Sidebar() {
  const t = useT();
  const collapsed = useSettings((s) => s.values['ui.sidebarCollapsed']);
  if (collapsed) return null;

  return (
    <div className="relative flex shrink-0" style={{ width: 'var(--sidebar-width)' }}>
      <nav
        aria-label={t('shell.sidebar')}
        className="scroll-thin flex min-w-0 flex-1 flex-col gap-3 overflow-x-hidden overflow-y-auto border-r border-subtle bg-surface py-2"
      >
        <SessionTree />
        <ConsoleList />
        <RecentList />
      </nav>
      <ResizeHandle
        side="right"
        cssVar="--sidebar-width"
        settingKey="ui.sidebarWidth"
        min={LIMITS.sidebarWidth.min}
        max={LIMITS.sidebarWidth.max}
        label={t('shell.resizeSidebar')}
      />
    </div>
  );
}

function relativeTime(ms: number, now: number): string {
  const rtf = new Intl.RelativeTimeFormat(getLocale(), { numeric: 'auto', style: 'narrow' });
  const diff = Math.round((ms - now) / 60_000);
  if (Math.abs(diff) < 60) return rtf.format(diff, 'minute');
  const h = Math.round(diff / 60);
  if (Math.abs(h) < 24) return rtf.format(h, 'hour');
  return rtf.format(Math.round(h / 24), 'day');
}

function RecentList() {
  const t = useT();
  const sessions = useLibrary((s) => s.sessions);
  const recent = useMemo(() => recentSessions(sessions), [sessions]);
  const now = useNow(60_000);
  if (recent.length === 0) return null;

  return (
    <section aria-labelledby="recent-heading" className="flex flex-col">
      <h2 id="recent-heading" className="label-caps flex h-7 items-center px-3">
        {t('shell.recent')}
      </h2>
      <ul className="flex flex-col px-1">
        {recent.map((s) => (
          <li key={s.id}>
            <button
              type="button"
              className="tree-row pl-2"
              data-env={envKind(s.envTag) ?? undefined}
              onClick={() => useTabs.getState().openSession(s)}
              title={t('library.connectTo', { name: s.name })}
            >
              <span className="mono flex-1 truncate">{s.name}</span>
              {envKind(s.envTag) && (
                <span className="env-letter" title={envLabel(s.envTag) ?? ''}>
                  {envLetter(s.envTag)}
                </span>
              )}
              <span className="text-xs text-muted tabular">
                {relativeTime(s.lastConnectedAt ?? now, now)}
              </span>
            </button>
          </li>
        ))}
      </ul>
    </section>
  );
}

/** Current time, refreshed every `intervalMs` (keeps relative times like "5 min ago" fresh). */
function useNow(intervalMs: number): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const id = window.setInterval(() => setNow(Date.now()), intervalMs);
    return () => window.clearInterval(id);
  }, [intervalMs]);
  return now;
}
