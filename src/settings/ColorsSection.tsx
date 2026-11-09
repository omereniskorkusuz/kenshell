/**
 * Mode and environment colours (P3-T07): every mode's colour and label, per-vendor
 * overrides and the environment tag colours; each row and everything at once can go back to
 * the design defaults. Changes apply live (token overrides, see app/effects.ts).
 */
import { RotateCcw } from 'lucide-react';
import { useState } from 'react';
import { useT, type TKey } from '../i18n';
import {
  ENVS,
  FAMILIES,
  MAX_LABEL,
  MODES,
  NO_COLORS,
  type ColorSettings,
  type Family,
  type Mode,
  withKey,
} from '../lib/modeColors';
import { useSettings } from '../stores/settings';
import { familyLabel } from '../lib/vendors';

/** Current value of a colour token (with any override already applied). */
function tokenColor(name: string): string {
  const v = getComputedStyle(document.documentElement).getPropertyValue(name).trim();
  return /^#[0-9a-f]{6}$/i.test(v) ? v : '#888888';
}

function ResetButton({ label, onClick }: { label: string; onClick: () => void }) {
  return (
    <button
      type="button"
      className="icon-btn icon-btn-sm"
      aria-label={label}
      title={label}
      onClick={onClick}
    >
      <RotateCcw size={12} />
    </button>
  );
}

export default function ColorsSection() {
  const t = useT();
  const colors = useSettings((s) => s.values['appearance.colors']);
  const [family, setFamily] = useState<Family>('ruijie');
  const save = (next: ColorSettings) => useSettings.getState().set('appearance.colors', next);
  const modeName = (m: Mode) => t(`mode.${m}` as TKey);

  const setMode = (m: Mode, color: string | null) =>
    save({ ...colors, modes: withKey(colors.modes, m, color) });
  const setLabel = (m: Mode, text: string) =>
    save({ ...colors, labels: withKey(colors.labels, m, text.trim().slice(0, MAX_LABEL) || null) });
  const setVendor = (m: Mode, color: string | null) => {
    const current = withKey(colors.vendors[family] ?? {}, m, color);
    const vendors = withKey(colors.vendors, family, Object.keys(current).length ? current : null);
    save({ ...colors, vendors });
  };
  const setEnv = (e: (typeof ENVS)[number], color: string | null) =>
    save({ ...colors, env: withKey(colors.env, e, color) });

  return (
    <div className="flex flex-col gap-3">
      <h3 className="label-caps">{t('colors.modes')}</h3>
      <p className="text-xs text-muted">{t('colors.modesHint')}</p>
      <div className="grid grid-cols-[6rem_auto_1fr_auto] items-center gap-x-3 gap-y-2">
        {MODES.map((m) => (
          <div key={m} className="contents" data-mode={m}>
            <span className="chip mode-chip w-fit">{colors.labels[m] ?? modeName(m)}</span>
            <input
              type="color"
              aria-label={t('colors.modeColor', { mode: modeName(m) })}
              className="h-7 w-10 cursor-pointer rounded-sm border border-subtle bg-transparent"
              value={colors.modes[m] ?? tokenColor(`--mode-${m}`)}
              onChange={(e) => setMode(m, e.target.value)}
            />
            <input
              className="field h-7 w-40 text-xs"
              aria-label={t('colors.modeLabel', { mode: modeName(m) })}
              placeholder={modeName(m)}
              maxLength={MAX_LABEL}
              defaultValue={colors.labels[m] ?? ''}
              onBlur={(e) => setLabel(m, e.target.value)}
            />
            {colors.modes[m] || colors.labels[m] ? (
              <ResetButton
                label={t('colors.reset')}
                onClick={() =>
                  save({
                    ...colors,
                    modes: withKey(colors.modes, m, null),
                    labels: withKey(colors.labels, m, null),
                  })
                }
              />
            ) : (
              <span />
            )}
          </div>
        ))}
      </div>

      <h3 className="label-caps mt-2">{t('colors.vendor')}</h3>
      <p className="text-xs text-muted">{t('colors.vendorHint')}</p>
      <select
        className="field w-48"
        aria-label={t('colors.vendor')}
        value={family}
        onChange={(e) => setFamily(e.target.value as Family)}
      >
        {FAMILIES.map((f) => (
          <option key={f} value={f}>
            {familyLabel(t, f)}
            {colors.vendors[f] ? ' •' : ''}
          </option>
        ))}
      </select>
      <div className="grid grid-cols-[6rem_auto_auto_1fr] items-center gap-x-3 gap-y-2">
        {MODES.map((m) => {
          const own = colors.vendors[family]?.[m];
          return (
            <div key={m} className="contents">
              <span className="text-sm text-secondary">{modeName(m)}</span>
              <input
                type="color"
                aria-label={t('colors.vendorColor', {
                  vendor: familyLabel(t, family),
                  mode: modeName(m),
                })}
                className="h-7 w-10 cursor-pointer rounded-sm border border-subtle bg-transparent"
                value={own ?? colors.modes[m] ?? tokenColor(`--mode-${m}`)}
                onChange={(e) => setVendor(m, e.target.value)}
              />
              {own ? (
                <ResetButton label={t('colors.inherit')} onClick={() => setVendor(m, null)} />
              ) : (
                <span className="text-xs text-muted">{t('colors.inherited')}</span>
              )}
              <span />
            </div>
          );
        })}
      </div>

      <h3 className="label-caps mt-2">{t('colors.env')}</h3>
      <div className="grid grid-cols-[6rem_auto_1fr] items-center gap-x-3 gap-y-2">
        {ENVS.map((e) => (
          <div key={e} className="contents">
            <span className="text-sm text-secondary">{e.toUpperCase()}</span>
            <input
              type="color"
              aria-label={t('colors.envColor', { env: e.toUpperCase() })}
              className="h-7 w-10 cursor-pointer rounded-sm border border-subtle bg-transparent"
              value={colors.env[e] ?? tokenColor(`--env-${e}`)}
              onChange={(ev) => setEnv(e, ev.target.value)}
            />
            {colors.env[e] ? (
              <ResetButton label={t('colors.reset')} onClick={() => setEnv(e, null)} />
            ) : (
              <span />
            )}
          </div>
        ))}
      </div>

      <button type="button" className="btn btn-sm mt-1 w-fit" onClick={() => save(NO_COLORS)}>
        <RotateCcw size={12} aria-hidden />
        {t('colors.resetAll')}
      </button>
    </div>
  );
}
