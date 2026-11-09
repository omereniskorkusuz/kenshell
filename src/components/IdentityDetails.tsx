/**
 * Identity details (P3-T06): everything known about a tab's connection and device in one
 * read-only sheet — protocol and target, SSH algorithms and host key fingerprint, serial port
 * settings, device facts from the identity command, CLI mode and the log file.
 */
import { Copy } from 'lucide-react';
import { useState, type ReactNode } from 'react';
import { useT, type TKey } from '../i18n';
import { copyText } from '../lib/clipboard';
import { formatUptime } from '../lib/format';
import { protocolOf, serialOf, type LiveTab } from '../lib/sessionState';
import { envLabel } from '../lib/tree';
import { useUi } from '../stores/ui';
import { Dialog } from './Dialog';
import { familyLabel } from '../lib/vendors';

interface Props {
  tab: LiveTab;
  onClose: () => void;
}

function Section({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="flex flex-col gap-1">
      <h3 className="label-caps">{title}</h3>
      <dl className="grid grid-cols-[minmax(8rem,auto)_1fr] gap-x-3 gap-y-1 text-sm">{children}</dl>
    </section>
  );
}

function Item({
  label,
  value,
  copy,
}: {
  label: string;
  value: ReactNode;
  copy?: string | undefined;
}) {
  const t = useT();
  if (value === null || value === undefined || value === '') return null;
  return (
    <>
      <dt className="text-secondary">{label}</dt>
      <dd className="mono flex min-w-0 items-center gap-1 break-all">
        <span className="min-w-0">{value}</span>
        {copy && (
          <button
            type="button"
            className="icon-btn icon-btn-sm shrink-0"
            aria-label={t('details.copy', { what: label })}
            title={t('details.copy', { what: label })}
            onClick={() =>
              void copyText(copy).then((ok) =>
                useUi.getState().flash(ok ? t('details.copied') : t('keys.copyFailed')),
              )
            }
          >
            <Copy size={12} />
          </button>
        )}
      </dd>
    </>
  );
}

export function IdentityDetails({ tab, onClose }: Props) {
  const t = useT();
  const proto = protocolOf(tab);
  const serial = serialOf(tab);
  const net = tab.netLink;
  const key = net?.hostKey ?? null;
  const facts = tab.device.facts;
  const vendor = tab.device.vendor;
  const target = tab.target;
  // Time of opening (the sheet is a snapshot; it is reopened to refresh).
  const [openedAt] = useState(() => Date.now());
  const yesNo = (v: boolean | null | undefined) =>
    v === null || v === undefined
      ? t('editor.lineDefault')
      : v
        ? t('editor.lineOn')
        : t('editor.lineOff');

  return (
    <Dialog title={t('details.title', { name: tab.name })} onClose={onClose} width="34rem">
      <div className="flex flex-col gap-4">
        <Section title={t('details.connection')}>
          <Item label={t('identity.protocol')} value={t(`protocol.${proto}` as TKey)} />
          {target.type === 'network' && (
            <Item
              label={t('details.target')}
              value={`${net?.host ?? target.host}:${net?.port ?? target.port ?? ''}`}
              copy={net?.host ?? target.host}
            />
          )}
          <Item label={t('editor.username')} value={net?.username ?? null} />
          <Item label={t('details.state')} value={t(`state.${tab.state}` as TKey)} />
          <Item
            label={t('details.since')}
            value={tab.connectedAt ? new Date(tab.connectedAt).toLocaleString() : null}
          />
          <Item
            label={t('identity.uptime')}
            value={tab.connectedAt ? formatUptime(openedAt - tab.connectedAt) : null}
          />
          <Item label={t('identity.env')} value={envLabel(tab.envTag)} />
        </Section>

        {key && (
          <Section title={t('details.security')}>
            <Item label={t('details.hostKey')} value={key.algorithm} />
            <Item label={t('details.fingerprint')} value={key.fingerprint} copy={key.fingerprint} />
            <Item label={t('details.kex')} value={key.kex} />
            <Item label={t('details.cipher')} value={key.cipher} />
            <Item label={t('details.mac')} value={key.mac} />
          </Section>
        )}

        {serial && (
          <Section title={t('details.serial')}>
            <Item
              label={t('editor.port')}
              value={tab.link?.path ?? serial.port}
              copy={tab.link?.path ?? serial.port}
            />
            <Item
              label={t('editor.baud')}
              value={
                tab.link
                  ? `${tab.link.baud}${tab.link.autoDetected ? ` (${t('details.autoDetected')})` : ''}`
                  : serial.baud === null
                    ? t('serial.autoBaud')
                    : String(serial.baud)
              }
            />
            <Item label={t('editor.framing')} value={tab.link?.framing ?? null} />
            <Item
              label={t('editor.flow')}
              value={t(`details.flow.${serial.flowControl}` as TKey)}
            />
            <Item label="DTR" value={yesNo(serial.dtr)} />
            <Item label="RTS" value={yesNo(serial.rts)} />
            <Item label={t('details.chip')} value={tab.link?.chip ?? null} />
          </Section>
        )}

        <Section title={t('details.device')}>
          <Item label={t('details.hostname')} value={tab.device.hostname} />
          <Item label={t('details.vendor')} value={vendor ? familyLabel(t, vendor.family) : null} />
          <Item label={t('details.model')} value={facts?.model ?? null} />
          <Item label={t('identity.osVersion')} value={facts?.version ?? null} />
          <Item
            label={t('identity.serial')}
            value={facts?.serial ?? null}
            copy={facts?.serial ?? undefined}
          />
          <Item label={t('identity.deviceUptime')} value={facts?.uptime ?? null} />
          {!vendor && !facts && !tab.device.hostname && (
            <p className="col-span-2 text-xs text-muted">{t('details.noDevice')}</p>
          )}
        </Section>

        <Section title={t('details.cli')}>
          <Item label={t('details.mode')} value={t(`mode.${tab.mode}` as TKey)} />
          <Item label={t('details.submode')} value={tab.submode} />
        </Section>

        {tab.logPath && (
          <Section title={t('details.log')}>
            <Item label={t('details.logFile')} value={tab.logPath} copy={tab.logPath} />
          </Section>
        )}
      </div>
    </Dialog>
  );
}
