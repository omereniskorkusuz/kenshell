/**
 * App settings: loaded once via `api.settingsAll()`, validated against defaults, persisted per
 * key via `api.settingSet` (debounced so drag-resizing does not spam the backend).
 */
import { create } from 'zustand';
import { api, type Settings } from '../lib/ipc';
import { NO_COLORS, sanitizeColors, type ColorSettings } from '../lib/modeColors';
import { sanitizeOverrides } from '../lib/shortcuts';
import { HIGHLIGHT_CATEGORIES, type HighlightCategory } from '../terminal/highlight';

export type ThemePref = 'dark' | 'light' | 'system';
export type LanguagePref = 'system' | 'tr' | 'en';
export type StripeWidth = 4 | 8;

export interface AppSettings {
  'appearance.theme': ThemePref;
  'appearance.modeStripeWidth': StripeWidth;
  /** Mode / vendor / environment colours and mode labels (P3-T07); empty = design tokens. */
  'appearance.colors': ColorSettings;
  'general.language': LanguagePref;
  'terminal.fontSize': number;
  'terminal.scrollback': number;
  /** Empty = the bundled JetBrains Mono (with system mono fallbacks). */
  'terminal.fontFamily': string;
  /** Line spacing × font size (1.0–1.6). */
  'terminal.lineHeight': number;
  'terminal.cursorStyle': 'block' | 'bar' | 'underline';
  /** Off by default (PuTTY habit; a blinking cursor also keeps the CPU awake). */
  'terminal.cursorBlink': boolean;
  /** Output colouring (P3-T10): on/off and the switched-off rule categories. */
  'terminal.highlight': boolean;
  /** Dim suggestion from history / built-ins, Tab or → accepts (P3-T12). */
  'terminal.autocomplete': boolean;
  /** Answer the device's `--More--` pager automatically (P3-T14, default off). */
  'terminal.autoMore': boolean;
  'terminal.highlightOff': HighlightCategory[];
  'console.hotplugToast': boolean;
  'console.autoReconnect': boolean;
  'connection.autoReconnect': boolean;
  /** Read the device identity in the background after login (P3-T04, default on). */
  'device.identify': boolean;
  'ui.sidebarWidth': number;
  'ui.sidebarCollapsed': boolean;
  'ui.sidePanelWidth': number;
  'ui.sidePanelPinned': boolean;
  'terminal.copyOnSelect': boolean;
  'terminal.rightClickPaste': boolean;
  /** What Backspace sends: DEL (0x7f, default) or ^H (0x08, some old devices). */
  'terminal.backspace': 'del' | 'ctrlH';
  'terminal.charset': 'utf-8' | 'iso-8859-9' | 'windows-1254' | 'cp437';
  'terminal.timestamps': boolean;
  /** Delay between lines when pasting into serial/telnet sessions (0 = off). */
  'terminal.pasteDelayMs': number;
  'log.enabled': boolean;
  /** Empty = ~/Documents/KenShell Logs (decided by the backend). */
  'log.dir': string;
  'log.template': string;
  'log.mode': 'printable' | 'all';
  'log.timestamps': boolean;
  /** Shortcut overrides: action id → notation, null = no shortcut (P5-T07). */
  'shortcuts.overrides': Record<string, string | null>;
  /** `auto` = WebGL when the GPU allows it, `dom` = always the DOM renderer. */
  'terminal.renderer': 'auto' | 'dom';
  /** Offer legacy SSH algorithms to every device (old IOS / RGOS). */
  'connection.sshLegacy': boolean;
  /** SSH keepalive seconds where the session sets none (0 = off). */
  'connection.sshKeepalive': number;
  /** Optional AI assistant (P5-T08): off by default; its code loads only when on. */
  'ai.enabled': boolean;
  /** Ask the update server once after start (P5-T10). */
  'updates.autoCheck': boolean;
  /** Linux: the serial group hint was shown once (P5-T12). */
  'console.groupHintShown': boolean;
  /** Model id for the AI assistant, typed by the user ('' = not set). */
  'ai.model': string;
}

export type SettingKey = keyof AppSettings;

export const DEFAULT_SETTINGS: AppSettings = {
  'appearance.theme': 'dark',
  'appearance.modeStripeWidth': 4,
  'appearance.colors': NO_COLORS,
  'general.language': 'system',
  'terminal.fontSize': 13,
  'terminal.scrollback': 10_000,
  'terminal.fontFamily': '',
  'terminal.lineHeight': 1.25,
  'terminal.cursorStyle': 'block',
  'terminal.cursorBlink': false,
  'terminal.highlight': true,
  'terminal.autocomplete': true,
  'terminal.autoMore': false,
  'terminal.highlightOff': [],
  'console.hotplugToast': true,
  'console.autoReconnect': true,
  'connection.autoReconnect': false,
  'device.identify': true,
  'ui.sidebarWidth': 220,
  'ui.sidebarCollapsed': false,
  'ui.sidePanelWidth': 320,
  'ui.sidePanelPinned': false,
  'terminal.copyOnSelect': true,
  'terminal.rightClickPaste': true,
  'terminal.backspace': 'del',
  'terminal.charset': 'utf-8',
  'terminal.timestamps': false,
  'terminal.pasteDelayMs': 50,
  'log.enabled': false,
  'log.dir': '',
  'log.template': '{name}_{date}_{time}.log',
  'log.mode': 'printable',
  'log.timestamps': false,
  'shortcuts.overrides': {},
  'terminal.renderer': 'auto',
  'connection.sshLegacy': false,
  'connection.sshKeepalive': 30,
  'ai.enabled': false,
  'updates.autoCheck': true,
  'console.groupHintShown': false,
  'ai.model': '',
};

export const LIMITS = {
  fontSize: { min: 8, max: 32 },
  scrollback: { min: 100, max: 100_000 },
  lineHeight: { min: 1, max: 1.6 },
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
      case 'appearance.colors':
        return sanitizeColors(value);
      case 'general.language':
        return oneOf(value, ['system', 'tr', 'en'] as const, d[k]);
      case 'terminal.fontSize':
        return clampInt(value, LIMITS.fontSize.min, LIMITS.fontSize.max, d[k]);
      case 'terminal.scrollback':
        return clampInt(value, LIMITS.scrollback.min, LIMITS.scrollback.max, d[k]);
      case 'terminal.lineHeight': {
        const n = typeof value === 'number' ? value : Number(value);
        if (!Number.isFinite(n)) return d[k];
        const { min, max } = LIMITS.lineHeight;
        return Math.round(Math.min(max, Math.max(min, n)) * 20) / 20;
      }
      case 'terminal.cursorStyle':
        return oneOf(value, ['block', 'bar', 'underline'] as const, d[k]);
      case 'terminal.fontFamily':
        // A CSS font-family list; quotes and commas allowed, nothing that ends the value.
        return typeof value === 'string'
          ? value
              .replace(/[;{}<>\\]/g, '')
              .trim()
              .slice(0, 200)
          : d[k];
      case 'ui.sidebarWidth':
        return clampInt(value, LIMITS.sidebarWidth.min, LIMITS.sidebarWidth.max, d[k]);
      case 'ui.sidePanelWidth':
        return clampInt(value, LIMITS.sidePanelWidth.min, LIMITS.sidePanelWidth.max, d[k]);
      case 'console.hotplugToast':
      case 'console.autoReconnect':
      case 'connection.autoReconnect':
      case 'device.identify':
      case 'ui.sidebarCollapsed':
      case 'ui.sidePanelPinned':
      case 'log.enabled':
      case 'terminal.copyOnSelect':
      case 'terminal.rightClickPaste':
      case 'terminal.timestamps':
      case 'terminal.cursorBlink':
      case 'terminal.highlight':
      case 'terminal.autocomplete':
      case 'terminal.autoMore':
      case 'log.timestamps':
      case 'connection.sshLegacy':
      case 'ai.enabled':
      case 'updates.autoCheck':
      case 'console.groupHintShown':
        return bool(value, d[k]);
      case 'ai.model':
        return typeof value === 'string' ? value.trim().slice(0, 100) : d[k];
      case 'shortcuts.overrides':
        return sanitizeOverrides(value);
      case 'terminal.renderer':
        return oneOf(value, ['auto', 'dom'] as const, d[k]);
      case 'connection.sshKeepalive':
        return clampInt(value, 0, 3600, d[k]);
      case 'terminal.highlightOff':
        return Array.isArray(value) ? HIGHLIGHT_CATEGORIES.filter((c) => value.includes(c)) : d[k];
      case 'terminal.backspace':
        return oneOf(value, ['del', 'ctrlH'] as const, d[k]);
      case 'terminal.charset':
        return oneOf(value, ['utf-8', 'iso-8859-9', 'windows-1254', 'cp437'] as const, d[k]);
      case 'terminal.pasteDelayMs':
        return clampInt(value, 0, 1000, d[k]);
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
