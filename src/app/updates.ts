/**
 * App updates (P5-T10): "ask, then update". A while after start (setting
 * `updates.autoCheck`, on by default) the app asks the update server once; a newer
 * version shows a toast with an Install button — nothing installs without that click.
 * Builds without an updater config never ask anything.
 */
import { t } from '../i18n';
import { describeError } from '../lib/errors';
import { updates } from '../lib/ipc';
import { getSetting } from '../stores/settings';
import { toast } from '../stores/toasts';
import { useUi } from '../stores/ui';

const FIRST_CHECK_MS = 20_000;

export async function installUpdate(): Promise<void> {
  try {
    await updates.install((done, total) => {
      useUi
        .getState()
        .flash(
          total
            ? t('updates.downloading', { percent: Math.floor((done / total) * 100) })
            : t('updates.downloadingBytes'),
        );
    });
  } catch (e) {
    const d = describeError(e, t);
    toast({ kind: 'error', message: d.message, detail: d.detail });
  }
}

/** Checks now; `quiet` = no message when up to date or on error (startup check). */
export async function checkForUpdate(quiet: boolean): Promise<void> {
  try {
    const found = await updates.check();
    if (found) {
      toast({
        kind: 'info',
        message: t('updates.available', { version: found.version }),
        detail: found.notes,
        durationMs: 60_000,
        actions: [{ label: t('updates.install'), primary: true, run: () => void installUpdate() }],
      });
    } else if (!quiet) useUi.getState().flash(t('updates.upToDate'));
  } catch (e) {
    if (quiet) return;
    const d = describeError(e, t);
    toast({ kind: 'error', message: d.message, detail: d.detail });
  }
}

export function scheduleUpdateCheck(): () => void {
  const timer = window.setTimeout(() => {
    void updates
      .status()
      .then((s) => {
        if (s.configured && getSetting('updates.autoCheck')) void checkForUpdate(true);
      })
      .catch(() => {});
  }, FIRST_CHECK_MS);
  return () => window.clearTimeout(timer);
}
