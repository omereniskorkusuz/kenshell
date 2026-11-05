import { describe, expect, it } from 'vitest';
import { DEFAULT_SETTINGS } from '../stores/settings';
import { PALETTE_TOGGLES, parsePaletteQuery } from './paletteQuery';

describe('palette query (P3-T17)', () => {
  it('prefixes pick a group', () => {
    expect(parsePaletteQuery('> theme')).toEqual({ mode: 'actions', search: 'theme' });
    expect(parsePaletteQuery('@core')).toEqual({ mode: 'sessions', search: 'core' });
    expect(parsePaletteQuery(' /vlan')).toEqual({ mode: 'snippets', search: 'vlan' });
    expect(parsePaletteQuery('usb', 'consoles')).toEqual({ mode: 'consoles', search: 'usb' });
    expect(parsePaletteQuery('x')).toEqual({ mode: 'all', search: 'x' });
  });

  it('palette toggles are boolean settings', () => {
    for (const [key] of PALETTE_TOGGLES) expect(typeof DEFAULT_SETTINGS[key]).toBe('boolean');
  });
});
