/**
 * Mode / environment colour customization (P3-T07), pure and tested.
 *
 * Global mode colours and environment colours replace the design-token values on <html>
 * (`--mode-config`, `--env-prod` …). Vendor overrides become `[data-vendor][data-mode]` rules
 * that set `--mode-current` for that vendor's panes, tabs and tree rows. Colours are only
 * accepted as `#rrggbb`, so nothing user-provided ever reaches CSS unvalidated.
 */
export const MODES = ['user', 'privileged', 'config', 'boot', 'shell'] as const;
export type Mode = (typeof MODES)[number];
export const ENVS = ['prod', 'lab', 'test'] as const;
export type Env = (typeof ENVS)[number];
/** Vendor families a colour override can target (from src/data/modes.json profiles). */
export const FAMILIES = [
  'cisco',
  'ruijie',
  'juniper',
  'aruba',
  'huawei',
  'fortinet',
  'alcatel',
] as const;
export type Family = (typeof FAMILIES)[number];
export const MAX_LABEL = 12;

export interface ColorSettings {
  modes: Partial<Record<Mode, string>>;
  labels: Partial<Record<Mode, string>>;
  vendors: Partial<Record<Family, Partial<Record<Mode, string>>>>;
  env: Partial<Record<Env, string>>;
}

export const NO_COLORS: ColorSettings = { modes: {}, labels: {}, vendors: {}, env: {} };

const HEX = /^#[0-9a-f]{6}$/i;
export const isHex = (v: unknown): v is string => typeof v === 'string' && HEX.test(v);

function pick<K extends string>(
  raw: unknown,
  keys: readonly K[],
  ok: (v: unknown) => string | null,
): Partial<Record<K, string>> {
  const out: Partial<Record<K, string>> = {};
  if (!raw || typeof raw !== 'object') return out;
  for (const k of keys) {
    const v = ok((raw as Record<string, unknown>)[k]);
    if (v !== null) out[k] = v;
  }
  return out;
}

const hex = (v: unknown) => (isHex(v) ? v.toLowerCase() : null);
const label = (v: unknown) => {
  if (typeof v !== 'string') return null;
  // Printable characters only (no control codes in the UI).
  const clean = [...v]
    .filter((c) => c >= ' ' && c !== '\u007f')
    .join('')
    .trim()
    .slice(0, MAX_LABEL);
  return clean || null;
};

/** A copy of `obj` without `key` (or with `key` = `value`). */
export function withKey<K extends string, V>(
  obj: Partial<Record<K, V>>,
  key: K,
  value: V | null,
): Partial<Record<K, V>> {
  const rest = Object.fromEntries(Object.entries(obj).filter(([k]) => k !== key)) as Partial<
    Record<K, V>
  >;
  if (value !== null) rest[key] = value;
  return rest;
}

/** Coerces a stored value into valid settings (unknown parts dropped). */
export function sanitizeColors(raw: unknown): ColorSettings {
  const r = (raw && typeof raw === 'object' ? raw : {}) as Record<string, unknown>;
  const vendors: ColorSettings['vendors'] = {};
  const rv =
    r.vendors && typeof r.vendors === 'object' ? (r.vendors as Record<string, unknown>) : {};
  for (const f of FAMILIES) {
    const m = pick(rv[f], MODES, hex);
    if (Object.keys(m).length) vendors[f] = m;
  }
  return {
    modes: pick(r.modes, MODES, hex),
    labels: pick(r.labels, MODES, label),
    vendors,
    env: pick(r.env, ENVS, hex),
  };
}

/** Token overrides for <html> (name → value; missing names fall back to the tokens). */
export function rootVariables(c: ColorSettings): Record<string, string> {
  const out: Record<string, string> = {};
  for (const m of MODES) if (c.modes[m]) out[`--mode-${m}`] = c.modes[m] as string;
  for (const e of ENVS) if (c.env[e]) out[`--env-${e}`] = c.env[e] as string;
  return out;
}

/** Stylesheet text for vendor overrides (empty when there are none). */
export function vendorCss(c: ColorSettings): string {
  const rules: string[] = [];
  for (const f of FAMILIES) {
    const modes = c.vendors[f];
    if (!modes) continue;
    for (const m of MODES) {
      const color = modes[m];
      if (!isHex(color)) continue;
      const extra = m === 'boot' ? ` --mode-boot: ${color};` : '';
      rules.push(`[data-vendor='${f}'][data-mode='${m}'] { --mode-current: ${color};${extra} }`);
    }
  }
  return rules.join('\n');
}
