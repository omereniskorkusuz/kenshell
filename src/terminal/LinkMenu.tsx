/**
 * Action menu for a clicked IP / interface in terminal output (P3-T11).
 * IP: SSH, Telnet, ping from the device or from this computer (P5-T05), copy. Interface: the vendor's show / config
 * commands (sent to the session, read-only), add a note, copy.
 */
import { useT } from '../i18n';
import { api, DEFAULT_PORTS, DEFAULT_SSH, type NetProtocol } from '../lib/ipc';
import { Menu, type MenuEntry } from '../components/Menu';
import { useDiag } from '../stores/diag';
import { useTabs } from '../stores/tabs';
import { useUi } from '../stores/ui';
import { ifaceCommands, type TermLink } from './links';

export interface LinkMenuState {
  x: number;
  y: number;
  link: TermLink;
}

export function LinkMenu({
  tabKey,
  menu,
  onClose,
}: {
  tabKey: string;
  menu: LinkMenuState;
  onClose: () => void;
}) {
  const t = useT();
  const tab = useTabs((s) => s.tabs.find((x) => x.key === tabKey));
  const connected = tab?.state === 'connected';
  const { link } = menu;
  const send = (cmd: string) => useTabs.getState().write(tabKey, `${cmd}\r`);
  const copy = () => {
    void api.clipboardWrite(link.text).catch(() => {});
  };
  const open = (protocol: NetProtocol) =>
    useTabs.getState().openTarget({
      type: 'network',
      protocol,
      host: link.text,
      port: DEFAULT_PORTS[protocol],
      username: null,
      ssh: DEFAULT_SSH,
    });

  let entries: MenuEntry[];
  if (link.kind === 'ip') {
    entries = [
      { label: t('terminal.ssh', { text: link.text }), onSelect: () => open('ssh') },
      { label: t('terminal.telnet', { text: link.text }), onSelect: () => open('telnet') },
      {
        label: t('terminal.ping'),
        hint: `ping ${link.text}`,
        disabled: !connected,
        onSelect: () => send(`ping ${link.text}`),
      },
      {
        label: t('terminal.pingLocal'),
        onSelect: () => {
          useUi.getState().openTool('nettools');
          void useDiag.getState().start({ kind: 'ping', target: link.text });
        },
      },
      { kind: 'separator' },
      { label: t('terminal.copy'), onSelect: copy },
    ];
  } else {
    const cmds = ifaceCommands(tab?.device.vendor?.family ?? null, link.text);
    entries = [
      {
        label: t('terminal.showIface'),
        hint: cmds.show,
        disabled: !connected,
        onSelect: () => send(cmds.show),
      },
      ...(cmds.config
        ? [
            {
              label: t('terminal.showIfaceConfig'),
              hint: cmds.config,
              disabled: !connected,
              onSelect: () => send(cmds.config as string),
            },
          ]
        : []),
      { kind: 'separator' },
      {
        label: t('terminal.addNote'),
        disabled: !tab || (tab.sessionId === null && tab.device.deviceId === null),
        onSelect: () => useUi.getState().startNote(`${link.iface.short}: `),
      },
      { label: t('terminal.copy'), onSelect: copy },
    ];
  }
  return (
    <Menu
      x={menu.x}
      y={menu.y}
      entries={entries}
      onClose={onClose}
      label={t('terminal.linkMenu', { text: link.text })}
    />
  );
}
