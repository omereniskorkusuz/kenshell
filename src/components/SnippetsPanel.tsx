/**
 * Snippet library (P3-T15), in the side panel: the user's snippets and the read-only Cisco /
 * Ruijie starter sets, by folder, filtered by vendor (the active device's by default) and
 * search. Send = pasted into the active tab (paced on serial/telnet, to the broadcast group
 * when it is on). Snippets with {{variables}} get a fill-in form (P3-T16).
 */
import { ChevronDown, ChevronRight, CopyPlus, Pencil, Play, Plus, Trash2 } from 'lucide-react';
import { useEffect, useMemo, useState } from 'react';
import { useT } from '../i18n';
import { describeError } from '../lib/errors';
import type { SnippetInput } from '../lib/ipc';
import {
  fillParts,
  fillSnippet,
  filterSnippets,
  groupSnippets,
  snippetPayload,
  snippetVariables,
  STARTER,
  toItem,
  type SnippetItem,
} from '../lib/snippets';
import { useSnippets } from '../stores/snippets';
import { useActiveTab, useTabs } from '../stores/tabs';
import { toast } from '../stores/toasts';

/** Vendor families (brand names, not translated). */
export const VENDOR_FAMILIES: [string, string][] = [
  ['cisco', 'Cisco'],
  ['ruijie', 'Ruijie'],
  ['juniper', 'Juniper'],
  ['alcatel', 'Alcatel-Lucent'],
  ['aruba', 'Aruba / HPE'],
  ['huawei', 'Huawei'],
  ['fortinet', 'Fortinet'],
];
const vendorLabel = (v: string | null) => VENDOR_FAMILIES.find(([id]) => id === v)?.[1] ?? '';

type Draft = SnippetInput & { id: number | null };

/** Last values typed per variable name, this run only (never saved: may be secrets). */
const lastValues: Record<string, string> = {};

export function SnippetsPanel() {
  const t = useT();
  const list = useSnippets((s) => s.list);
  const loaded = useSnippets((s) => s.loaded);
  const tab = useActiveTab();
  const family = tab?.device.vendor?.family ?? null;
  const [query, setQuery] = useState('');
  // null = follow the active device.
  const [vendorPick, setVendorPick] = useState<string | null>(null);
  const vendor =
    vendorPick ?? (family && VENDOR_FAMILIES.some(([id]) => id === family) ? family : 'all');
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());
  const [draft, setDraft] = useState<Draft | null>(null);
  const [filling, setFilling] = useState<SnippetItem | null>(null);

  useEffect(() => {
    if (!loaded) void useSnippets.getState().load();
  }, [loaded]);

  const groups = useMemo(
    () => groupSnippets(filterSnippets([...list.map(toItem), ...STARTER], vendor, query)),
    [list, vendor, query],
  );
  const folders = useMemo(() => [...new Set(list.map((s) => s.folder).filter(Boolean))], [list]);
  const canSend = tab?.state === 'connected';

  const fail = (e: unknown) => {
    const d = describeError(e, t);
    toast({ kind: 'error', message: d.message, detail: d.detail });
  };
  const sendBody = (body: string) => {
    if (!tab || tab.state !== 'connected') return;
    useTabs.getState().pasteInput(tab.key, snippetPayload(body));
    useTabs.getState().activate(tab.key);
  };
  const send = (s: SnippetItem) => {
    if (snippetVariables(s.body).length > 0) setFilling(s);
    else sendBody(s.body);
  };
  const remove = async (s: SnippetItem) => {
    if (s.id === null) return;
    try {
      await useSnippets.getState().remove(s.id);
      const input = { folder: s.folder, name: s.name, body: s.body, vendor: s.vendor };
      toast({
        message: t('snippets.deleted', { name: s.name }),
        actions: [
          {
            label: t('common.undo'),
            primary: true,
            run: () => void useSnippets.getState().save(null, input).catch(fail),
          },
        ],
      });
    } catch (e) {
      fail(e);
    }
  };

  if (filling) {
    return (
      <SnippetFill
        item={filling}
        hostname={tab?.device.hostname ?? null}
        canSend={canSend}
        onCancel={() => setFilling(null)}
        onSend={(values) => {
          sendBody(fillSnippet(filling.body, values));
          setFilling(null);
        }}
      />
    );
  }

  if (draft) {
    return (
      <SnippetEditor
        draft={draft}
        folders={folders}
        onCancel={() => setDraft(null)}
        onSave={async (d) => {
          try {
            await useSnippets.getState().save(d.id, d);
            setDraft(null);
          } catch (e) {
            fail(e);
          }
        }}
      />
    );
  }

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex items-center gap-1 border-b border-subtle p-2">
        <input
          className="field h-7 min-w-0 flex-1 text-sm"
          type="search"
          placeholder={t('snippets.search')}
          aria-label={t('snippets.search')}
          value={query}
          onChange={(e) => setQuery(e.target.value)}
        />
        <select
          className="field h-7 w-28 text-sm"
          aria-label={t('snippets.vendor')}
          value={vendor}
          onChange={(e) => setVendorPick(e.target.value)}
        >
          <option value="all">{t('snippets.allVendors')}</option>
          {VENDOR_FAMILIES.map(([id, label]) => (
            <option key={id} value={id}>
              {label}
            </option>
          ))}
        </select>
        <button
          type="button"
          className="icon-btn icon-btn-sm"
          aria-label={t('snippets.new')}
          title={t('snippets.new')}
          onClick={() =>
            setDraft({
              id: null,
              folder: '',
              name: '',
              body: '',
              vendor: vendor === 'all' ? null : vendor,
            })
          }
        >
          <Plus size={14} />
        </button>
      </div>
      {!canSend && <p className="px-3 pt-2 text-xs text-muted">{t('snippets.connectFirst')}</p>}
      <div className="min-h-0 flex-1 overflow-y-auto p-1">
        {groups.length === 0 && (
          <p className="p-4 text-center text-sm text-muted">{t('snippets.none')}</p>
        )}
        {groups.map(([folder, items]) => {
          const open = !collapsed.has(folder);
          const builtin = items[0]?.builtin ?? false;
          return (
            <section key={`${builtin}:${folder}`} className="mb-1">
              <button
                type="button"
                className="flex h-6 w-full items-center gap-1 px-1 text-left text-xs font-medium text-secondary"
                aria-expanded={open}
                onClick={() =>
                  setCollapsed((c) => {
                    const n = new Set(c);
                    if (open) n.add(folder);
                    else n.delete(folder);
                    return n;
                  })
                }
              >
                {open ? <ChevronDown size={12} /> : <ChevronRight size={12} />}
                <span className="truncate">{folder || t('snippets.mine')}</span>
                {builtin && <span className="ml-1 normal-case">· {t('snippets.starter')}</span>}
              </button>
              {open && (
                <ul>
                  {items.map((s) => (
                    <SnippetRow
                      key={s.key}
                      item={s}
                      canSend={canSend}
                      onSend={() => send(s)}
                      onEdit={() =>
                        setDraft({
                          id: s.builtin ? null : s.id,
                          folder: s.builtin ? s.folder : s.folder,
                          name: s.builtin ? `${s.name} (${t('snippets.copy')})` : s.name,
                          body: s.body,
                          vendor: s.vendor,
                        })
                      }
                      onDelete={() => void remove(s)}
                    />
                  ))}
                </ul>
              )}
            </section>
          );
        })}
      </div>
    </div>
  );
}

function SnippetRow({
  item,
  canSend,
  onSend,
  onEdit,
  onDelete,
}: {
  item: SnippetItem;
  canSend: boolean;
  onSend: () => void;
  onEdit: () => void;
  onDelete: () => void;
}) {
  const t = useT();
  const vars = snippetVariables(item.body);
  const first = item.body.split('\n').find((l) => l.trim() && !/^conf(igure)? t/i.test(l)) ?? '';
  return (
    <li className="group flex items-start gap-1 rounded-sm px-1 py-1 hover:bg-hover">
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-1.5">
          <span className="truncate text-sm">{item.name}</span>
          {item.vendor && (
            <span className="shrink-0 text-[10px] text-muted">{vendorLabel(item.vendor)}</span>
          )}
        </div>
        <div className="mono truncate text-xs text-muted" title={item.body}>
          {first.trim()}
        </div>
        {vars.length > 0 && (
          <div className="truncate text-[10px] text-muted">
            {vars.map((v) => `{{${v}}}`).join(' ')}
          </div>
        )}
      </div>
      <div className="flex shrink-0 items-center opacity-70 group-hover:opacity-100">
        <button
          type="button"
          className="icon-btn icon-btn-sm"
          disabled={!canSend}
          aria-label={t('snippets.send', { name: item.name })}
          title={
            vars.length > 0 ? t('snippets.fillAndSend') : t('snippets.send', { name: item.name })
          }
          onClick={onSend}
        >
          <Play size={13} />
        </button>
        <button
          type="button"
          className="icon-btn icon-btn-sm"
          aria-label={item.builtin ? t('snippets.copyToMine') : t('snippets.edit')}
          title={item.builtin ? t('snippets.copyToMine') : t('snippets.edit')}
          onClick={onEdit}
        >
          {item.builtin ? <CopyPlus size={13} /> : <Pencil size={13} />}
        </button>
        {!item.builtin && (
          <button
            type="button"
            className="icon-btn icon-btn-sm"
            aria-label={t('snippets.delete', { name: item.name })}
            title={t('snippets.delete', { name: item.name })}
            onClick={onDelete}
          >
            <Trash2 size={13} />
          </button>
        )}
      </div>
    </li>
  );
}

function SnippetEditor({
  draft,
  folders,
  onCancel,
  onSave,
}: {
  draft: Draft;
  folders: string[];
  onCancel: () => void;
  onSave: (d: Draft) => Promise<void>;
}) {
  const t = useT();
  const [d, setD] = useState(draft);
  const valid = d.name.trim() !== '' && d.body.trim() !== '';
  const save = () => {
    if (valid) void onSave(d);
  };
  return (
    <form
      className="flex min-h-0 flex-1 flex-col gap-2 overflow-y-auto p-2"
      aria-label={d.id === null ? t('snippets.new') : t('snippets.edit')}
      onSubmit={(e) => {
        e.preventDefault();
        save();
      }}
      onKeyDown={(e) => {
        if (e.key === 'Escape') {
          e.stopPropagation();
          onCancel();
        } else if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) {
          e.preventDefault();
          save();
        }
      }}
    >
      <label className="flex flex-col gap-1 text-xs text-secondary">
        {t('snippets.name')}
        <input
          className="field h-7 text-sm"
          value={d.name}
          maxLength={100}
          autoFocus
          onChange={(e) => setD({ ...d, name: e.target.value })}
        />
      </label>
      <div className="flex gap-2">
        <label className="flex min-w-0 flex-1 flex-col gap-1 text-xs text-secondary">
          {t('snippets.folder')}
          <input
            className="field h-7 text-sm"
            list="snippet-folders"
            value={d.folder}
            maxLength={100}
            placeholder={t('snippets.mine')}
            onChange={(e) => setD({ ...d, folder: e.target.value })}
          />
          <datalist id="snippet-folders">
            {folders.map((f) => (
              <option key={f} value={f} />
            ))}
          </datalist>
        </label>
        <label className="flex w-32 flex-col gap-1 text-xs text-secondary">
          {t('snippets.vendor')}
          <select
            className="field h-7 text-sm"
            value={d.vendor ?? ''}
            onChange={(e) => setD({ ...d, vendor: e.target.value || null })}
          >
            <option value="">{t('snippets.anyVendor')}</option>
            {VENDOR_FAMILIES.map(([id, label]) => (
              <option key={id} value={id}>
                {label}
              </option>
            ))}
          </select>
        </label>
      </div>
      <label className="flex min-h-0 flex-1 flex-col gap-1 text-xs text-secondary">
        {t('snippets.body')}
        <textarea
          className="field mono min-h-40 flex-1 text-sm"
          spellCheck={false}
          value={d.body}
          placeholder={t('snippets.bodyHint')}
          onChange={(e) => setD({ ...d, body: e.target.value })}
        />
      </label>
      <div className="flex justify-end gap-2">
        <button type="button" className="btn btn-ghost" onClick={onCancel}>
          {t('common.cancel')}
        </button>
        <button type="submit" className="btn btn-primary" disabled={!valid}>
          {t('snippets.save')}
        </button>
      </div>
    </form>
  );
}

/** Fill-in form for a snippet's {{variables}} with a live preview (P3-T16). Not a warning:
 * it only asks for the values the template needs. */
function SnippetFill({
  item,
  hostname,
  canSend,
  onCancel,
  onSend,
}: {
  item: SnippetItem;
  hostname: string | null;
  canSend: boolean;
  onCancel: () => void;
  onSend: (values: Record<string, string>) => void;
}) {
  const t = useT();
  const names = snippetVariables(item.body);
  const [values, setValues] = useState<Record<string, string>>(() =>
    Object.fromEntries(
      names.map((n) => [n, lastValues[n] ?? (n === 'hostname' && hostname ? hostname : '')]),
    ),
  );
  const complete = names.every((n) => (values[n] ?? '').trim() !== '');
  const submit = () => {
    if (!complete || !canSend) return;
    Object.assign(lastValues, values);
    onSend(values);
  };
  return (
    <form
      className="flex min-h-0 flex-1 flex-col gap-2 overflow-y-auto p-2"
      aria-label={t('snippets.fillTitle', { name: item.name })}
      onSubmit={(e) => {
        e.preventDefault();
        submit();
      }}
      onKeyDown={(e) => {
        if (e.key === 'Escape') {
          e.stopPropagation();
          onCancel();
        }
      }}
    >
      <p className="text-sm font-medium">{item.name}</p>
      {names.map((n, i) => (
        <label key={n} className="flex flex-col gap-1 text-xs text-secondary">
          <span className="mono">{n}</span>
          <input
            className="field mono h-7 text-sm"
            value={values[n] ?? ''}
            spellCheck={false}
            autoFocus={i === 0}
            onChange={(e) => setValues((v) => ({ ...v, [n]: e.target.value }))}
          />
        </label>
      ))}
      <div className="flex flex-col gap-1 text-xs text-secondary">
        {t('snippets.preview')}
        <pre className="mono max-h-64 overflow-auto rounded-sm border border-subtle bg-terminal p-2 text-xs text-primary">
          {fillParts(item.body, values).map((p, i) =>
            'text' in p ? (
              p.text
            ) : p.value.trim() ? (
              <mark key={i} className="snippet-var">
                {p.value}
              </mark>
            ) : (
              <mark key={i} className="snippet-var" data-empty>
                {`{{${p.name}}}`}
              </mark>
            ),
          )}
        </pre>
      </div>
      {!canSend && <p className="text-xs text-muted">{t('snippets.connectFirst')}</p>}
      <div className="flex justify-end gap-2">
        <button type="button" className="btn btn-ghost" onClick={onCancel}>
          {t('common.cancel')}
        </button>
        <button type="submit" className="btn btn-primary" disabled={!complete || !canSend}>
          {t('snippets.sendFilled')}
        </button>
      </div>
    </form>
  );
}
