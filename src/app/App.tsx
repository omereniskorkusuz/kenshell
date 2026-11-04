import { useEffect, useState } from 'react';
import { useT } from '../i18n';
import { appInfo, appReady, modKey } from '../lib/ipc';

/**
 * Phase 0 shell: static "Quiet Console" layout built only from design tokens.
 * Real layout, panels and state arrive in P1-T01.
 */
export function App() {
  const t = useT();
  const [version, setVersion] = useState<string | null>(null);

  useEffect(() => {
    // First commit done → show the (initially hidden) window. Note: rAF does not fire
    // while a WKWebView window is hidden, so this must not wait for an animation frame.
    void appReady();
    void appInfo().then((info) => setVersion(info?.version ?? null));
  }, []);

  const shortcut = `${modKey()}+K`;

  return (
    <div className="flex h-full flex-col">
      <header
        className="flex shrink-0 items-center gap-3 border-b border-subtle bg-surface px-3"
        style={{ height: 'var(--top-bar-height)' }}
      >
        <span className="font-semibold tracking-tight">{t('app.name')}</span>
        <button
          type="button"
          className="h-7 w-80 max-w-[40vw] rounded-md border border-subtle bg-app px-2 text-left text-muted hover:border-strong"
        >
          {t('shell.quickConnect')}
        </button>
        <div className="ml-auto flex items-center gap-1 text-secondary">
          <button type="button" className="rounded-md px-2 py-1 hover:bg-hover">
            {t('shell.scanNetwork')}
          </button>
          <button
            type="button"
            className="rounded-md px-2 py-1 font-mono text-xs hover:bg-hover"
            aria-label={t('shell.commandPalette')}
          >
            {shortcut}
          </button>
          <button type="button" className="rounded-md px-2 py-1 hover:bg-hover">
            {t('shell.settings')}
          </button>
        </div>
      </header>

      <div className="flex min-h-0 flex-1">
        <nav
          className="flex shrink-0 flex-col gap-4 border-r border-subtle bg-surface p-3"
          style={{ width: 'var(--sidebar-width)' }}
        >
          <section>
            <h2 className="label-caps mb-1">{t('shell.sessions')}</h2>
            <p className="text-sm text-muted">{t('shell.noSessions')}</p>
          </section>
          <section>
            <h2 className="label-caps mb-1">{t('shell.detectedConsoles')}</h2>
          </section>
          <section>
            <h2 className="label-caps mb-1">{t('shell.recent')}</h2>
          </section>
        </nav>

        <main className="flex min-w-0 flex-1 bg-terminal" data-mode="unknown">
          <div className="mode-stripe shrink-0" />
          <div className="m-auto max-w-md text-center">
            <h1 className="text-lg font-medium text-secondary">{t('shell.emptyTitle')}</h1>
            <p className="mt-1 text-sm text-muted">{t('shell.emptyHint', { shortcut })}</p>
          </div>
        </main>
      </div>

      <footer
        className="flex shrink-0 items-center gap-4 border-t border-subtle bg-surface px-3 text-xs text-muted"
        style={{ height: 'var(--status-bar-height)' }}
      >
        <span>{t('shell.statusReady')}</span>
        {version && <span className="ml-auto font-mono">{t('shell.version', { version })}</span>}
      </footer>
    </div>
  );
}
