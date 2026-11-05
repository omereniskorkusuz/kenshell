/** Left sidebar: saved sessions, detected consoles, recent connections. Resizable 180–320px. */
import { useEffect, useMemo, useRef, useState } from 'react';
import { Search, Star, X } from 'lucide-react';
import { useT, getLocale } from '../i18n';
import {
  envKind,
  envLabel,
  envLetter,
  favoriteSessions,
  folderPath,
  recentSessions,
  searchSessions,
} from '../lib/tree';
import { api, type SessionRecord } from '../lib/ipc';
import { useUi } from '../stores/ui';
import { useLibrary } from '../stores/library';
import { LIMITS, useSettings } from '../stores/settings';
import { useTabs } from '../stores/tabs';
import { ConsoleList } from './ConsoleList';
import { ResizeHandle } from './ResizeHandle';
import { SessionTree } from './SessionTree';

/** Search box + (results | favorites + tree), then consoles and recent. */
function SessionSearch() {
  const t = useT();
  const [query, setQuery] = useState('');
  const input = useRef<HTMLInputElement>(null);
  const sessions = useLibrary((s) => s.sessions);
  const folders = useLibrary((s) => s.folders);
  const focusTick = useUi((s) => s.searchFocus);
  // Note content is searched in the backend (FTS5), shortly after typing stops.
  const [byNotes, setByNotes] = useState<ReadonlySet<number>>(() => new Set());
  useEffect(() => {
    const q = query.trim();
    if (q.length < 2) return;
    let live = true;
    const timer = setTimeout(() => {
      api
        .notesSearch(q)
        .then((hits) => {
          if (!live) return;
          const ids = new Set<number>();
          const devices = new Set(hits.map((h) => h.deviceId).filter((d) => d !== null));
          for (const h of hits) if (h.sessionId !== null) ids.add(h.sessionId);
          for (const s of sessions)
            if (s.deviceId !== null && devices.has(s.deviceId)) ids.add(s.id);
          setByNotes(ids);
        })
        .catch(() => {});
    }, 200);
    return () => {
      live = false;
      clearTimeout(timer);
    };
  }, [query, sessions]);
  const results = useMemo(
    () => searchSessions(sessions, folders, query, query.trim().length >= 2 ? byNotes : undefined),
    [sessions, folders, query, byNotes],
  );
  const favorites = useMemo(() => favoriteSessions(sessions), [sessions]);
  const searching = query.trim() !== '';

  useEffect(() => {
    if (focusTick === 0) return;
    input.current?.focus();
    input.current?.select();
  }, [focusTick]);

  const open = (s: SessionRecord) => {
    useTabs.getState().openSession(s);
    setQuery('');
  };

  return (
    <>
      <div className="relative mx-2">
        <Search
          size={13}
          className="pointer-events-none absolute left-2 top-1.5 text-muted"
          aria-hidden
        />
        <input
          ref={input}
          type="text"
          role="searchbox"
          className="field h-7 w-full pl-7 pr-6 text-sm"
          placeholder={t('library.search')}
          aria-label={t('library.search')}
          value={query}
          spellCheck={false}
          onChange={(e) => setQuery(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Escape') setQuery('');
            if (e.key === 'Enter' && results[0]) open(results[0]);
          }}
        />
        {searching && (
          <button
            type="button"
            className="icon-btn absolute right-1 top-1"
            aria-label={t('search.close')}
            onClick={() => setQuery('')}
          >
            <X size={12} />
          </button>
        )}
      </div>
      {searching ? (
        <SessionList
          heading={t('library.results', { count: results.length })}
          items={results}
          empty={t('library.noResults')}
          showPath
          onOpen={open}
        />
      ) : (
        <>
          {favorites.length > 0 && (
            <SessionList heading={t('library.favorites')} items={favorites} onOpen={open} star />
          )}
          <SessionTree />
          <ConsoleList />
          <RecentList />
        </>
      )}
    </>
  );
}

function SessionList(p: {
  heading: string;
  items: SessionRecord[];
  empty?: string;
  showPath?: boolean;
  star?: boolean;
  onOpen: (s: SessionRecord) => void;
}) {
  const t = useT();
  const folders = useLibrary((s) => s.folders);
  return (
    <section className="flex flex-col">
      <h2 className="label-caps flex h-7 items-center px-3">{p.heading}</h2>
      {p.items.length === 0 && p.empty ? (
        <p className="px-3 text-sm text-muted">{p.empty}</p>
      ) : (
        <ul className="flex flex-col px-1">
          {p.items.map((s) => (
            <li key={s.id}>
              <button
                type="button"
                className="tree-row pl-2"
                data-env={envKind(s.envTag) ?? undefined}
                onClick={() => p.onOpen(s)}
                title={t('library.connectTo', { name: s.name })}
              >
                {p.star && <Star size={11} className="shrink-0 text-muted" aria-hidden />}
                <span className="mono min-w-0 flex-1 truncate">{s.name}</span>
                {p.showPath && s.folderId !== null && (
                  <span className="min-w-0 max-w-[45%] truncate text-xs text-muted">
                    {folderPath(folders, s.folderId)}
                  </span>
                )}
                {envKind(s.envTag) && (
                  <span className="env-letter" title={envLabel(s.envTag) ?? ''}>
                    {envLetter(s.envTag)}
                  </span>
                )}
              </button>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}

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
        <SessionSearch />
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
