import { describe, expect, it } from 'vitest';
import en from './en.json';
import tr from './tr.json';
import { detectLocale, translate } from './index';

function keys(obj: object, prefix = ''): string[] {
  return Object.entries(obj).flatMap(([k, v]) =>
    typeof v === 'string' ? [`${prefix}${k}`] : keys(v as object, `${prefix}${k}.`),
  );
}

describe('i18n', () => {
  it('tr and en have exactly the same keys', () => {
    expect(keys(tr).sort()).toEqual(keys(en).sort());
  });

  it('interpolates parameters', () => {
    expect(translate('en', 'shell.version', { version: '1.2.3' })).toBe('v1.2.3');
  });

  it('selects plural forms', () => {
    expect(translate('en', 'unit.session', { count: 1 })).toBe('1 session');
    expect(translate('en', 'unit.session', { count: 3 })).toBe('3 sessions');
    expect(translate('tr', 'unit.session', { count: 3 })).toBe('3 oturum');
  });

  it('detects locale from system languages', () => {
    expect(detectLocale(['tr-TR', 'en-US'])).toBe('tr');
    expect(detectLocale(['de-DE', 'en-GB'])).toBe('en');
    expect(detectLocale([])).toBe('en');
  });
});
