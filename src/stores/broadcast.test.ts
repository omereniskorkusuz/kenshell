import { describe, expect, it, vi } from 'vitest';
import type { LiveTab } from '../lib/sessionState';
import { useTabs } from './tabs';

const tab = (key: string, extra: Partial<LiveTab> = {}) =>
  ({ key, state: 'connected', prompt: null, ...extra }) as unknown as LiveTab;

describe('broadcast (P3-T13)', () => {
  it('input goes to the connected tabs of the group, other tabs stay alone', () => {
    const write = vi.fn();
    const paste = vi.fn();
    useTabs.setState({
      tabs: [
        tab('a'),
        tab('b'),
        tab('c'),
        tab('d', { state: 'disconnected' }),
        tab('e', { prompt: { kind: 'password' } as unknown as LiveTab['prompt'] }),
      ],
      broadcast: [],
      write,
      paste,
    });
    const st = () => useTabs.getState();
    st().input('a', 'x');
    expect(write.mock.calls).toEqual([['a', 'x']]);

    write.mockClear();
    st().toggleBroadcast('a');
    st().toggleBroadcast('b');
    st().toggleBroadcast('d');
    st().toggleBroadcast('e');
    st().input('a', 'show clock\r');
    expect(write.mock.calls.map((c) => c[0])).toEqual(['a', 'b']);
    st().pasteInput('b', 'conf t');
    expect(paste.mock.calls.map((c) => c[0])).toEqual(['b', 'a']);

    write.mockClear();
    st().input('c', 'y');
    expect(write.mock.calls.map((c) => c[0])).toEqual(['c']);

    st().toggleBroadcast('b');
    expect(st().broadcast).toEqual(['a', 'd', 'e']);
    st().broadcastAll();
    expect(st().broadcast).toEqual(['a', 'b', 'c', 'd', 'e']);
    st().stopBroadcast();
    expect(st().broadcast).toEqual([]);
  });
});
