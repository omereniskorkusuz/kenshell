import { describe, expect, it } from 'vitest';
import { shortName, starLayout } from './topology';

describe('topology layout (P4-T10)', () => {
  it('puts neighbours on a circle starting at the top', () => {
    const p = starLayout(4, 200);
    expect(p[0]?.x).toBeCloseTo(100);
    expect(p[0]?.y).toBeCloseTo(28);
    expect(p[1]?.x).toBeCloseTo(172);
    expect(starLayout(0, 200)).toEqual([]);
  });
  it('short names', () => {
    expect(shortName('core-sw1.lab.example.com')).toBe('core-sw1');
    expect(shortName('a-very-long-hostname-x', 10)).toBe('a-very-lo…');
  });
});
