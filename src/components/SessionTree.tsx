/**
 * Saved sessions tree: nested folders, PROD first, inline rename, drag & drop, context menu,
 * full keyboard navigation (roving tabindex, WAI-ARIA tree pattern).
 *
 * P2-T10: several sessions can be marked (⌘/Ctrl-click, Shift-click, ⌘/Ctrl+A) for the bulk
 * bar; dragging a marked session moves all marked ones. A folder dropped on the top edge of
 * another folder is placed before it, anywhere else on the row it goes inside.
 */
import {
  useMemo,
  useRef,
  useState,
  type DragEvent,
  type KeyboardEvent,
  type MouseEvent,
  type ReactNode,
} from 'react';
import { ChevronDown, ChevronRight, Folder as FolderIcon, Plus } from 'lucide-react';
import { rangeBetween } from '../lib/bulk';
import { useT } from '../i18n';
import { isMac, type Folder, type SessionRecord } from '../lib/ipc';
import { formatChord } from '../lib/shortcuts';
import {
  buildTree,
  descendantFolderIds,
  envKind,
  envLabel,
  envLetter,
  folderPath,
  type TreeNode,
} from '../lib/tree';
import { newSerialSession } from '../app/actions';
import { useLibrary } from '../stores/library';
import { useTabs } from '../stores/tabs';
import { useUi } from '../stores/ui';
import { BulkBar } from './BulkBar';
import { Menu, useMenu, type MenuEntry } from './Menu';

type Sel = `f${number}` | `s${number}`;
const DND_TYPE = 'application/x-kenshell-node';

interface DragPayload {
  kind: 'folder' | 'session';
  id: number;
}

function readDrag(e: DragEvent): DragPayload | null {
  try {
    const raw = e.dataTransfer.getData(DND_TYPE);
    return raw ? (JSON.parse(raw) as DragPayload) : null;
  } catch {
    return null;
  }
}

export function SessionTree() {
  const t = useT();
  const folders = useLibrary((s) => s.folders);
  const sessions = useLibrary((s) => s.sessions);
  const loaded = useLibrary((s) => s.loaded);
  const collapsed = useLibrary((s) => s.collapsed);
  const renaming = useLibrary((s) => s.renaming);
  const tabs = useTabs((s) => s.tabs);
  const tree = useMemo(() => buildTree(folders, sessions), [folders, sessions]);
  const [selected, setSelected] = useState<Sel | null>(null);
  const [dropTarget, setDropTarget] = useState<number | 'root' | null>(null);
  /** Session row a dragged session would be inserted before. */
  const [dropBefore, setDropBefore] = useState<number | null>(null);
  const [dragging, setDragging] = useState<DragPayload | null>(null);
  /** Folder row a dragged folder would be placed before. */
  const [dropBeforeFolder, setDropBeforeFolder] = useState<number | null>(null);
  /** Sessions marked for bulk edit, and the anchor of Shift-click ranges. */
  const [marked, setMarked] = useState<Set<number>>(() => new Set());
  const [anchor, setAnchor] = useState<number | null>(null);
  const menu = useMenu();
  const [menuEntries, setMenuEntries] = useState<MenuEntry[]>([]);
  const addMenu = useMenu();
  const treeRef = useRef<HTMLDivElement>(null);
  const mac = isMac();

  const lib = useLibrary.getState;
  const openSession = (s: SessionRecord) => useTabs.getState().openSession(s);
  const editSession = (s: SessionRecord) => useUi.getState().openEditor({ sessionId: s.id });

  /** Session ids in the order they are shown (inside expanded folders only). */
  const visibleSessionIds = (): number[] => {
    const out: number[] = [];
    const walk = (nodes: TreeNode[]) => {
      for (const n of nodes) {
        if (n.kind === 'session') out.push(n.session.id);
        else if (!collapsed.has(n.folder.id)) walk(n.children);
      }
    };
    walk(tree);
    return out;
  };
  // Marks of sessions that no longer exist are dropped.
  const markedIds = Array.from(marked).filter((id) => sessions.some((x) => x.id === id));
  const clearMarks = () => {
    setMarked(new Set());
    setAnchor(null);
  };
  /** Click on a session row: plain = single, ⌘/Ctrl = toggle, Shift = range from anchor. */
  const markOnClick = (e: MouseEvent, id: number) => {
    if (e.shiftKey && anchor !== null) {
      setMarked(new Set(rangeBetween(visibleSessionIds(), anchor, id)));
      return;
    }
    if (e.metaKey || e.ctrlKey) {
      const next = new Set(marked);
      if (marked.size === 0 && anchor !== null && anchor !== id) next.add(anchor);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      setMarked(next);
      setAnchor(id);
      return;
    }
    setMarked(new Set());
    setAnchor(id);
  };

  const moveTargets = (
    exclude: Set<number>,
    current: number | null,
    onPick: (id: number | null) => void,
  ): MenuEntry[] => [
    { kind: 'label', label: t('library.moveTo') },
    { label: t('library.rootFolder'), disabled: current === null, onSelect: () => onPick(null) },
    ...folders
      .filter((f) => !exclude.has(f.id))
      .map((f) => ({
        label: folderPath(folders, f.id),
        disabled: f.id === current,
        onSelect: () => onPick(f.id),
      }))
      .sort((a, b) => a.label.localeCompare(b.label)),
  ];

  const sessionMenu = (s: SessionRecord): MenuEntry[] => [
    { label: t('library.connect'), hint: 'Enter', onSelect: () => openSession(s) },
    { label: t('library.edit'), onSelect: () => editSession(s) },
    {
      label: t('library.rename'),
      hint: 'F2',
      onSelect: () => lib().setRenaming({ kind: 'session', id: s.id }),
    },
    { label: t('library.duplicate'), onSelect: () => void lib().duplicateSession(s.id) },
    {
      label: s.favorite ? t('library.unfavorite') : t('library.favorite'),
      onSelect: () => void lib().toggleFavorite(s.id),
    },
    { kind: 'separator' },
    ...moveTargets(new Set(), s.folderId, (fid) => void lib().moveSession(s.id, fid)),
    { kind: 'separator' },
    {
      label: t('library.delete'),
      hint: mac ? '⌘⌫' : 'Del',
      danger: true,
      onSelect: () => void lib().deleteSession(s.id),
    },
  ];

  const folderMenu = (f: Folder): MenuEntry[] => [
    {
      label: t('library.newSessionHere'),
      onSelect: () =>
        useUi.getState().openEditor({
          sessionId: null,
          initial: { folderId: f.id, protocol: 'serial' },
        }),
    },
    { label: t('library.newSubfolder'), onSelect: () => void lib().createFolder(f.id) },
    {
      label: t('library.rename'),
      hint: 'F2',
      onSelect: () => lib().setRenaming({ kind: 'folder', id: f.id }),
    },
    { kind: 'separator' },
    ...moveTargets(
      descendantFolderIds(folders, f.id),
      f.parentId,
      (pid) => void lib().moveFolder(f.id, pid),
    ),
    { kind: 'separator' },
    {
      label: t('library.delete'),
      hint: mac ? '⌘⌫' : 'Del',
      danger: true,
      onSelect: () => void lib().deleteFolder(f.id),
    },
  ];

  const openMenuFor = (node: TreeNode, x: number, y: number) => {
    setSelected(node.kind === 'folder' ? `f${node.folder.id}` : `s${node.session.id}`);
    setMenuEntries(node.kind === 'folder' ? folderMenu(node.folder) : sessionMenu(node.session));
    menu.openAt(x, y);
  };

  // ---------------------------------------------------------------- keyboard

  const rows = () =>
    Array.from(treeRef.current?.querySelectorAll<HTMLElement>('[data-tree-row]') ?? []);
  const focusRow = (el: HTMLElement | undefined) => {
    if (!el) return;
    setSelected(el.dataset.treeRow as Sel);
    el.focus();
  };
  /** After inline rename the input disappears: put focus back on its row. */
  const refocus = (sel: Sel) =>
    setTimeout(() => {
      const el = treeRef.current?.querySelector<HTMLElement>(`[data-tree-row="${sel}"]`);
      if (el && (document.activeElement === document.body || !document.activeElement)) el.focus();
    }, 0);

  const onRowKeyDown = (e: KeyboardEvent<HTMLElement>, node: TreeNode) => {
    if (renaming) return;
    const list = rows();
    const i = list.indexOf(e.currentTarget);
    const isFolder = node.kind === 'folder';
    const isOpen = isFolder && !collapsed.has(node.folder.id);
    const del = e.key === 'Delete' || (e.key === 'Backspace' && (e.metaKey || !mac));
    if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 'a') {
      setMarked(new Set(visibleSessionIds()));
    } else if (e.key === 'Escape' && marked.size > 0) clearMarks();
    else if (e.key === 'ArrowDown') focusRow(list[i + 1]);
    else if (e.key === 'ArrowUp') focusRow(list[i - 1]);
    else if (e.key === 'Home') focusRow(list[0]);
    else if (e.key === 'End') focusRow(list[list.length - 1]);
    else if (e.key === 'ArrowRight' && isFolder) {
      if (!isOpen) lib().toggleCollapsed(node.folder.id, false);
      else focusRow(list[i + 1]);
    } else if (e.key === 'ArrowLeft') {
      if (isFolder && isOpen) lib().toggleCollapsed(node.folder.id, true);
      else {
        const parentId = isFolder ? node.folder.parentId : node.session.folderId;
        focusRow(list.find((el) => el.dataset.treeRow === `f${parentId}`));
      }
    } else if (e.key === 'Enter') {
      if (isFolder) lib().toggleCollapsed(node.folder.id);
      else openSession(node.session);
    } else if (e.key === 'F2') {
      lib().setRenaming(
        isFolder
          ? { kind: 'folder', id: node.folder.id }
          : { kind: 'session', id: node.session.id },
      );
    } else if (del && !isFolder && markedIds.length > 1 && marked.has(node.session.id)) {
      void lib().bulkDelete(markedIds);
      clearMarks();
    } else if (del) {
      focusRow(list[i + 1] ?? list[i - 1]);
      if (isFolder) void lib().deleteFolder(node.folder.id);
      else void lib().deleteSession(node.session.id);
    } else if (e.key === 'ContextMenu' || (e.key === 'F10' && e.shiftKey)) {
      const r = e.currentTarget.getBoundingClientRect();
      openMenuFor(node, r.left + 24, r.bottom);
    } else return;
    e.preventDefault();
    e.stopPropagation();
  };

  // ---------------------------------------------------------------- drag & drop

  const canDropInto = (target: number | null): boolean => {
    const d = dragging;
    if (!d) return false;
    if (d.kind === 'folder')
      return target === null || !descendantFolderIds(folders, d.id).has(target);
    return true;
  };

  const onDrop = (e: DragEvent, target: number | null) => {
    e.preventDefault();
    e.stopPropagation();
    setDropTarget(null);
    const d = readDrag(e) ?? dragging;
    setDragging(null);
    if (!d) return;
    if (d.kind === 'folder') {
      // Never into itself or one of its own sub-folders.
      if (target !== null && descendantFolderIds(folders, d.id).has(target)) return;
      void lib().moveFolder(d.id, target);
    } else if (markedIds.length > 1 && marked.has(d.id)) {
      void lib().bulkUpdate(markedIds, { folder: { value: target } });
    } else {
      void lib().moveSession(d.id, target);
    }
    if (target !== null) lib().toggleCollapsed(target, false);
  };

  const dragProps = (payload: DragPayload) => ({
    draggable: !renaming,
    onDragStart: (e: DragEvent) => {
      setDragging(payload);
      e.dataTransfer.setData(DND_TYPE, JSON.stringify(payload));
      e.dataTransfer.effectAllowed = 'move';
    },
    onDragEnd: () => {
      setDragging(null);
      setDropTarget(null);
      setDropBefore(null);
      setDropBeforeFolder(null);
    },
  });

  const prodPinned = (id: number): boolean => {
    const find = (nodes: TreeNode[]): boolean | null => {
      for (const n of nodes) {
        if (n.kind !== 'folder') continue;
        if (n.folder.id === id) return n.hasProd;
        const inner = find(n.children);
        if (inner !== null) return inner;
      }
      return null;
    };
    return find(tree) ?? false;
  };

  /**
   * Folder rows: a dragged folder over the top third → "before this folder" (same parent),
   * elsewhere → "into this folder"; sessions always go into the folder.
   */
  const folderDropProps = (f: Folder) => {
    const into = dropProps(f.id);
    // Folders holding PROD stay pinned on top, so "before" only works within one group.
    const beforeAllowed = () =>
      dragging?.kind === 'folder' &&
      dragging.id !== f.id &&
      canDropInto(f.parentId) &&
      prodPinned(dragging.id) === prodPinned(f.id);
    const isTop = (e: DragEvent) => {
      const r = e.currentTarget.getBoundingClientRect();
      return e.clientY - r.top < r.height / 3;
    };
    return {
      onDragOver: (e: DragEvent) => {
        if (beforeAllowed() && isTop(e)) {
          e.preventDefault();
          e.stopPropagation();
          e.dataTransfer.dropEffect = 'move';
          setDropBeforeFolder(f.id);
          setDropTarget(null);
          return;
        }
        setDropBeforeFolder(null);
        into.onDragOver(e);
      },
      onDragLeave: (e: DragEvent) => {
        if (!e.currentTarget.contains(e.relatedTarget as Node | null)) setDropBeforeFolder(null);
        into.onDragLeave(e);
      },
      onDrop: (e: DragEvent) => {
        if (dropBeforeFolder === f.id && beforeAllowed()) {
          e.preventDefault();
          e.stopPropagation();
          setDropBeforeFolder(null);
          const d = readDrag(e) ?? dragging;
          setDragging(null);
          if (d?.kind === 'folder') void lib().reorderFolder(d.id, f.parentId, f.id);
          return;
        }
        into.onDrop(e);
      },
    };
  };

  const dropProps = (target: number | null) => ({
    onDragOver: (e: DragEvent) => {
      if (!dragging || !canDropInto(target)) return;
      e.preventDefault();
      e.stopPropagation();
      e.dataTransfer.dropEffect = 'move';
      setDropTarget(target ?? 'root');
    },
    onDragLeave: (e: DragEvent) => {
      if (!e.currentTarget.contains(e.relatedTarget as Node | null)) setDropTarget(null);
    },
    onDrop: (e: DragEvent) => onDrop(e, target),
  });

  /** Dropping a session onto another session row: insert before it (same or other folder). */
  const sessionDropProps = (target: SessionRecord) => ({
    onDragOver: (e: DragEvent) => {
      if (dragging?.kind !== 'session' || dragging.id === target.id) return;
      e.preventDefault();
      e.stopPropagation();
      e.dataTransfer.dropEffect = 'move';
      setDropBefore(target.id);
      setDropTarget(null);
    },
    onDragLeave: (e: DragEvent) => {
      if (!e.currentTarget.contains(e.relatedTarget as Node | null)) setDropBefore(null);
    },
    onDrop: (e: DragEvent) => {
      e.preventDefault();
      e.stopPropagation();
      setDropBefore(null);
      const d = readDrag(e) ?? dragging;
      setDragging(null);
      if (d?.kind !== 'session' || d.id === target.id) return;
      if (markedIds.length > 1 && marked.has(d.id)) {
        // Keep the marked sessions' shown order, all placed before the target.
        const order = visibleSessionIds().filter((id) => marked.has(id) && id !== target.id);
        void (async () => {
          for (const id of order) await lib().reorderSession(id, target.folderId, target.id);
        })();
        return;
      }
      void lib().reorderSession(d.id, target.folderId, target.id);
    },
  });

  // ---------------------------------------------------------------- render

  const tabFor = (s: SessionRecord) => tabs.find((x) => x.sessionId === s.id);
  const firstSel: Sel | null = tree[0]
    ? tree[0].kind === 'folder'
      ? `f${tree[0].folder.id}`
      : `s${tree[0].session.id}`
    : null;
  const focusable = selected ?? firstSel;

  const renderNode = (node: TreeNode): ReactNode => {
    const pad = { paddingLeft: 4 + node.depth * 14 };
    if (node.kind === 'folder') {
      const f = node.folder;
      const sel: Sel = `f${f.id}`;
      const open = !collapsed.has(f.id);
      const isRenaming = renaming?.kind === 'folder' && renaming.id === f.id;
      return (
        <div key={sel} role="none">
          <div
            role="treeitem"
            aria-level={node.depth + 1}
            aria-expanded={open}
            aria-selected={selected === sel}
            tabIndex={focusable === sel ? 0 : -1}
            data-tree-row={sel}
            data-drop={dropTarget === f.id || undefined}
            data-drop-before={dropBeforeFolder === f.id || undefined}
            className="tree-row cursor-default"
            style={pad}
            onClick={() => {
              setSelected(sel);
              lib().toggleCollapsed(f.id);
            }}
            onFocus={() => setSelected(sel)}
            onKeyDown={(e) => onRowKeyDown(e, node)}
            onContextMenu={(e: MouseEvent) => {
              e.preventDefault();
              openMenuFor(node, e.clientX, e.clientY);
            }}
            {...dragProps({ kind: 'folder', id: f.id })}
            {...folderDropProps(f)}
          >
            <span className="flex w-3 shrink-0 justify-center text-muted" aria-hidden>
              {open ? <ChevronDown size={12} /> : <ChevronRight size={12} />}
            </span>
            <FolderIcon size={13} className="shrink-0 text-muted" aria-hidden />
            {isRenaming ? (
              <RenameInput
                initial={f.name}
                label={t('library.rename')}
                onDone={(name) => {
                  lib().setRenaming(null);
                  refocus(sel);
                  if (name !== null) void lib().renameFolder(f.id, name);
                }}
              />
            ) : (
              <span className="flex-1 truncate text-secondary">{f.name}</span>
            )}
          </div>
          {open && node.children.length > 0 && (
            <div role="group">{node.children.map(renderNode)}</div>
          )}
        </div>
      );
    }

    const s = node.session;
    const sel: Sel = `s${s.id}`;
    const tab = tabFor(s);
    const kind = envKind(s.envTag);
    const isRenaming = renaming?.kind === 'session' && renaming.id === s.id;
    const port = s.serial?.port ?? s.host ?? '';
    return (
      <div
        key={sel}
        role="treeitem"
        aria-level={node.depth + 1}
        aria-selected={marked.size > 0 ? marked.has(s.id) : selected === sel}
        data-marked={marked.has(s.id) || undefined}
        aria-label={[s.name, envLabel(s.envTag), tab ? t(`state.${tab.state}`) : null]
          .filter(Boolean)
          .join(', ')}
        tabIndex={focusable === sel ? 0 : -1}
        data-tree-row={sel}
        data-mode={tab ? tab.mode : undefined}
        data-vendor={tab?.device.vendor?.family}
        data-env={kind ?? undefined}
        className="tree-row cursor-default"
        style={pad}
        title={port}
        onClick={(e) => {
          setSelected(sel);
          markOnClick(e, s.id);
        }}
        onDoubleClick={() => openSession(s)}
        onFocus={() => setSelected(sel)}
        onKeyDown={(e) => onRowKeyDown(e, node)}
        onContextMenu={(e: MouseEvent) => {
          e.preventDefault();
          openMenuFor(node, e.clientX, e.clientY);
        }}
        data-drop-before={dropBefore === s.id || undefined}
        {...dragProps({ kind: 'session', id: s.id })}
        {...sessionDropProps(s)}
      >
        <span
          className="mode-rail"
          style={tab ? undefined : { background: 'transparent' }}
          aria-hidden
        />
        <span
          aria-hidden
          className={`w-2 shrink-0 text-center text-[9px] leading-none ${
            tab?.state === 'connected' ? 'text-success' : 'text-muted'
          }`}
        >
          {tab ? (tab.state === 'connected' ? '●' : '○') : ''}
        </span>
        {isRenaming ? (
          <RenameInput
            initial={s.name}
            label={t('library.rename')}
            onDone={(name) => {
              lib().setRenaming(null);
              refocus(sel);
              if (name !== null) void lib().renameSession(s.id, name);
            }}
          />
        ) : (
          <span className="mono flex-1 truncate">{s.name}</span>
        )}
        {kind && (
          <span className="env-letter w-3 shrink-0 text-center" title={envLabel(s.envTag) ?? ''}>
            {envLetter(s.envTag)}
          </span>
        )}
      </div>
    );
  };

  return (
    <section aria-labelledby="sessions-heading" className="flex flex-col">
      <div
        className="flex h-7 items-center pr-1 pl-3"
        {...dropProps(null)}
        data-drop={dropTarget === 'root' || undefined}
      >
        <h2 id="sessions-heading" className="label-caps flex-1">
          {t('shell.sessions')}
        </h2>
        <button
          type="button"
          className="icon-btn icon-btn-sm"
          aria-label={t('library.add')}
          aria-haspopup="menu"
          title={t('library.add')}
          onClick={(e) => addMenu.openBelow(e.currentTarget)}
        >
          <Plus size={14} />
        </button>
      </div>
      <div
        ref={treeRef}
        role="tree"
        aria-labelledby="sessions-heading"
        aria-multiselectable
        aria-description={t('library.bulkHint')}
        className={`flex min-h-6 flex-col px-1 ${dropTarget === 'root' ? 'rounded-sm bg-hover' : ''}`}
        {...dropProps(null)}
      >
        {tree.map(renderNode)}
        {loaded && tree.length === 0 && (
          <div className="px-2 py-1 text-sm text-muted">
            <p>{t('shell.noSessions')}</p>
            <button type="button" className="btn btn-sm mt-2" onClick={newSerialSession}>
              {t('actions.newSerialSession')}
            </button>
          </div>
        )}
      </div>
      {markedIds.length > 1 && <BulkBar ids={markedIds} onClear={clearMarks} />}
      {menu.at && (
        <Menu
          x={menu.at.x}
          y={menu.at.y}
          entries={menuEntries}
          onClose={menu.close}
          label={t('library.menu')}
        />
      )}
      {addMenu.at && (
        <Menu
          x={addMenu.at.x}
          y={addMenu.at.y}
          label={t('library.add')}
          onClose={addMenu.close}
          entries={[
            {
              label: t('actions.newSerialSession'),
              hint: formatChord('mod+n', mac),
              onSelect: newSerialSession,
            },
            { label: t('actions.newFolder'), onSelect: () => void lib().createFolder(null) },
          ]}
        />
      )}
    </section>
  );
}

function RenameInput({
  initial,
  label,
  onDone,
}: {
  initial: string;
  label: string;
  onDone: (name: string | null) => void;
}) {
  const [value, setValue] = useState(initial);
  const done = useRef(false);
  const finish = (name: string | null) => {
    if (done.current) return;
    done.current = true;
    onDone(name);
  };
  return (
    <input
      autoFocus
      aria-label={label}
      className="field mono h-5 flex-1 px-1 text-sm"
      value={value}
      maxLength={128}
      onFocus={(e) => e.currentTarget.select()}
      onChange={(e) => setValue(e.target.value)}
      onClick={(e) => e.stopPropagation()}
      onDoubleClick={(e) => e.stopPropagation()}
      onKeyDown={(e) => {
        e.stopPropagation();
        if (e.key === 'Enter') finish(value.trim() ? value : null);
        else if (e.key === 'Escape') finish(null);
      }}
      onBlur={() => finish(value.trim() ? value : null)}
    />
  );
}
