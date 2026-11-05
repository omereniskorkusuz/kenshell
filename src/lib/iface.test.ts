import { describe, expect, it } from 'vitest';
import vectors from '../data/iface-cases.json';
import { canonicalName, parseIface, sameIface, scanIfaces } from './iface';

describe('interface normalization (shared vectors with the backend)', () => {
  it.each(vectors.cases)('$in', (c) => {
    const got = parseIface(c.in);
    expect(got?.key ?? null).toBe(c.key);
    if (got) {
      expect(got.short).toBe(c.short);
      expect(canonicalName(got)).toBe(c.canonical);
    }
  });

  it.each(vectors.scan)('scan: $text', (s) => {
    expect(scanIfaces(s.text).map((x) => x.iface.key)).toEqual(s.found);
  });

  it('Ruijie spaced and Cisco short forms are the same', () => {
    expect(sameIface('GigabitEthernet 0/24', 'Gi0/24')).toBe(true);
    expect(sameIface('Gi0/24', 'Te0/24')).toBe(false);
    const text = 'x GigabitEthernet 0/24, y';
    const [hit] = scanIfaces(text);
    expect(hit && text.slice(hit.start, hit.end)).toBe('GigabitEthernet 0/24');
  });
});
