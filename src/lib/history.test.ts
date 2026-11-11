import { describe, expect, it } from 'vitest';
import { addHistory, historyKey, recentItems, removeHistory, sanitizeHistory } from './history';
import { DEFAULT_SERIAL, DEFAULT_SSH, type OpenTarget } from './ipc';

const net = (host: string, username: string | null = null): OpenTarget => ({
  type: 'network',
  protocol: 'ssh',
  host,
  port: 22,
  username,
  ssh: DEFAULT_SSH,
});
const serial: OpenTarget = {
  type: 'serial',
  settings: { ...DEFAULT_SERIAL, port: '/dev/cu.usb1' },
};
const entry = (target: OpenTarget, at: number, sessionId: number | null = null) => ({
  name: 'x',
  envTag: null,
  sessionId,
  target,
  at,
});

describe('connection history', () => {
  it('keeps one entry per endpoint, newest first, capped', () => {
    let h = addHistory([], entry(net('10.0.0.1', 'admin'), 1));
    h = addHistory(h, entry(serial, 2));
    h = addHistory(h, entry(net('10.0.0.1', 'admin'), 3));
    expect(h.map((e) => e.at)).toEqual([3, 2]);
    expect(historyKey(net('SW1'), null)).toBe(historyKey(net('sw1'), null));
    expect(historyKey(net('sw1', 'a'), null)).not.toBe(historyKey(net('sw1', 'b'), null));
    expect(historyKey(net('sw1'), 7)).toBe('s:7');
    for (let i = 0; i < 50; i++) h = addHistory(h, entry(net(`h${i}`), 10 + i), 5);
    expect(h).toHaveLength(5);
    expect(removeHistory(h, (h[0] as { key: string }).key)).toHaveLength(4);
  });

  it('drops malformed stored values', () => {
    const good = addHistory([], entry(net('10.0.0.1'), 1));
    expect(sanitizeHistory(good)).toEqual(good);
    expect(sanitizeHistory('x')).toEqual([]);
    expect(
      sanitizeHistory([
        null,
        {
          name: 'a',
          at: 1,
          target: { type: 'network', protocol: 'ftp', host: 'h', port: 1, username: null, ssh: {} },
        },
        { name: 'b', at: 'now', target: net('h') },
        { name: 'c', at: 2, target: { type: 'serial', settings: { port: '' } } },
        ...good,
        ...good,
      ]),
    ).toEqual(good);
  });

  it('recent rows use the saved session while it exists', () => {
    const h = [
      ...addHistory([], entry(net('10.0.0.9'), 5, 3)),
      ...addHistory([], entry(net('10.0.0.1'), 9)),
      ...addHistory([], entry(net('10.0.0.2'), 1, 99)),
    ];
    const sessions = [{ id: 3, name: 'core', envTag: 'prod', lastConnectedAt: 5 }];
    const r = recentItems(h, sessions);
    expect(r.map((x) => x.name)).toEqual(['x', 'core', 'x']);
    expect(r[1]?.session).toBe(sessions[0]);
    expect(r[1]?.envTag).toBe('prod');
    expect(r[2]?.session).toBeNull();
    expect(recentItems(h, sessions, 1)).toHaveLength(1);
  });
});
