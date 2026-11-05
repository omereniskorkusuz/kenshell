import { describe, expect, it } from 'vitest';
import { rootVariables, sanitizeColors, vendorCss } from './modeColors';

describe('mode colour settings', () => {
  it('keeps only valid colours, labels, modes and families', () => {
    const c = sanitizeColors({
      modes: { config: '#F0A43A', user: 'red', nope: '#000000' },
      labels: { config: '  KONFİG\n', user: 'x'.repeat(40), privileged: '' },
      vendors: {
        ruijie: { config: '#ff0000', boot: 'url(x)' },
        evil: { config: '#000000' },
        cisco: { user: '#123' },
      },
      env: { prod: '#aa0000', staging: '#00aa00' },
    });
    expect(c).toEqual({
      modes: { config: '#f0a43a' },
      labels: { config: 'KONFİG', user: 'x'.repeat(12) },
      vendors: { ruijie: { config: '#ff0000' } },
      env: { prod: '#aa0000' },
    });
    expect(sanitizeColors('garbage')).toEqual({ modes: {}, labels: {}, vendors: {}, env: {} });
  });

  it('turns settings into token overrides and vendor rules', () => {
    const c = sanitizeColors({
      modes: { privileged: '#112233' },
      env: { lab: '#445566' },
      vendors: { ruijie: { config: '#ff8800', boot: '#990099' } },
    });
    expect(rootVariables(c)).toEqual({ '--mode-privileged': '#112233', '--env-lab': '#445566' });
    expect(vendorCss(c)).toBe(
      "[data-vendor='ruijie'][data-mode='config'] { --mode-current: #ff8800; }\n" +
        "[data-vendor='ruijie'][data-mode='boot'] { --mode-current: #990099; --mode-boot: #990099; }",
    );
  });
});
