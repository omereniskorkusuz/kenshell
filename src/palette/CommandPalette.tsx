/**
 * Command palette (Ctrl/Cmd+K, lazy chunk). Groups: Sessions, Detected consoles, Actions,
 * Settings. Prefix ">" = actions only, "@" = sessions only.
 */
import { Command, defaultFilter } from 'cmdk';
import { useEffect, useMemo, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { Settings2, SquareTerminal, Usb, Zap } from 'lucide-react';
import { useT } from '../i18n';
import { isMac } from '../lib/ipc';
import { formatChord } from '../lib/shortcuts';
import { envKind, envLabel, folderPath, isProd } from '../lib/tree';
import { ACTIONS, isEnabled } from '../app/actions';
import { chipLabel, useConsoles } from '../stores/consoles';
import { useLibrary } from '../stores/library';
import { useTabs } from '../stores/tabs';
import { useUi } from '../stores/ui';
import { parsePaletteQuery } from '../lib/paletteQuery';

export default function CommandPalette() {
  const t = useT();
  const initialQuery = useUi((s) => s.paletteQuery);
  const scope = useUi((s) => s.paletteScope);
  const [query, setQuery] = useState(initialQuery);
  const sessions = useLibrary((s) => s.sessions);
  const folders = useLibrary((s) => s.folders);
  const ports = useConsoles((s) => s.snapshot.ports);
  const mac = isMac();
  const [opener] = useState(() => document.activeElement as HTMLElement | null);
  const inputRef = useRef<HTMLInputElement>(null);

  const { mode, search } = parsePaletteQuery(query, scope);
  const close = () => useUi.getState().closePalette();
  const run = (fn: () => void) => {
    close();
    // Let the palette unmount (and focus return) before the action moves focus.
    setTimeout(fn, 0);
  };

  useEffect(() => {
    inputRef.current?.focus();
    return () => {
      if (
        opener?.isConnected &&
        (document.activeElement === document.body || !document.activeElement)
      ) {
        opener.focus();
      }
    };
  }, [opener]);

  const sortedSessions = useMemo(
    () =>
      [...sessions].sort(
        (a, b) =>
          Number(isProd(b.envTag)) - Number(isProd(a.envTag)) ||
          (b.lastConnectedAt ?? 0) - (a.lastConnectedAt ?? 0) ||
          a.name.localeCompare(b.name),
      ),
    [sessions],
  );

  const actions = ACTIONS.filter((a) => !a.hidden && isEnabled(a));
  const showSessions = mode === 'all' || mode === 'sessions';
  const showConsoles = mode === 'all' || mode === 'consoles';
  const showActions = mode === 'all' || mode === 'actions';

  return createPortal(
    <div
      className="fixed inset-0 flex items-start justify-center bg-app/50 pt-[12vh]"
      style={{ zIndex: 'var(--z-palette)' }}
      onPointerDown={(e) => {
        if (e.target === e.currentTarget) close();
      }}
    >
      <Command
        label={t('shell.commandPalette')}
        className="elevated flex max-h-[60vh] max-w-[calc(100vw-32px)] flex-col overflow-hidden"
        style={{ width: 'var(--palette-width)' }}
        shouldFilter
        loop
        filter={(value, _search, keywords) => (search ? defaultFilter(value, search, keywords) : 1)}
        onKeyDown={(e) => {
          if (e.key === 'Escape') {
            e.preventDefault();
            e.stopPropagation();
            close();
          }
        }}
      >
        <Command.Input
          ref={inputRef}
          value={query}
          onValueChange={setQuery}
          placeholder={t('palette.placeholder')}
          className="h-11 shrink-0 border-b border-subtle px-4 text-primary placeholder:text-muted"
        />
        <Command.List className="scroll-thin min-h-0 flex-1 overflow-y-auto py-1">
          <Command.Empty className="px-4 py-6 text-center text-sm text-muted">
            {t('palette.empty')}
          </Command.Empty>

          {showSessions && sortedSessions.length > 0 && (
            <Command.Group heading={t('palette.sessions')}>
              {sortedSessions.map((s) => (
                <Command.Item
                  key={`s${s.id}`}
                  value={`${s.name}\u2063${s.id}`}
                  keywords={[s.serial?.port ?? '', folderPath(folders, s.folderId), s.envTag ?? '']}
                  onSelect={() => run(() => useTabs.getState().openSession(s))}
                  data-env={envKind(s.envTag) ?? undefined}
                >
                  <SquareTerminal size={14} className="shrink-0 text-muted" aria-hidden />
                  <span className="mono min-w-0 flex-1 truncate">{s.name}</span>
                  {envLabel(s.envTag) && (
                    <span className="chip env-chip h-4 px-1">{envLabel(s.envTag)}</span>
                  )}
                  <span className="mono truncate text-xs text-muted">
                    {s.serial?.port.replace(/^\/dev\//, '')}
                  </span>
                </Command.Item>
              ))}
            </Command.Group>
          )}

          {showConsoles && ports.length > 0 && (
            <Command.Group heading={t('palette.consoles')}>
              {ports.map((p) => (
                <Command.Item
                  key={`c${p.key}`}
                  value={`${t('palette.connectConsole', { port: p.displayName })}\u2063${p.key}`}
                  keywords={[chipLabel(p), p.path, 'usb', 'console', 'konsol']}
                  onSelect={() => run(() => useTabs.getState().openConsole(p))}
                >
                  <Usb size={14} className="shrink-0 text-muted" aria-hidden />
                  <span className="min-w-0 flex-1 truncate">
                    {t('palette.connectConsole', { port: p.displayName })}
                  </span>
                  <span className="text-xs text-muted">{chipLabel(p)}</span>
                </Command.Item>
              ))}
            </Command.Group>
          )}

          {showActions && (
            <>
              <Command.Group heading={t('palette.actions')}>
                {actions
                  .filter((a) => a.group === 'actions')
                  .map((a) => (
                    <Command.Item
                      key={a.id}
                      value={`${t(a.title)}\u2063${a.id}`}
                      keywords={a.keywords ?? []}
                      onSelect={() => run(a.run)}
                    >
                      <Zap size={14} className="shrink-0 text-muted" aria-hidden />
                      <span className="min-w-0 flex-1 truncate">{t(a.title)}</span>
                      {a.shortcut && <span className="kbd">{formatChord(a.shortcut, mac)}</span>}
                    </Command.Item>
                  ))}
              </Command.Group>
              <Command.Group heading={t('palette.settings')}>
                {actions
                  .filter((a) => a.group === 'settings')
                  .map((a) => (
                    <Command.Item
                      key={a.id}
                      value={`${t(a.title)}\u2063${a.id}`}
                      keywords={a.keywords ?? []}
                      onSelect={() => run(a.run)}
                    >
                      <Settings2 size={14} className="shrink-0 text-muted" aria-hidden />
                      <span className="min-w-0 flex-1 truncate">{t(a.title)}</span>
                      {a.shortcut && <span className="kbd">{formatChord(a.shortcut, mac)}</span>}
                    </Command.Item>
                  ))}
              </Command.Group>
            </>
          )}
        </Command.List>
        <div className="flex shrink-0 items-center gap-3 border-t border-subtle px-4 py-1.5 text-xs text-muted">
          <span>
            <span className="kbd">&gt;</span> {t('palette.prefixActions')}
          </span>
          <span>
            <span className="kbd">@</span> {t('palette.prefixSessions')}
          </span>
        </div>
      </Command>
    </div>,
    document.body,
  );
}
