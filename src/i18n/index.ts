/**
 * Minimal, typed i18n (see ADR-004). Two locales, flat dotted keys derived from en.json,
 * `{{var}}` interpolation and CLDR plurals via `Intl.PluralRules` (`key_one`, `key_other`).
 */
import { useSyncExternalStore } from 'react';
import en from './en.json';
import tr from './tr.json';

export type Locale = 'en' | 'tr';
export const LOCALES: readonly Locale[] = ['en', 'tr'];

type Dict = typeof en;
type Leaves<T, P extends string = ''> = {
  [K in keyof T & string]: T[K] extends string ? `${P}${K}` : Leaves<T[K], `${P}${K}.`>;
}[keyof T & string];
type StripPlural<K> = K extends `${infer B}_${'one' | 'other'}` ? B : K;

export type TKey = StripPlural<Leaves<Dict>>;
export type TParams = Record<string, string | number>;

const DICTS: Record<Locale, Dict> = { en, tr };

function lookup(dict: unknown, key: string): string | undefined {
  let node: unknown = dict;
  for (const part of key.split('.')) {
    if (node === null || typeof node !== 'object') return undefined;
    node = (node as Record<string, unknown>)[part];
  }
  return typeof node === 'string' ? node : undefined;
}

export function detectLocale(languages: readonly string[]): Locale {
  for (const lang of languages) {
    const base = lang.toLowerCase().split('-')[0];
    if (base === 'tr' || base === 'en') return base;
  }
  return 'en';
}

/** English until the language setting is applied (the default setting is English too). */
let current: Locale = 'en';
const listeners = new Set<() => void>();

export function getLocale(): Locale {
  return current;
}

export function setLocale(locale: Locale): void {
  if (locale === current) return;
  current = locale;
  document.documentElement.lang = locale;
  listeners.forEach((l) => l());
}

export function translate(locale: Locale, key: TKey, params?: TParams): string {
  let k: string = key;
  if (params && typeof params.count === 'number') {
    const cat = new Intl.PluralRules(locale).select(params.count);
    const pluralKey = `${key}_${cat === 'one' ? 'one' : 'other'}`;
    if (lookup(DICTS[locale], pluralKey) ?? lookup(DICTS.en, pluralKey)) k = pluralKey;
  }
  const raw = lookup(DICTS[locale], k) ?? lookup(DICTS.en, k) ?? key;
  if (!params) return raw;
  return raw.replace(/\{\{(\w+)\}\}/g, (m, name: string) =>
    name in params ? String(params[name]) : m,
  );
}

export function t(key: TKey, params?: TParams): string {
  return translate(current, key, params);
}

function subscribe(cb: () => void): () => void {
  listeners.add(cb);
  return () => listeners.delete(cb);
}

/** React hook: re-renders on locale change. */
export function useT(): typeof t {
  useSyncExternalStore(subscribe, getLocale, getLocale);
  return t;
}
