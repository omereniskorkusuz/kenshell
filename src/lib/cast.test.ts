import { describe, expect, it } from 'vitest';
import { formatClock, indexAt, parseCast, parseSize } from './cast';

const CAST = [
  JSON.stringify({ version: 2, width: 120, height: 40, timestamp: 1793923200, title: 'core-sw1' }),
  JSON.stringify([0.5, 'o', 'core-sw1#']),
  JSON.stringify([1.0, 'o', 'show ver\r\n']),
  'garbage',
  JSON.stringify([31.0, 'o', 'Cisco IOS\r\n']),
  JSON.stringify([31.5, 'r', '100x30']),
  JSON.stringify([32.0, 'i', 'typed']),
  '',
].join('\n');

describe('cast', () => {
  it('parses and limits idle time', () => {
    const c = parseCast(CAST, 2);
    expect(c?.header).toEqual({ width: 120, height: 40, title: 'core-sw1', timestamp: 1793923200 });
    expect(c?.events.map((e) => [e.t, e.kind])).toEqual([
      [0.5, 'o'],
      [1.0, 'o'],
      [3.0, 'o'],
      [3.5, 'r'],
    ]);
    expect(c?.duration).toBe(3.5);
  });

  it('rejects other formats', () => {
    expect(parseCast('{"version":1}')).toBeNull();
    expect(parseCast('not json')).toBeNull();
    expect(parseCast(JSON.stringify({ version: 2 }))?.header.width).toBe(80);
  });

  it('seeks and formats', () => {
    const ev = parseCast(CAST)?.events ?? [];
    expect(indexAt(ev, 0)).toBe(0);
    expect(indexAt(ev, 1.0)).toBe(2);
    expect(indexAt(ev, 99)).toBe(4);
    expect(parseSize('100x30')).toEqual({ cols: 100, rows: 30 });
    expect(parseSize('x')).toBeNull();
    expect(formatClock(75.9)).toBe('1:15');
  });
});
