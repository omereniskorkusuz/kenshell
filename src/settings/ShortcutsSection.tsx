/**
 * Settings → Shortcuts (P5-T07): every action, its shortcut (default or the user's),
 * change by pressing the new keys, remove, reset. A chord another action already uses
 * is not taken over: the row says which action has it (no dialogs).
 */
import { RotateCcw, X } from 'lucide-react';
import { useEffect, useMemo, useState } from 'react';
import { ACTIONS, shortcutOf } from '../app/actions';
import { useT } from '../i18n';
import { isMac } from '../lib/ipc';
import { chordFromEvent, formatChord, normalizeSpec, RESERVED } from '../lib/shortcuts';
import { useSettings } from '../stores/settings';
import { useUi } from '../stores/ui';

export function ShortcutsSection() {
  const t = useT();
  const mac = isMac();
  const overrides = useSettings((s) => s.values['shortcuts.overrides']);
  const [editing, setEditing] = useState<string | null>(null);
  const [note, setNote] = useState<{ id: string; text: string } | null>(null);
  const [filter, setFilter] = useState('');

  const rows = useMemo(
    () =>
      ACTIONS.filter((a) => !a.hidden)
        .map((a) => ({ a, title: t(a.title), spec: shortcutOf(a) }))
        .filter(
          (r) =>
            !filter.trim() ||
            r.title.toLowerCase().includes(filter.trim().toLowerCase()) ||
            (r.spec && formatChord(r.spec, mac).toLowerCase().includes(filter.toLowerCase())),
        ),
    // `overrides` changes what shortcutOf returns.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [t, filter, mac, overrides],
  );

  const without = (o: Record<string, string | null>, id: string) =>
    Object.fromEntries(Object.entries(o).filter(([k]) => k !== id));
  const save = (next: Record<string, string | null>) =>
    useSettings.getState().set('shortcuts.overrides', next);

  // Recording a new chord: global shortcuts pause, the next chord is taken.
  useEffect(() => {
    if (!editing) return;
    useUi.getState().setCapturingShortcut(true);
    const onKey = (e: KeyboardEvent) => {
      e.preventDefault();
      e.stopPropagation();
      if (e.key === 'Escape') {
        setEditing(null);
        return;
      }
      const spec = chordFromEvent(e, mac);
      if (!spec) return; // a lone modifier: keep waiting
      const norm = normalizeSpec(spec);
      if ((RESERVED as readonly string[]).includes(norm)) {
        setNote({ id: editing, text: t('shortcuts.reserved', { keys: formatChord(norm, mac) }) });
        setEditing(null);
        return;
      }
      const owner = ACTIONS.find((a) => a.id !== editing && shortcutOf(a) === norm);
      if (owner) {
        setNote({ id: editing, text: t('shortcuts.taken', { action: t(owner.title) }) });
        setEditing(null);
        return;
      }
      const action = ACTIONS.find((a) => a.id === editing);
      const cur = useSettings.getState().values['shortcuts.overrides'];
      // Choosing the default again simply drops the override.
      save(
        action?.shortcut && normalizeSpec(action.shortcut) === norm
          ? without(cur, editing)
          : { ...cur, [editing]: norm },
      );
      setNote(null);
      setEditing(null);
    };
    window.addEventListener('keydown', onKey, { capture: true });
    return () => {
      window.removeEventListener('keydown', onKey, { capture: true });
      useUi.getState().setCapturingShortcut(false);
    };
  }, [editing, mac, t]);

  const remove = (id: string) => {
    save({ ...overrides, [id]: null });
    setNote(null);
  };
  const reset = (id: string) => {
    save(without(overrides, id));
    setNote(null);
  };

  return (
    <div className="flex flex-col gap-3">
      <div className="flex items-center gap-2">
        <input
          className="field h-8 min-w-0 flex-1 text-sm"
          value={filter}
          placeholder={t('shortcuts.filter')}
          aria-label={t('shortcuts.filter')}
          onChange={(e) => setFilter(e.target.value)}
        />
        <button
          type="button"
          className="btn btn-ghost h-8"
          disabled={Object.keys(overrides).length === 0}
          onClick={() => save({})}
        >
          {t('shortcuts.resetAll')}
        </button>
      </div>
      <p className="text-xs text-muted">{t('shortcuts.hint')}</p>
      <ul className="flex flex-col">
        {rows.map(({ a, title, spec }) => {
          const custom = a.id in overrides;
          return (
            <li key={a.id} className="flex flex-col border-b border-subtle py-1.5">
              <div className="flex items-center gap-2">
                <span className="min-w-0 flex-1 truncate text-sm">{title}</span>
                <button
                  type="button"
                  className={`btn h-7 min-w-24 justify-center ${editing === a.id ? 'btn-primary' : 'btn-ghost'}`}
                  aria-label={t('shortcuts.change', { action: title })}
                  onClick={() => setEditing(editing === a.id ? null : a.id)}
                >
                  {editing === a.id ? (
                    t('shortcuts.press')
                  ) : spec ? (
                    <span className="kbd">{formatChord(spec, mac)}</span>
                  ) : (
                    <span className="text-muted">{t('shortcuts.none')}</span>
                  )}
                </button>
                <button
                  type="button"
                  className="icon-btn icon-btn-sm"
                  disabled={!spec}
                  aria-label={t('shortcuts.remove', { action: title })}
                  title={t('shortcuts.removeShort')}
                  onClick={() => remove(a.id)}
                >
                  <X size={13} />
                </button>
                <button
                  type="button"
                  className="icon-btn icon-btn-sm"
                  disabled={!custom}
                  aria-label={t('shortcuts.reset', { action: title })}
                  title={t('shortcuts.resetShort')}
                  onClick={() => reset(a.id)}
                >
                  <RotateCcw size={13} />
                </button>
              </div>
              {note?.id === a.id && (
                <p className="tone-text text-xs" data-tone="warning" role="status">
                  {note.text}
                </p>
              )}
            </li>
          );
        })}
      </ul>
    </div>
  );
}
