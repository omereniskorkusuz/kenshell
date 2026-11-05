/**
 * Identity bar (Concept): always shows what this pane is connected to.
 *   row 1: SERIAL  COM4 @ 9600 8N1 · FTDI FT232R  ›  core-sw1
 *   row 2: [mode chip] [env chip]  uptime  ● status        [Reconnect] [BREAK] [panel]
 * Compact (pane < 800px tall or < 1100px wide): one 32px row; drop order uptime → chip →
 * protocol. Mode, env, name and status never drop.
 *
 * Mode/env colors come from data-mode/data-env on the pane (CSS variables), never React state.
 */
import { useEffect, useState } from 'react';
import { KeyRound, PanelRight, RotateCw } from 'lucide-react';
import { useT, type TKey } from '../i18n';
import { compactParts } from '../lib/compact';
import { formatUptime, formatUptimeShort, msToNextMinute } from '../lib/format';
import { isMac } from '../lib/ipc';
import { protocolOf, serialOf, tabStatus, targetLabel, type LiveTab } from '../lib/sessionState';
import { formatChord } from '../lib/shortcuts';
import { envLabel } from '../lib/tree';
import { useTabs } from '../stores/tabs';
import { useUi } from '../stores/ui';
import { Menu, useMenu, type MenuEntry } from './Menu';

export const COMMON_BAUDS = [9600, 19200, 38400, 57600, 115200] as const;

interface Props {
  tab: LiveTab;
  compact: boolean;
  width: number;
  visible: boolean;
}

export function IdentityBar({ tab, compact, width, visible }: Props) {
  const t = useT();
  const mac = isMac();
  const status = tabStatus(tab, t);
  const sidePanelOpen = useUi((s) => s.sidePanelOpen);
  const baudMenu = useMenu();
  const parts = compact ? compactParts(width) : { uptime: true, chip: true, protocol: true };
  const env = envLabel(tab.envTag);
  const connected = tab.state === 'connected';
  const busy = tab.state === 'connecting' || tab.state === 'detecting';

  const serial = serialOf(tab);
  const proto = protocolOf(tab);
  const baudEntries: MenuEntry[] = serial
    ? [
        { kind: 'label', label: t('serial.changeBaud') },
        {
          label: t('serial.autoBaud'),
          checked: serial.baud === null,
          onSelect: () => useTabs.getState().changeBaud(tab.key, null),
        },
        ...COMMON_BAUDS.map((b) => ({
          label: String(b),
          checked: (tab.link?.baud ?? serial.baud) === b,
          onSelect: () => useTabs.getState().changeBaud(tab.key, b),
        })),
      ]
    : [];
  const hostKey = tab.netLink?.hostKey ?? null;
  const breakable = proto === 'serial' || proto === 'telnet';

  const protocol = parts.protocol && (
    <span className="label-caps shrink-0" title={t('identity.protocol')}>
      {t(`protocol.${proto}` as TKey)}
    </span>
  );
  const target = serial ? (
    <button
      type="button"
      className="mono min-w-0 truncate rounded-sm px-0.5 text-left text-secondary hover:bg-hover hover:text-primary"
      title={t('serial.changeBaud')}
      aria-haspopup="menu"
      onClick={(e) => baudMenu.openBelow(e.currentTarget)}
    >
      {targetLabel(tab, t, parts.chip)}
    </button>
  ) : (
    <span
      className="mono min-w-0 truncate px-0.5 text-secondary"
      title={
        hostKey
          ? t('ssh.hostKeyTitle', {
              algorithm: hostKey.algorithm,
              fingerprint: hostKey.fingerprint,
            })
          : undefined
      }
    >
      {targetLabel(tab, t, parts.chip)}
    </span>
  );
  const name = (
    <span className="mono min-w-0 shrink truncate font-semibold text-primary" title={tab.name}>
      {tab.name}
    </span>
  );
  const modeChip = (
    <span
      className="chip mode-chip shrink-0"
      title={tab.submode ? `${t('identity.modeHint')} · ${tab.submode}` : t('identity.modeHint')}
    >
      {t(`mode.${tab.mode}` as TKey)}
    </span>
  );
  // What the device told us: host name from the prompt (when it differs from the tab name)
  // and the vendor once identified (Ruijie is never shown as Cisco: "Cisco-like" until sure).
  const host = tab.device.hostname;
  const deviceText = [host && host !== tab.name ? host : null, tab.device.vendor?.label]
    .filter(Boolean)
    .join(' · ');
  const device = deviceText && (
    <span className="min-w-0 shrink-[3] truncate text-xs text-muted" title={t('identity.device')}>
      {deviceText}
    </span>
  );
  const envChip = env && (
    <span className="chip env-chip shrink-0" title={t('identity.env')}>
      {env}
    </span>
  );
  const statusEl = (
    <span className="flex min-w-0 items-center gap-1.5" title={status.detail ?? undefined}>
      <span
        className="status-dot"
        data-tone={tab.reloadAt !== null && connected ? 'busy' : status.tone}
        aria-hidden
      />
      {tab.reloadAt !== null && connected ? (
        <ReloadCounter since={tab.reloadAt} />
      ) : (
        <span className="tone-text truncate" data-tone={status.tone}>
          {status.text}
        </span>
      )}
      {status.suggestion !== null && (
        <>
          <span className="text-muted" aria-hidden>
            —
          </span>
          <button
            type="button"
            className="shrink-0 underline decoration-strong underline-offset-2 hover:text-primary"
            aria-haspopup="menu"
            onClick={(e) => baudMenu.openBelow(e.currentTarget)}
          >
            {t('autobaud.change')}
          </button>
        </>
      )}
    </span>
  );
  const uptime = parts.uptime && connected && tab.connectedAt !== null && (
    <Uptime since={tab.connectedAt} live={visible} />
  );
  const actions = (
    <div className="ml-auto flex shrink-0 items-center gap-1 pl-2">
      {tab.state === 'disconnected' && tab.error?.code === 'hostKeyChanged' && (
        <button
          type="button"
          className="btn btn-sm"
          title={t('ssh.forgetHostKeyHint')}
          onClick={() => useTabs.getState().forgetHostKey(tab.key)}
        >
          <KeyRound size={12} aria-hidden />
          {t('ssh.forgetHostKey')}
        </button>
      )}
      {tab.state === 'disconnected' && (
        <button
          type="button"
          className="btn btn-primary btn-sm"
          title={`${t('actions.reconnect')} (${formatChord('mod+shift+r', mac)})`}
          onClick={() => useTabs.getState().reconnect(tab.key)}
        >
          <RotateCw size={12} aria-hidden />
          {t('actions.reconnect')}
        </button>
      )}
      {breakable && (
        <button
          type="button"
          className="btn btn-sm mono"
          disabled={!connected}
          title={`${t('identity.breakHint')} (${formatChord('mod+shift+b', mac)})`}
          aria-label={t('actions.sendBreak')}
          onClick={() => useTabs.getState().sendBreak(tab.key)}
        >
          {t('identity.break')}
        </button>
      )}
      <button
        type="button"
        className="icon-btn"
        aria-pressed={sidePanelOpen}
        aria-label={t('actions.toggleSidePanel')}
        title={`${t('actions.toggleSidePanel')} (${formatChord('mod+j', mac)})`}
        onClick={() => useUi.getState().toggleSidePanel()}
      >
        <PanelRight size={15} />
      </button>
    </div>
  );

  return (
    <div
      className="shrink-0 border-b border-subtle bg-surface text-sm"
      aria-busy={busy || undefined}
      style={{ height: compact ? 'var(--identity-height-compact)' : 'var(--identity-height)' }}
    >
      {compact ? (
        <div className="flex h-full min-w-0 items-center gap-2 px-2">
          {modeChip}
          {envChip}
          {name}
          {protocol}
          <span className="min-w-0 shrink-[2] truncate">{target}</span>
          {statusEl}
          {uptime}
          {actions}
        </div>
      ) : (
        <div className="flex h-full flex-col justify-center gap-1 px-3">
          <div className="flex min-w-0 items-center gap-2">
            {protocol}
            {target}
            <span className="text-muted" aria-hidden>
              ›
            </span>
            {name}
            {device}
          </div>
          <div className="flex min-w-0 items-center gap-2">
            {modeChip}
            {envChip}
            {uptime}
            {statusEl}
            {actions}
          </div>
        </div>
      )}
      {baudMenu.at && (
        <Menu
          x={baudMenu.at.x}
          y={baudMenu.at.y}
          entries={baudEntries}
          onClose={baudMenu.close}
          label={t('serial.changeBaud')}
        />
      )}
    </div>
  );
}

/** "Device restarting · 01:42" while boot messages are seen (until the next prompt). */
function ReloadCounter({ since }: { since: number }) {
  const t = useT();
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const id = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(id);
  }, []);
  return (
    <span className="tone-text truncate" data-tone="busy" role="status">
      {t('identity.reloading', { time: formatUptime(now - since).slice(3) })}
    </span>
  );
}

/** Connected time; minute resolution, re-rendered only when the minute changes. */
function Uptime({ since, live }: { since: number; live: boolean }) {
  const t = useT();
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!live) return;
    const id = setTimeout(() => setNow(Date.now()), msToNextMinute(Date.now() - since) + 50);
    return () => clearTimeout(id);
  }, [live, now, since]);
  return (
    <span
      className="mono tabular shrink-0 text-xs text-secondary"
      title={`${t('identity.uptime')} · ${formatUptime(now - since)}`}
    >
      {formatUptimeShort(now - since)}
    </span>
  );
}
