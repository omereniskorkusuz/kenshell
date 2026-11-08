/**
 * Notes bar under a session's terminal: a one-line input that adds a time-stamped note
 * (like a work log) and, expanded, the session's notes newest first. Same notes as the
 * side panel's Notes tab (device notes once the device is identified, else the saved
 * session's), so edits in either show up in both.
 */
import { ChevronDown, ChevronUp, NotebookPen, Trash2, X } from 'lucide-react';
import { useCallback, useEffect, useState } from 'react';
import { useT } from '../i18n';
import { describeError } from '../lib/errors';
import { api, type Note } from '../lib/ipc';
import type { LiveTab } from '../lib/sessionState';
import { useNotes } from '../stores/notes';
import { toast } from '../stores/toasts';
import { useUi } from '../stores/ui';

/** Note time: clock only for today, else date + clock. */
export function noteTime(ms: number, now = Date.now()): string {
  const d = new Date(ms);
  const sameDay = new Date(now).toDateString() === d.toDateString();
  return sameDay
    ? d.toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit' })
    : d.toLocaleString(undefined, { dateStyle: 'short', timeStyle: 'short' });
}

export function NotesBar({ tab }: { tab: LiveTab }) {
  const t = useT();
  const sessionId = tab.sessionId ?? null;
  const deviceId = tab.device.deviceId ?? null;
  const owned = sessionId !== null || deviceId !== null;
  const rev = useNotes((s) => s.rev);
  const [notes, setNotes] = useState<Note[]>([]);
  const [draft, setDraft] = useState('');
  const [expanded, setExpanded] = useState(true);

  const load = useCallback(() => {
    void rev;
    if (!owned) return;
    api
      .notesList(sessionId, deviceId)
      .then(setNotes)
      .catch(() => setNotes([]));
  }, [owned, sessionId, deviceId, rev]);
  useEffect(load, [load]);

  const fail = (e: unknown) => {
    const d = describeError(e, t);
    toast({ kind: 'error', message: d.message, detail: d.detail });
  };
  const add = async () => {
    const body = draft.trim();
    if (!body || !owned) return;
    setDraft('');
    try {
      await api.noteCreate(sessionId, deviceId, body);
      useNotes.getState().changed(sessionId, deviceId);
    } catch (e) {
      setDraft(body);
      fail(e);
    }
  };
  const remove = async (n: Note) => {
    try {
      await api.noteDelete(n.id);
      useNotes.getState().changed(sessionId, deviceId);
    } catch (e) {
      fail(e);
    }
  };

  return (
    <div
      className="flex shrink-0 flex-col border-t border-subtle bg-surface text-sm"
      role="region"
      aria-label={t('notesBar.title')}
    >
      <div className="flex h-9 items-center gap-2 px-2">
        <NotebookPen size={14} className="shrink-0 text-muted" aria-hidden />
        <span className="shrink-0 text-xs text-muted">
          {t('notesBar.title')}
          {notes.length > 0 && ` · ${notes.length}`}
        </span>
        {owned ? (
          <input
            className="field h-7 min-w-0 flex-1 text-sm"
            aria-label={t('notes.add')}
            placeholder={t('notesBar.placeholder')}
            value={draft}
            onChange={(e) => setDraft(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter' && !e.nativeEvent.isComposing) {
                e.preventDefault();
                void add();
              }
              // Escape hands the keyboard back to the terminal.
              if (e.key === 'Escape') (e.target as HTMLInputElement).blur();
            }}
          />
        ) : (
          <span className="min-w-0 flex-1 truncate text-xs text-muted">
            {t('notes.needsSession')}
          </span>
        )}
        {owned && notes.length > 0 && (
          <button
            type="button"
            className="icon-btn icon-btn-sm"
            aria-expanded={expanded}
            aria-label={expanded ? t('notesBar.collapse') : t('notesBar.expand')}
            title={expanded ? t('notesBar.collapse') : t('notesBar.expand')}
            onClick={() => setExpanded(!expanded)}
          >
            {expanded ? <ChevronDown size={14} /> : <ChevronUp size={14} />}
          </button>
        )}
        <button
          type="button"
          className="icon-btn icon-btn-sm"
          aria-label={t('notesBar.close')}
          title={t('notesBar.close')}
          onClick={() => useUi.getState().toggleNotesBar(tab.key)}
        >
          <X size={14} />
        </button>
      </div>
      {owned && expanded && notes.length > 0 && (
        <ul className="scroll-thin max-h-36 overflow-y-auto border-t border-subtle px-2 py-1">
          {notes.map((n) => (
            <li key={n.id} className="group flex items-start gap-2 py-0.5">
              <time
                className="mono shrink-0 pt-px text-xs text-muted"
                dateTime={new Date(n.createdAt).toISOString()}
              >
                {noteTime(n.createdAt)}
              </time>
              <span className="min-w-0 flex-1 whitespace-pre-wrap break-words">{n.body}</span>
              <button
                type="button"
                className="icon-btn icon-btn-sm opacity-0 group-hover:opacity-100 focus-visible:opacity-100"
                aria-label={t('notes.delete')}
                title={t('notes.delete')}
                onClick={() => void remove(n)}
              >
                <Trash2 size={12} />
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
