/** Detected consoles (live, hotplug) + driver hint cards. */
import { Copy, ExternalLink, RefreshCw, Usb } from 'lucide-react';
import { useT } from '../i18n';
import { copyText } from '../lib/clipboard';
import { DEFAULT_SERIAL, openExternal, type ConsolePort, type DriverHint } from '../lib/ipc';
import { chipLabel, useConsoles } from '../stores/consoles';
import { useTabs } from '../stores/tabs';
import { useUi } from '../stores/ui';
import { Menu, useMenu } from './Menu';
import { useState } from 'react';

export function ConsoleList() {
  const t = useT();
  const ports = useConsoles((s) => s.snapshot.ports);
  const hints = useConsoles((s) => s.snapshot.driverHints);
  const loaded = useConsoles((s) => s.loaded);
  const rescanning = useConsoles((s) => s.rescanning);

  return (
    <section aria-labelledby="consoles-heading" className="flex flex-col">
      <div className="flex h-7 items-center pr-1 pl-3">
        <h2 id="consoles-heading" className="label-caps flex-1">
          {t('shell.detectedConsoles')}
        </h2>
        <button
          type="button"
          className="icon-btn icon-btn-sm"
          aria-label={t('actions.rescanConsoles')}
          title={t('actions.rescanConsoles')}
          disabled={rescanning}
          onClick={() => void useConsoles.getState().rescan()}
        >
          <RefreshCw
            size={13}
            className={rescanning ? 'animate-spin motion-reduce:animate-none' : ''}
          />
        </button>
      </div>
      <ul className="flex flex-col gap-0.5 px-1">
        {ports.map((p) => (
          <ConsoleRow key={p.key} port={p} />
        ))}
      </ul>
      {loaded && ports.length === 0 && hints.length === 0 && (
        <p className="px-3 py-1 text-sm text-muted">{t('consoles.empty')}</p>
      )}
      {hints.map((h) => (
        <DriverHintCard key={`${h.chip.vid}:${h.chip.pid}`} hint={h} />
      ))}
    </section>
  );
}

function ConsoleRow({ port }: { port: ConsolePort }) {
  const t = useT();
  const fresh = useConsoles((s) => s.highlighted.has(port.key));
  const open = useTabs((s) =>
    s.tabs.find((x) => x.settings.port === port.path || x.settings.port === port.altPath),
  );
  const menu = useMenu();
  const connect = () => useTabs.getState().openConsole(port);
  const saveAs = () =>
    useUi.getState().openEditor({
      sessionId: null,
      initial: {
        name: port.lastDevice ?? port.displayName,
        protocol: 'serial',
        serial: { ...DEFAULT_SERIAL, port: port.path, baud: port.rememberedBaud },
      },
    });

  return (
    <li
      className={`group rounded-sm px-2 py-1 hover:bg-hover ${fresh ? 'row-new' : ''}`}
      onContextMenu={(e) => {
        e.preventDefault();
        menu.openAt(e.clientX, e.clientY);
      }}
      onDoubleClick={connect}
    >
      <div className="flex items-center gap-2">
        <Usb size={14} className="shrink-0 text-muted" aria-hidden />
        <span className="mono min-w-0 flex-1 truncate" title={port.path}>
          {port.displayName}
        </span>
        <button
          type="button"
          className="btn btn-sm"
          onClick={connect}
          aria-label={t('consoles.connectTo', { port: port.displayName })}
        >
          {open ? t('consoles.show') : t('consoles.connect')}
        </button>
      </div>
      <div className="mt-0.5 flex flex-wrap items-center gap-x-2 gap-y-0.5 pl-[22px] text-xs text-muted">
        <span className="chip h-4 px-1">{chipLabel(port)}</span>
        {port.rememberedBaud !== null && (
          <span className="mono tabular" title={t('consoles.rememberedBaud')}>
            {port.rememberedBaud}
          </span>
        )}
        {port.lastDevice && (
          <span className="mono truncate" title={t('consoles.lastDevice')}>
            {port.lastDevice}
          </span>
        )}
      </div>
      {menu.at && (
        <Menu
          x={menu.at.x}
          y={menu.at.y}
          label={port.displayName}
          onClose={menu.close}
          entries={[
            { label: t('consoles.connect'), onSelect: connect },
            { label: t('consoles.saveAsSession'), onSelect: saveAs },
          ]}
        />
      )}
    </li>
  );
}

function DriverHintCard({ hint }: { hint: DriverHint }) {
  const t = useT();
  const [copied, setCopied] = useState(false);
  const chip = `${hint.chip.vendor} ${hint.chip.model}`;
  const url = hint.chip.driverUrl;

  return (
    <div role="note" className="mx-2 my-1 rounded-md border border-subtle bg-app p-2 text-sm">
      <p className="text-primary">{t('consoles.driverMissing', { chip })}</p>
      {hint.chip.driverName && (
        <p className="mt-1 text-xs text-muted">
          {t('consoles.driverName')}: <span className="text-secondary">{hint.chip.driverName}</span>
        </p>
      )}
      <div className="mt-2 flex flex-wrap items-center gap-1">
        {url && (
          <>
            <button type="button" className="btn btn-sm" onClick={() => openExternal(url)}>
              <ExternalLink size={12} aria-hidden />
              {t('consoles.driverPage')}
            </button>
            <button
              type="button"
              className="icon-btn icon-btn-sm"
              aria-label={t('consoles.copyLink')}
              title={copied ? t('consoles.copied') : t('consoles.copyLink')}
              onClick={() =>
                void copyText(url).then((ok) => {
                  setCopied(ok);
                  useUi.getState().flash(ok ? t('consoles.copied') : t('consoles.copyFailed'));
                })
              }
            >
              <Copy size={12} />
            </button>
          </>
        )}
        <button
          type="button"
          className="btn btn-ghost btn-sm text-secondary"
          onClick={() => void useConsoles.getState().rescan()}
        >
          {t('consoles.rescanAfterInstall')}
        </button>
      </div>
    </div>
  );
}
