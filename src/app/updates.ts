/**
 * App updates (P5-T10): "ask, then update". A while after start (setting
 * `updates.autoCheck`, on by default) the app asks the update server once; a newer
 * version opens the update card (`UpdatePrompt`) — nothing installs without a click.
 * Builds without an updater config never ask anything.
 */
import { t } from '../i18n';
import { describeError } from '../lib/errors';
import { updates } from '../lib/ipc';
import { getSetting } from '../stores/settings';
import { toast } from '../stores/toasts';
import { useUi } from '../stores/ui';
import { useUpdate } from '../stores/update';

const FIRST_CHECK_MS = 20_000;

export async function installUpdate(): Promise<void> {
  const u = useUpdate.getState();
  u.progress(0, null);
  try {
    // Restarts the app when done; until then the card shows the progress.
    await updates.install((done, total) => useUpdate.getState().progress(done, total));
  } catch (e) {
    useUpdate.getState().failed(describeError(e, t).message);
  }
}

/** Checks now; `quiet` = no message when up to date or on error (startup check). */
export async function checkForUpdate(quiet: boolean): Promise<void> {
  try {
    const found = await updates.check();
    if (found) useUpdate.getState().found(found);
    else if (!quiet) useUi.getState().flash(t('updates.upToDate'));
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
