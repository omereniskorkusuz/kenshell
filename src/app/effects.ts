/**
 * Applies settings to the document: theme (data-theme on <html>, 'system' follows the OS
 * live), language, and layout CSS variables. Runs outside React so it costs no re-renders.
 */
import { detectLocale, setLocale } from '../i18n';
import { rootVariables, vendorCss, type ColorSettings } from '../lib/modeColors';
import { useSettings, type AppSettings, type ThemePref } from '../stores/settings';

const DARK_QUERY = '(prefers-color-scheme: dark)';

export function resolveTheme(pref: ThemePref, systemDark: boolean): 'dark' | 'light' {
  if (pref === 'system') return systemDark ? 'dark' : 'light';
  return pref;
}

function systemPrefersDark(): boolean {
  return typeof matchMedia === 'function' ? matchMedia(DARK_QUERY).matches : true;
}

function apply(values: AppSettings): void {
  const root = document.documentElement;
  const theme = resolveTheme(values['appearance.theme'], systemPrefersDark());
  if (root.dataset.theme !== theme) root.dataset.theme = theme;

  const lang = values['general.language'];
  setLocale(lang === 'system' ? detectLocale(navigator.languages ?? []) : lang);

  root.style.setProperty('--mode-stripe-width', `${values['appearance.modeStripeWidth']}px`);
  root.style.setProperty('--sidebar-width', `${values['ui.sidebarWidth']}px`);
  root.style.setProperty('--side-panel-width', `${values['ui.sidePanelWidth']}px`);
  applyColors(values['appearance.colors']);
}

let appliedVars: string[] = [];
let lastColors: ColorSettings | null = null;

/** Mode / environment token overrides on <html>, vendor overrides in one <style>. */
function applyColors(colors: ColorSettings): void {
  if (colors === lastColors) return;
  lastColors = colors;
  const root = document.documentElement;
  const vars = rootVariables(colors);
  for (const name of appliedVars) if (!(name in vars)) root.style.removeProperty(name);
  for (const [name, value] of Object.entries(vars)) root.style.setProperty(name, value);
  appliedVars = Object.keys(vars);
  const css = vendorCss(colors);
  let el = document.getElementById('ks-vendor-colors');
  if (!css) {
    el?.remove();
    return;
  }
  if (!el) {
    el = document.createElement('style');
    el.id = 'ks-vendor-colors';
    document.head.appendChild(el);
  }
  el.textContent = css;
}

/** Installs the settings → document bridge. Returns a disposer. */
export function installSettingsEffects(): () => void {
  apply(useSettings.getState().values);
  const unsub = useSettings.subscribe((s, prev) => {
    if (s.values !== prev.values) apply(s.values);
  });
  const mq = typeof matchMedia === 'function' ? matchMedia(DARK_QUERY) : null;
  const onSystemChange = () => apply(useSettings.getState().values);
  mq?.addEventListener('change', onSystemChange);
  return () => {
    unsub();
    mq?.removeEventListener('change', onSystemChange);
  };
}
