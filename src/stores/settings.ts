/**
 * App settings: loaded once via `api.settingsAll()`, validated against defaults, persisted per
 * key via `api.settingSet` (debounced so drag-resizing does not spam the backend).
 */
import { create } from 'zustand';
import { api, type Settings } from '../lib/ipc';

export type ThemePref = 'dark' | 'light' | 'system';
export type LanguagePref = 'system' | 'tr' | 'en';
export type StripeWidth = 4 | 8;

export interface AppSettings {
  'appearance.theme': ThemePref;
  'appearance.modeStripeWidth': StripeWidth;
  'general.language': LanguagePref;
  'terminal.fontSize': number;
  'terminal.scrollback': number;
  'console.hotplugToast': boolean;
  'console.autoReconnect': boolean;
  'connection.autoReconnect': boolean;
  'ui.sidebarWidth': number;
  'ui.sidebarCollapsed': boolean;
  'ui.sidePanelWidth': number;
  'ui.sidePanelPinned': boolean;
  'log.enabled': boolean;
  /** Empty = ~/Documents/KenShell Logs (decided by the backend). */
  'log.dir': string;
  'log.template': string;
  'log.mode': 'printable' | 'all';
  'log.timestamps': boolean;
}

export type SettingKey = keyof AppSettings;

export const DEFAULT_SETTINGS: AppSettings = {
  'appearance.theme': 'dark',
  'appearance.modeStripeWidth': 4,
  'general.language': 'system',
  'terminal.fontSize': 13,
  'terminal.scrollback': 10_000,
  'console.hotplugToast': true,
  'console.autoReconnect': true,
  'connection.autoReconnect': false,
  'ui.sidebarWidth': 220,
  'ui.sidebarCollapsed': false,
  'ui.sidePanelWidth': 320,
  'ui.sidePanelPinned': false,
  'log.enabled': false,
  'log.dir': '',
  'log.template': '{name}_{date}_{time}.log',
  'log.mode': 'printable',
  'log.timestamps': false,
};

export const LIMITS = {
  fontSize: { min: 8, max: 32 },
  scrollback: { min: 100, max: 100_000 },
  sidebarWidth: { min: 180, max: 320 },
  sidePanelWidth: { min: 280, max: 480 },
} as const;

const clampInt = (v: unknown, min: number, max: number, fallback: number): number => {
  const n = typeof v === 'number' ? v : typeof v === 'string' && v.trim() ? Number(v) : NaN;
  if (!Number.isFinite(n)) return fallback;
  return Math.min(max, Math.max(min, Math.round(n)));
};
const oneOf = <T extends string | number>(v: unknown, options: readonly T[], fallback: T): T =>
  options.includes(v as T) ? (v as T) : fallback;
const bool = (v: unknown, fallback: boolean): boolean => (typeof v === 'boolean' ? v : fallback);

/** Coerces one raw value into a valid setting (unknown/invalid → default). */
export function sanitizeValue<K extends SettingKey>(key: K, value: unknown): AppSettings[K] {
  const d = DEFAULT_SETTINGS;
  // A non-generic copy so `switch` narrows `d[k]` to the right type.
  const k: SettingKey = key;
  const out = ((): AppSettings[SettingKey] => {
    switch (k) {
      case 'appearance.theme':
        return oneOf(value, ['dark', 'light', 'system'] as const, d[k]);
      case 'appearance.modeStripeWidth':
        return oneOf(value, [4, 8] as const, d[k]);
      case 'general.language':
        return oneOf(value, ['system', 'tr', 'en'] as const, d[k]);
      case 'terminal.fontSize':
        return clampInt(value, LIMITS.fontSize.min, LIMITS.fontSize.max, d[k]);
      case 'terminal.scrollback':
        return clampInt(value, LIMITS.scrollback.min, LIMITS.scrollback.max, d[k]);
      case 'ui.sidebarWidth':
        return clampInt(value, LIMITS.sidebarWidth.min, LIMITS.sidebarWidth.max, d[k]);
      case 'ui.sidePanelWidth':
        return clampInt(value, LIMITS.sidePanelWidth.min, LIMITS.sidePanelWidth.max, d[k]);
      case 'console.hotplugToast':
      case 'console.autoReconnect':
      case 'connection.autoReconnect':
      case 'ui.sidebarCollapsed':
      case 'ui.sidePanelPinned':
      case 'log.enabled':
      case 'log.timestamps':
        return bool(value, d[k]);
      case 'log.mode':
        return oneOf(value, ['printable', 'all'] as const, d[k]);
      case 'log.dir':
        return typeof value === 'string' ? value.trim().slice(0, 1024) : d[k];
      case 'log.template':
        return typeof value === 'string' && value.trim() ? value.trim().slice(0, 200) : d[k];
      default:
        return d[k];
    }
  })();
  return out as AppSettings[K];
}

export function sanitizeSettings(raw: Settings | null | undefined): AppSettings {
  const out = { ...DEFAULT_SETTINGS };
  for (const key of Object.keys(DEFAULT_SETTINGS) as SettingKey[]) {
    if (raw && key in raw) (out as Record<SettingKey, unknown>)[key] = sanitizeValue(key, raw[key]);
  }
  return out;
}

interface SettingsState {
  values: AppSettings;
  loaded: boolean;
  load(): Promise<void>;
  set<K extends SettingKey>(key: K, value: AppSettings[K]): void;
}

const PERSIST_DELAY_MS = 300;
const pending = new Map<SettingKey, ReturnType<typeof setTimeout>>();

/** Called when persisting fails (wired to a toast by the app). */
let onPersistError: (e: unknown) => void = () => {};
export function setPersistErrorHandler(fn: (e: unknown) => void): void {
  onPersistError = fn;
}

export const useSettings = create<SettingsState>()((set, get) => ({
  values: DEFAULT_SETTINGS,
  loaded: false,
  async load() {
    try {
      const raw = await api.settingsAll();
      set({ values: sanitizeSettings(raw), loaded: true });
    } catch (e) {
      set({ loaded: true });
      onPersistError(e);
    }
  },
  set(key, value) {
    const clean = sanitizeValue(key, value);
    if (get().values[key] === clean) return;
    set({ values: { ...get().values, [key]: clean } });
    const prev = pending.get(key);
    if (prev) clearTimeout(prev);
    pending.set(
      key,
      setTimeout(() => {
        pending.delete(key);
        api.settingSet(key, get().values[key]).catch(onPersistError);
      }, PERSIST_DELAY_MS),
    );
  },
}));

/** Non-hook accessor for code outside React. */
export const getSetting = <K extends SettingKey>(key: K): AppSettings[K] =>
  useSettings.getState().values[key];
