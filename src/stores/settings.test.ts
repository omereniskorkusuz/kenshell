import { describe, expect, it } from 'vitest';
import { DEFAULT_SETTINGS, sanitizeSettings, sanitizeValue } from './settings';

describe('terminal appearance settings', () => {
  it('defaults: steady block cursor, bundled font, 1.25 spacing', () => {
    expect(DEFAULT_SETTINGS['terminal.cursorBlink']).toBe(false);
    expect(DEFAULT_SETTINGS['terminal.cursorStyle']).toBe('block');
    expect(DEFAULT_SETTINGS['terminal.fontFamily']).toBe('');
    expect(DEFAULT_SETTINGS['terminal.lineHeight']).toBe(1.25);
  });

  it('coerces bad values', () => {
    expect(sanitizeValue('terminal.cursorStyle', 'beam')).toBe('block');
    expect(sanitizeValue('terminal.cursorBlink', 'yes')).toBe(false);
    expect(sanitizeValue('terminal.lineHeight', 9)).toBe(1.6);
    expect(sanitizeValue('terminal.lineHeight', '1.33')).toBe(1.35);
    expect(sanitizeValue('terminal.lineHeight', 'x')).toBe(1.25);
    expect(sanitizeValue('terminal.fontFamily', ' "Cascadia Mono"; } body { ')).toBe(
      '"Cascadia Mono"  body',
    );
    expect(sanitizeSettings({ 'terminal.cursorStyle': 'bar' })['terminal.cursorStyle']).toBe('bar');
  });
});
