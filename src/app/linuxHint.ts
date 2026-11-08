/**
 * Linux first run (P5-T12): if this user is not in the group that owns the serial
 * devices (`dialout` / `uucp`), say so once — with the command to fix it — instead of
 * letting the first console connection fail with "access denied". No dialog.
 */
import { t } from '../i18n';
import { api, isLinux } from '../lib/ipc';
import { getSetting, useSettings } from '../stores/settings';
import { toast } from '../stores/toasts';

export async function serialGroupHint(): Promise<void> {
  if (!isLinux() || getSetting('console.groupHintShown')) return;
  const access = await api.serialAccess().catch(() => null);
  if (!access || access.ok || !access.group) return;
  const command = t('consoles.groupCommand', { group: access.group });
  toast({
    kind: 'info',
    message: t('consoles.groupHint', { group: access.group }),
    detail: command,
    durationMs: 30_000,
    actions: [
      {
        label: t('consoles.groupCopy'),
        primary: true,
        run: () => void api.clipboardWrite(command).catch(() => {}),
      },
    ],
  });
  useSettings.getState().set('console.groupHintShown', true);
}
