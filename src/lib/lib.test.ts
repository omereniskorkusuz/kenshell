import { describe, expect, it, vi } from 'vitest';
import { translate } from '../i18n';
import { autobaudPhase, autobaudStatus, autobaudSteps, formatSteps } from './autobaud';
import { ByteQueue, concatChunks } from './bytes';
import { COMPACT_MAX_HEIGHT, COMPACT_MAX_WIDTH, isCompact } from './compact';
import { describeError, errorCode } from './errors';
import { FrameBatcher, type FrameScheduler } from './frameBatcher';
import { normalizePort, parseQuickConnect } from './quickConnect';

const bytes = (...n: number[]) => new Uint8Array(n);
const tr = (k: Parameters<typeof translate>[1], p?: Parameters<typeof translate>[2]) =>
  translate('en', k, p);

describe('quick connect parser', () => {
  const saved = [{ id: 7, name: 'core-sw-01' }];

  it('parses Windows COM ports with optional baud', () => {
    expect(parseQuickConnect('COM4', saved)).toEqual({ kind: 'serial', port: 'COM4', baud: null });
    expect(parseQuickConnect('com4 9600', saved)).toEqual({
      kind: 'serial',
      port: 'COM4',
      baud: 9600,
    });
    expect(parseQuickConnect('COM04@115200', saved)).toEqual({
      kind: 'serial',
      port: 'COM4',
      baud: 115200,
    });
    expect(parseQuickConnect('COM4 auto', saved)).toEqual({
      kind: 'serial',
      port: 'COM4',
      baud: null,
    });
  });

  it('parses macOS device paths, with and without /dev/', () => {
    expect(parseQuickConnect('/dev/cu.usbserial-XYZ', saved)).toEqual({
      kind: 'serial',
      port: '/dev/cu.usbserial-XYZ',
      baud: null,
    });
    expect(parseQuickConnect('cu.usbserial-XYZ 115200', saved)).toEqual({
      kind: 'serial',
      port: '/dev/cu.usbserial-XYZ',
      baud: 115200,
    });
    expect(normalizePort('ttyUSB0')).toBe('/dev/ttyUSB0');
    expect(normalizePort('router1')).toBeNull();
  });

  it('rejects out-of-range baud rates without throwing', () => {
    expect(parseQuickConnect('COM4 12', saved)).toEqual({
      kind: 'invalidBaud',
      port: 'COM4',
      value: '12',
    });
    expect(parseQuickConnect('COM4 9999999', saved).kind).toBe('invalidBaud');
  });

  it('prefers an exact saved session name', () => {
    expect(parseQuickConnect('  Core-SW-01 ', saved)).toEqual({
      kind: 'session',
      session: saved[0],
    });
  });

  it('parses network targets', () => {
    expect(parseQuickConnect('ssh admin@10.0.0.1', saved)).toEqual({
      kind: 'network',
      protocol: 'ssh',
      host: '10.0.0.1',
      port: null,
      username: 'admin',
    });
    expect(parseQuickConnect('ssh admin@10.0.0.1:2222', saved)).toMatchObject({ port: 2222 });
    expect(parseQuickConnect('telnet 10.0.0.5 2001', saved)).toEqual({
      kind: 'network',
      protocol: 'telnet',
      host: '10.0.0.5',
      port: 2001,
      username: null,
    });
    expect(parseQuickConnect('10.0.0.5 2001', saved)).toMatchObject({
      protocol: 'telnet',
      port: 2001,
    });
    expect(parseQuickConnect('10.0.0.1', saved)).toMatchObject({
      protocol: 'ssh',
      host: '10.0.0.1',
    });
    expect(parseQuickConnect('ssh://root@[fe80::1]:22', saved)).toMatchObject({
      protocol: 'ssh',
      host: 'fe80::1',
      port: 22,
      username: 'root',
    });
    expect(parseQuickConnect('raw 10.0.0.9 4001', saved)).toMatchObject({
      protocol: 'raw',
      port: 4001,
    });
    expect(parseQuickConnect('raw 10.0.0.9', saved)).toEqual({ kind: 'invalidPort', value: '' });
    expect(parseQuickConnect('ssh 10.0.0.1:99999', saved)).toEqual({
      kind: 'invalidPort',
      value: '99999',
    });
    expect(parseQuickConnect('', saved)).toEqual({ kind: 'empty' });
    expect(parseQuickConnect('hello world foo', saved).kind).toBe('noMatch');
  });
});

describe('hidden tab byte queue', () => {
  it('keeps everything under the cap', () => {
    const q = new ByteQueue(10);
    q.push(bytes(1, 2, 3));
    q.push(bytes(4, 5));
    expect(q.size).toBe(5);
    const { bytes: out, dropped } = q.drain();
    expect([...out]).toEqual([1, 2, 3, 4, 5]);
    expect(dropped).toBe(0);
    expect(q.size).toBe(0);
  });

  it('drops the oldest bytes when the cap is exceeded', () => {
    const q = new ByteQueue(5);
    q.push(bytes(1, 2, 3));
    q.push(bytes(4, 5, 6, 7));
    const { bytes: out, dropped } = q.drain();
    expect([...out]).toEqual([3, 4, 5, 6, 7]);
    expect(dropped).toBe(2);
  });

  it('a single chunk larger than the cap keeps only its tail', () => {
    const q = new ByteQueue(3);
    q.push(bytes(9));
    q.push(bytes(1, 2, 3, 4, 5));
    const { bytes: out, dropped } = q.drain();
    expect([...out]).toEqual([3, 4, 5]);
    expect(dropped).toBe(3);
  });

  it('concatenates chunks', () => {
    expect([...concatChunks([bytes(1), bytes(), bytes(2, 3)])]).toEqual([1, 2, 3]);
  });
});

describe('frame batcher', () => {
  function fakeScheduler() {
    const frames = new Map<number, () => void>();
    const timers = new Map<number, () => void>();
    let id = 0;
    const s: FrameScheduler = {
      requestFrame: (cb) => (frames.set(++id, cb), id),
      cancelFrame: (i) => void frames.delete(i),
      setTimer: (cb) => (timers.set(++id, cb), id),
      clearTimer: (i) => void timers.delete(i),
    };
    const fire = (m: Map<number, () => void>) => [...m.values()].forEach((cb) => cb());
    return { s, frame: () => fire(frames), timer: () => fire(timers), frames, timers };
  }

  it('writes all chunks of a frame in one call', () => {
    const f = fakeScheduler();
    const sink = vi.fn();
    const b = new FrameBatcher(sink, f.s, 50);
    b.push(bytes(1));
    b.push(bytes(2, 3));
    expect(sink).not.toHaveBeenCalled();
    f.frame();
    expect(sink).toHaveBeenCalledTimes(1);
    expect([...(sink.mock.calls[0]?.[0] as Uint8Array)]).toEqual([1, 2, 3]);
    expect(f.timers.size).toBe(0);
  });

  it('falls back to a timer when no animation frame comes (hidden window)', () => {
    const f = fakeScheduler();
    const sink = vi.fn();
    const b = new FrameBatcher(sink, f.s, 50);
    b.push(bytes(7));
    f.timer();
    expect(sink).toHaveBeenCalledTimes(1);
    expect(f.frames.size).toBe(0);
  });

  it('take() moves pending bytes out without writing', () => {
    const f = fakeScheduler();
    const sink = vi.fn();
    const b = new FrameBatcher(sink, f.s);
    b.push(bytes(1, 2));
    expect([...(b.take() ?? [])]).toEqual([1, 2]);
    f.frame();
    f.timer();
    expect(sink).not.toHaveBeenCalled();
  });
});

describe('compact identity bar', () => {
  it('is compact when the pane is short or narrow', () => {
    expect(isCompact({ width: 1400, height: 900 })).toBe(false);
    expect(isCompact({ width: 1400, height: COMPACT_MAX_HEIGHT - 1 })).toBe(true);
    expect(isCompact({ width: COMPACT_MAX_WIDTH - 1, height: 900 })).toBe(true);
  });
});

describe('autobaud status text', () => {
  const trial = (baud: number) => ({ baud, score: 0.2, promptMatch: false, bytes: 10 });

  it('shows failed and accepted steps', () => {
    const p = {
      trials: [trial(9600), trial(115200)],
      current: null,
      result: 115200,
      suggestion: null,
    };
    expect(formatSteps(autobaudSteps(p))).toBe('9600 ✗ 115200 ✓');
    expect(autobaudPhase(p)).toBe('found');
    expect(autobaudStatus(p, tr).text).toContain('115200 ✓');
  });

  it('shows the candidate in progress', () => {
    const p = { trials: [trial(9600)], current: 115200, result: null, suggestion: null };
    expect(formatSteps(autobaudSteps(p))).toBe('9600 ✗ 115200…');
    expect(autobaudPhase(p)).toBe('running');
  });

  it('offers a suggestion when nothing was accepted', () => {
    const p = { trials: [trial(9600)], current: null, result: null, suggestion: 38400 };
    const s = autobaudStatus(p, tr);
    expect(s.phase).toBe('suggestion');
    expect(s.suggestion).toBe(38400);
  });
});

describe('error messages', () => {
  it('maps codes to translated messages and never uses raw OS text as the message', () => {
    const e = { code: 'portBusy', detail: 'COM4: Access is denied. (os error 5)' };
    const d = describeError(e, tr, { port: 'COM4' });
    expect(d.code).toBe('portBusy');
    expect(d.message).toContain('COM4');
    expect(d.message).not.toContain('os error');
    expect(d.detail).toContain('os error');
  });

  it('unknown values become internal errors', () => {
    expect(errorCode(new Error('x'))).toBe('internal');
    expect(errorCode({ code: 'bogus' })).toBe('internal');
  });
});

describe('uptime at minute resolution', () => {
  it('formats and schedules the next change', async () => {
    const { formatUptimeShort, msToNextMinute } = await import('./format');
    expect(formatUptimeShort(3_725_000)).toBe('01:02');
    expect(formatUptimeShort(59_999)).toBe('00:00');
    expect(msToNextMinute(0)).toBe(60_000);
    expect(msToNextMinute(61_500)).toBe(58_500);
  });
});
