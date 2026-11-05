import { describe, expect, it } from 'vitest';
import { OutputHistory } from './history';

const enc = new TextEncoder();

describe('terminal output history', () => {
  it('keeps text and byte chunks in order', () => {
    const h = new OutputHistory(4096);
    h.push('a\r\n');
    h.push(enc.encode('ğüş\r\n'));
    h.push(enc.encode('x'));
    h.push('\x1b[32mb\x1b[0m\r\n');
    const r = h.replay();
    expect(r).toHaveLength(3);
    expect(h.text()).toBe('a\r\nğüş\r\nx\x1b[32mb\x1b[0m\r\n');
  });

  it('drops the oldest lines, cutting only at line starts', () => {
    const line = (i: number) =>
      `\x1b[1mline ${String(i).padStart(4, '0')}\x1b[0m ${'x'.repeat(40)}\r\n`;
    for (const asBytes of [false, true]) {
      const h = new OutputHistory(1024);
      for (let i = 0; i < 200; i++) h.push(asBytes ? enc.encode(line(i)) : line(i));
      const text = h.text();
      expect(h.length).toBeLessThanOrEqual(1024);
      expect(text.startsWith('\x1b[1mline ')).toBe(true);
      expect(text.endsWith(line(199))).toBe(true);
    }
    const big = new OutputHistory(1024);
    big.push(Array.from({ length: 200 }, (_, i) => line(i)).join(''));
    expect(big.text().length).toBeLessThanOrEqual(1024);
    expect(big.text().startsWith('\x1b[1mline ')).toBe(true);
  });

  it('shrinks when the limit drops and clears', () => {
    const h = new OutputHistory(100_000);
    for (let i = 0; i < 1000; i++) h.push(`row ${i}\r\n`);
    h.setLimit(2048);
    expect(h.length).toBeLessThanOrEqual(2048);
    expect(h.text().endsWith('row 999\r\n')).toBe(true);
    h.clear();
    expect(h.text()).toBe('');
  });
});
