/**
 * Bulk edit bar (P2-T10): shown under the session tree while two or more sessions are
 * marked (⌘/Ctrl-click, Shift-click, ⌘/Ctrl+A). Every action is one all-or-nothing backend
 * call; delete has undo in its toast (no confirm dialogs).
 */
import { Star, StarOff, Tags, Trash2, User, X } from 'lucide-react';
import { useState } from 'react';
import { useT } from '../i18n';
import { parseTagList, type SessionPatch } from '../lib/bulk';
import { folderPath } from '../lib/tree';
import { useLibrary } from '../stores/library';
import { Menu, useMenu, type MenuEntry } from './Menu';

const ENVS = ['prod', 'lab', 'test'] as const;

interface Props {
  ids: number[];
  onClear: () => void;
}

type Form = 'tags' | 'user' | null;

export function BulkBar({ ids, onClear }: Props) {
  const t = useT();
  const folders = useLibrary((s) => s.folders);
  const lib = useLibrary.getState;
  const folderMenu = useMenu();
  const envMenu = useMenu();
  const [form, setForm] = useState<Form>(null);
  const [addText, setAddText] = useState('');
  const [removeText, setRemoveText] = useState('');
  const [user, setUser] = useState('');

  const apply = async (patch: SessionPatch) => {
    if (await lib().bulkUpdate(ids, patch)) setForm(null);
  };

  const folderEntries: MenuEntry[] = [
    { label: t('library.bulkRoot'), onSelect: () => void apply({ folder: { value: null } }) },
    ...folders
      .map((f) => ({
        label: folderPath(folders, f.id),
        onSelect: () => void apply({ folder: { value: f.id } }),
      }))
      .sort((a, b) => a.label.localeCompare(b.label)),
  ];
  const envEntries: MenuEntry[] = [
    ...ENVS.map((env) => ({
      label: env.toUpperCase(),
      onSelect: () => void apply({ envTag: { value: env } }),
    })),
    { kind: 'separator' },
    { label: t('library.bulkEnvNone'), onSelect: () => void apply({ envTag: { value: null } }) },
  ];

  return (
    <div
      role="toolbar"
      aria-label={t('library.bulkSelected', { count: ids.length })}
      className="mx-1 mt-1 flex flex-col gap-1.5 rounded-md border border-subtle bg-elevated p-1.5"
    >
      <div className="flex items-center gap-1">
        <span className="flex-1 text-xs text-secondary" role="status">
          {t('library.bulkSelected', { count: ids.length })}
        </span>
        <button
          type="button"
          className="icon-btn icon-btn-sm"
          aria-label={t('library.bulkClear')}
          title={`${t('library.bulkClear')} (Esc)`}
          onClick={onClear}
        >
          <X size={12} />
        </button>
      </div>
      <div className="flex flex-wrap gap-1">
        <button
          type="button"
          className="btn btn-sm"
          aria-haspopup="menu"
          onClick={(e) => folderMenu.openBelow(e.currentTarget)}
        >
          {t('library.bulkFolder')}
        </button>
        <button
          type="button"
          className="btn btn-sm"
          aria-haspopup="menu"
          onClick={(e) => envMenu.openBelow(e.currentTarget)}
        >
          {t('library.bulkEnv')}
        </button>
        <button
          type="button"
          className="icon-btn"
          aria-label={t('library.bulkTags')}
          title={t('library.bulkTags')}
          aria-pressed={form === 'tags'}
          onClick={() => setForm(form === 'tags' ? null : 'tags')}
        >
          <Tags size={14} />
        </button>
        <button
          type="button"
          className="icon-btn"
          aria-label={t('library.bulkUser')}
          title={t('library.bulkUser')}
          aria-pressed={form === 'user'}
          onClick={() => setForm(form === 'user' ? null : 'user')}
        >
          <User size={14} />
        </button>
        <button
          type="button"
          className="icon-btn"
          aria-label={t('library.bulkFavOn')}
          title={t('library.bulkFavOn')}
          onClick={() => void apply({ favorite: true })}
        >
          <Star size={14} />
        </button>
        <button
          type="button"
          className="icon-btn"
          aria-label={t('library.bulkFavOff')}
          title={t('library.bulkFavOff')}
          onClick={() => void apply({ favorite: false })}
        >
          <StarOff size={14} />
        </button>
        <button
          type="button"
          className="icon-btn text-danger"
          aria-label={t('library.bulkDelete', { count: ids.length })}
          title={t('library.bulkDelete', { count: ids.length })}
          onClick={() => {
            void lib().bulkDelete(ids);
            onClear();
          }}
        >
          <Trash2 size={14} />
        </button>
      </div>

      {form === 'tags' && (
        <form
          className="flex flex-col gap-1"
          onSubmit={(e) => {
            e.preventDefault();
            void apply({ addTags: parseTagList(addText), removeTags: parseTagList(removeText) });
          }}
        >
          <input
            autoFocus
            className="field h-6 text-xs"
            aria-label={t('library.bulkAddTags')}
            placeholder={t('library.bulkAddTags')}
            value={addText}
            spellCheck={false}
            onChange={(e) => setAddText(e.target.value)}
          />
          <input
            className="field h-6 text-xs"
            aria-label={t('library.bulkRemoveTags')}
            placeholder={t('library.bulkRemoveTags')}
            value={removeText}
            spellCheck={false}
            onChange={(e) => setRemoveText(e.target.value)}
          />
          <p className="text-[11px] text-muted">{t('library.bulkTagsHint')}</p>
          <button
            type="submit"
            className="btn btn-sm w-fit"
            disabled={!addText.trim() && !removeText.trim()}
          >
            {t('library.bulkApply')}
          </button>
        </form>
      )}

      {form === 'user' && (
        <form
          className="flex flex-col gap-1"
          onSubmit={(e) => {
            e.preventDefault();
            void apply({ username: { value: user.trim() || null } });
          }}
        >
          <input
            autoFocus
            className="field mono h-6 text-xs"
            aria-label={t('library.bulkUser')}
            placeholder={t('library.bulkUser')}
            value={user}
            spellCheck={false}
            autoComplete="off"
            onChange={(e) => setUser(e.target.value)}
          />
          <p className="text-[11px] text-muted">{t('library.bulkUserHint')}</p>
          <button type="submit" className="btn btn-sm w-fit">
            {t('library.bulkApply')}
          </button>
        </form>
      )}

      {folderMenu.at && (
        <Menu
          x={folderMenu.at.x}
          y={folderMenu.at.y}
          entries={folderEntries}
          onClose={folderMenu.close}
          label={t('library.bulkFolder')}
        />
      )}
      {envMenu.at && (
        <Menu
          x={envMenu.at.x}
          y={envMenu.at.y}
          entries={envEntries}
          onClose={envMenu.close}
          label={t('library.bulkEnv')}
        />
      )}
    </div>
  );
}
