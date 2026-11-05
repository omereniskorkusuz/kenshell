/**
 * Device notes (P3-T08), in the side panel. Notes of the active tab: its device's (once the
 * device is identified — the same notes over SSH or console) plus its saved session's.
 * Enter adds (Shift+Enter = new line); a note starting with an interface name
 * (`Gi1/0/24: camera`) is linked to that interface. Markdown is shown, never executed.
 */
import { Pencil, Pin, PinOff, Trash2, Download } from 'lucide-react';
import { useCallback, useEffect, useState } from 'react';
import { useT } from '../i18n';
import { describeError } from '../lib/errors';
import { api, type Note } from '../lib/ipc';
import { Markdown } from '../lib/markdown';
import { useNotes } from '../stores/notes';
import { useActiveTab } from '../stores/tabs';
import { toast } from '../stores/toasts';
import { useUi } from '../stores/ui';

const fail = (e: unknown, t: ReturnType<typeof useT>) => {
  const d = describeError(e, t);
  toast({ kind: 'error', message: d.message, detail: d.detail });
};

export function NotesPanel() {
  const t = useT();
  const tab = useActiveTab();
  const sessionId = tab?.sessionId ?? null;
  const deviceId = tab?.device.deviceId ?? null;
  if (!tab) {
    return <p className="p-4 text-center text-sm text-muted">{t('notes.noTab')}</p>;
  }
  if (sessionId === null && deviceId === null) {
    return <p className="p-4 text-center text-sm text-muted">{t('notes.needsSession')}</p>;
  }
  // A new owner (other tab, or the device just got identified) starts a fresh list.
  return <NotesList key={`${sessionId}:${deviceId}`} sessionId={sessionId} deviceId={deviceId} />;
}

function NotesList({ sessionId, deviceId }: { sessionId: number | null; deviceId: number | null }) {
  const t = useT();
  const [notes, setNotes] = useState<Note[]>([]);
  const [draft, setDraft] = useState('');
  const [editing, setEditing] = useState<{ id: number; text: string } | null>(null);

  const load = useCallback(() => {
    api
      .notesList(sessionId, deviceId)
      .then(setNotes)
      .catch(() => setNotes([]));
  }, [sessionId, deviceId]);
  useEffect(load, [load]);
  /** After a change: this list, the terminal tooltips and the session list subtitles. */
  const reload = () => {
    load();
    useNotes.getState().changed(sessionId, deviceId);
  };

  const add = async () => {
    const body = draft.trim();
    if (!body) return;
    // Clear at once: typing the next note must not land in this one while it saves.
    setDraft('');
    try {
      await api.noteCreate(sessionId, deviceId, body);
      reload();
    } catch (e) {
      setDraft((cur) => (cur ? `${body}\n${cur}` : body));
      fail(e, t);
    }
  };
  const save = async () => {
    if (!editing) return;
    try {
      await api.noteUpdate(editing.id, editing.text);
      setEditing(null);
      reload();
    } catch (e) {
      fail(e, t);
    }
  };
  const remove = async (n: Note) => {
    try {
      await api.noteDelete(n.id);
      reload();
      toast({
        message: t('notes.deleted'),
        actions: [
          {
            label: t('common.undo'),
            primary: true,
            run: () =>
              void api
                .noteCreate(n.sessionId, n.deviceId, n.body)
                .then((c) => (n.pinned ? api.notePin(c.id, true) : c))
                .then(reload)
                .catch((e: unknown) => fail(e, t)),
          },
        ],
      });
    } catch (e) {
      fail(e, t);
    }
  };
  const exportAll = async () => {
    const path = await api.pickPath('save', ['md'], 'kenshell-notes.md').catch(() => null);
    if (!path) return;
    try {
      await api.notesExport(path);
      useUi.getState().flash(t('notes.exported', { path }));
    } catch (e) {
      fail(e, t);
    }
  };

  const when = (ms: number) =>
    new Date(ms).toLocaleString(undefined, { dateStyle: 'short', timeStyle: 'short' });

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex flex-col gap-1 border-b border-subtle p-2">
        <textarea
          className="field min-h-14 text-sm"
          rows={2}
          aria-label={t('notes.add')}
          placeholder={t('notes.placeholder')}
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter' && !e.shiftKey && !e.nativeEvent.isComposing) {
              e.preventDefault();
              void add();
            }
          }}
        />
        <div className="flex items-center gap-2 text-xs text-muted">
          <span className="flex-1">
            {deviceId !== null ? t('notes.onDevice') : t('notes.onSession')}
          </span>
          <button
            type="button"
            className="icon-btn icon-btn-sm"
            aria-label={t('notes.export')}
            title={t('notes.export')}
            onClick={() => void exportAll()}
          >
            <Download size={12} />
          </button>
        </div>
      </div>
      <ul
        className="scroll-thin flex min-h-0 flex-1 flex-col gap-2 overflow-y-auto p-2"
        aria-label={t('panel.notes')}
      >
        {notes.length === 0 && (
          <li className="p-4 text-center text-sm text-muted">{t('panel.notesEmpty')}</li>
        )}
        {notes.map((n) => (
          <li
            key={n.id}
            className={`group flex flex-col gap-1 rounded-md border p-2 text-sm ${
              n.pinned ? 'border-strong bg-elevated' : 'border-subtle'
            }`}
          >
            <div className="flex items-center gap-1 text-xs text-muted">
              {n.interfaceShort && <span className="chip mono">{n.interfaceShort}</span>}
              <span className="flex-1" title={new Date(n.updatedAt).toLocaleString()}>
                {when(n.createdAt)}
              </span>
              <button
                type="button"
                className="icon-btn icon-btn-sm"
                aria-label={n.pinned ? t('notes.unpin') : t('notes.pin')}
                aria-pressed={n.pinned}
                title={n.pinned ? t('notes.unpin') : t('notes.pin')}
                onClick={() =>
                  void api
                    .notePin(n.id, !n.pinned)
                    .then(reload)
                    .catch((e: unknown) => fail(e, t))
                }
              >
                {n.pinned ? <PinOff size={12} /> : <Pin size={12} />}
              </button>
              <button
                type="button"
                className="icon-btn icon-btn-sm"
                aria-label={t('notes.edit')}
                title={t('notes.edit')}
                onClick={() => setEditing({ id: n.id, text: n.body })}
              >
                <Pencil size={12} />
              </button>
              <button
                type="button"
                className="icon-btn icon-btn-sm"
                aria-label={t('notes.delete')}
                title={t('notes.delete')}
                onClick={() => void remove(n)}
              >
                <Trash2 size={12} />
              </button>
            </div>
            {editing?.id === n.id ? (
              <textarea
                autoFocus
                className="field min-h-16 text-sm"
                aria-label={t('notes.edit')}
                value={editing.text}
                onChange={(e) => setEditing({ id: n.id, text: e.target.value })}
                onBlur={() => void save()}
                onKeyDown={(e) => {
                  if (e.key === 'Escape') setEditing(null);
                  if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) void save();
                }}
              />
            ) : (
              <Markdown text={n.body} />
            )}
          </li>
        ))}
      </ul>
    </div>
  );
}
