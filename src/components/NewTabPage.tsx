/**
 * "New session" tab (the + in the tab strip): a place of its own to start a connection
 * from. Type a target like in quick connect, or pick a recent connection, a detected
 * console or a saved session. The tab turns into the session it opens.
 */
import { Star, Usb } from 'lucide-react';
import { useEffect, useMemo, useRef, useState } from 'react';
import { useT } from '../i18n';
import { newSerialSession } from '../app/actions';
import { DEFAULT_PORTS, DEFAULT_SERIAL, DEFAULT_SSH } from '../lib/ipc';
import { parseQuickConnect } from '../lib/quickConnect';
import { chipLabel, useConsoles } from '../stores/consoles';
import { useLibrary } from '../stores/library';
import { useSettings } from '../stores/settings';
import { useTabs } from '../stores/tabs';
import { LogoMark } from './Logo';

export function NewTabPage() {
  const t = useT();
  const [value, setValue] = useState('');
  const input = useRef<HTMLInputElement>(null);
  const sessions = useLibrary((s) => s.sessions);
  const history = useSettings((s) => s.values['history.connections']);
  const ports = useConsoles((s) => s.snapshot.ports);
  const intent = useMemo(() => parseQuickConnect(value, sessions), [value, sessions]);
  const favorites = useMemo(
    () => [...sessions].sort((a, b) => Number(b.favorite) - Number(a.favorite)).slice(0, 8),
    [sessions],
  );

  useEffect(() => {
    input.current?.focus();
  }, []);

  const submit = () => {
    const tabs = useTabs.getState();
    if (intent.kind === 'session') {
      const s = sessions.find((x) => x.id === intent.session.id);
      if (s) tabs.openSession(s);
    } else if (intent.kind === 'serial') {
      tabs.openSerial({ ...DEFAULT_SERIAL, port: intent.port, baud: intent.baud });
    } else if (intent.kind === 'network') {
      tabs.openTarget({
        type: 'network',
        protocol: intent.protocol,
        host: intent.host,
        port: intent.port ?? DEFAULT_PORTS[intent.protocol],
        username: intent.username,
        ssh: DEFAULT_SSH,
      });
    }
  };

  const row = 'tree-row w-full pl-2 text-left';
  return (
    <div className="flex h-full overflow-y-auto bg-app" data-testid="new-tab">
      <div className="mx-auto flex w-full max-w-xl flex-col gap-5 px-6 py-10">
        <div className="flex items-center gap-3">
          <LogoMark size={36} />
          <h1 className="text-lg font-medium text-primary">{t('newTab.title')}</h1>
        </div>
        <div className="flex gap-2">
          <input
            ref={input}
            className="field mono h-9 min-w-0 flex-1 text-sm"
            placeholder={t('newTab.placeholder')}
            aria-label={t('quick.label')}
            spellCheck={false}
            autoCapitalize="off"
            autoComplete="off"
            value={value}
            onChange={(e) => setValue(e.target.value)}
            onKeyDown={(e) => e.key === 'Enter' && submit()}
          />
          <button
            type="button"
            className="btn btn-primary"
            disabled={!['session', 'serial', 'network'].includes(intent.kind)}
            onClick={submit}
          >
            {t('newTab.connect')}
          </button>
        </div>
        <p className="-mt-3 text-xs text-muted">{t('newTab.hint')}</p>

        {ports.length > 0 && (
          <section className="flex flex-col gap-1">
            <h2 className="label-caps">{t('shell.detectedConsoles')}</h2>
            {ports.map((p) => (
              <button
                key={p.path}
                type="button"
                className={row}
                onClick={() => useTabs.getState().openConsole(p)}
              >
                <Usb size={13} className="shrink-0 text-muted" aria-hidden />
                <span className="mono flex-1 truncate">{p.displayName}</span>
                <span className="text-xs text-muted">{chipLabel(p)}</span>
              </button>
            ))}
          </section>
        )}

        {history.length > 0 && (
          <section className="flex flex-col gap-1">
            <h2 className="label-caps">{t('shell.recent')}</h2>
            {history.slice(0, 8).map((h) => {
              const saved =
                h.sessionId !== null ? sessions.find((s) => s.id === h.sessionId) : null;
              return (
                <button
                  key={h.key}
                  type="button"
                  className={row}
                  onClick={() =>
                    saved
                      ? useTabs.getState().openSession(saved)
                      : useTabs.getState().openTarget(h.target, { name: h.name, envTag: h.envTag })
                  }
                >
                  <span className="mono flex-1 truncate">{saved?.name ?? h.name}</span>
                </button>
              );
            })}
          </section>
        )}

        {favorites.length > 0 && (
          <section className="flex flex-col gap-1">
            <h2 className="label-caps">{t('shell.sessions')}</h2>
            {favorites.map((s) => (
              <button
                key={s.id}
                type="button"
                className={row}
                onClick={() => useTabs.getState().openSession(s)}
              >
                {s.favorite && <Star size={12} className="shrink-0 text-warning" aria-hidden />}
                <span className="mono flex-1 truncate">{s.name}</span>
                <span className="truncate text-xs text-muted">
                  {s.host ?? s.serial?.port ?? ''}
                </span>
              </button>
            ))}
          </section>
        )}

        <div>
          <button type="button" className="btn btn-sm" onClick={newSerialSession}>
            {t('actions.newSerialSession')}
          </button>
        </div>
      </div>
    </div>
  );
}
