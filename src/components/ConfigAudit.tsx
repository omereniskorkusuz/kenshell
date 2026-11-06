/**
 * Config analysis (P4-T07): the running-config (read hidden with `show running-config`)
 * checked against the vendor's best-practice rules (`src/data/config-rules.json`).
 * Offline: nothing is changed on the device; secrets in reported lines are masked.
 */
import { RotateCw, X } from 'lucide-react';
import { useEffect, useRef, useState } from 'react';
import { useT } from '../i18n';
import { describeError } from '../lib/errors';
import { api, type Finding } from '../lib/ipc';
import type { LiveTab } from '../lib/sessionState';
import { FindingList } from './Findings';

export function ConfigAudit({ tab, onClose }: { tab: LiveTab; onClose: () => void }) {
  const t = useT();
  const [busy, setBusy] = useState(false);
  const [findings, setFindings] = useState<Finding[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const alive = useRef(true);
  const family = tab.device.vendor?.family ?? null;

  const run = async () => {
    if (tab.runtimeId === null) return;
    setBusy(true);
    setError(null);
    setFindings(null);
    try {
      const config = await api.liveCapture(tab.runtimeId, 'show running-config', 180);
      const result = await api.configAudit(family, config);
      if (alive.current) setFindings(result);
    } catch (e) {
      if (alive.current) setError(describeError(e, t).message);
    } finally {
      if (alive.current) setBusy(false);
    }
  };

  useEffect(() => {
    alive.current = true;
    queueMicrotask(() => void run());
    return () => {
      alive.current = false;
    };
    // Runs once per opening.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex items-center gap-1 border-b border-subtle p-2">
        <span className="min-w-0 flex-1 truncate text-sm font-medium">
          {t('cfgAudit.title')} · <span className="mono">{tab.device.hostname ?? tab.name}</span>
        </span>
        <button
          type="button"
          className="icon-btn icon-btn-sm"
          disabled={busy || tab.state !== 'connected'}
          aria-label={t('diag.rerun')}
          title={t('diag.rerun')}
          onClick={() => void run()}
        >
          <RotateCw size={13} />
        </button>
        <button
          type="button"
          className="icon-btn icon-btn-sm"
          aria-label={t('common.close')}
          onClick={onClose}
        >
          <X size={13} />
        </button>
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto p-2">
        <p className="mb-2 text-xs text-muted">{t('cfgAudit.hint')}</p>
        {busy && (
          <p className="text-sm text-secondary" role="status">
            {t('cfgAudit.reading')}
          </p>
        )}
        {error && <p className="text-sm text-danger">{error}</p>}
        {findings && <FindingList findings={findings} prefix="cfgRule" />}
      </div>
    </div>
  );
}
