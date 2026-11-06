import { describe, expect, it } from 'vitest';
import {
  chordFromEvent,
  effective,
  normalizeSpec,
  sanitizeOverrides,
  type KeyLike,
} from './shortcuts';

describe('shortcut customization', () => {
  const ev = (key: string, mods: Partial<KeyLike> = {}, code?: string): KeyLike => ({
    key,
    ...(code ? { code } : {}),
    metaKey: false,
    ctrlKey: false,
    shiftKey: false,
    altKey: false,
    ...mods,
  });

  it('records chords from key events', () => {
    expect(chordFromEvent(ev('K', { metaKey: true, shiftKey: true }, 'KeyK'), true)).toBe(
      'mod+shift+k',
    );
    expect(chordFromEvent(ev('k', { ctrlKey: true }, 'KeyK'), false)).toBe('mod+k');
    expect(chordFromEvent(ev('k', { ctrlKey: true }, 'KeyK'), true)).toBe('ctrl+k');
    expect(chordFromEvent(ev('ArrowLeft', { altKey: true }), true)).toBe('alt+arrowleft');
    expect(chordFromEvent(ev('!', { metaKey: true, shiftKey: true }, 'Digit1'), true)).toBe(
      'mod+shift+1',
    );
    expect(chordFromEvent(ev('Meta', { metaKey: true }), true)).toBeNull();
    expect(chordFromEvent(ev('k', {}, 'KeyK'), true)).toBeNull();
    expect(chordFromEvent(ev('k', { shiftKey: true }, 'KeyK'), true)).toBeNull();
  });

  it('normalizes, sanitizes and resolves overrides', () => {
    expect(normalizeSpec('shift+mod+K')).toBe('mod+shift+k');
    expect(
      sanitizeOverrides({
        'tab.close': 'shift+mod+w',
        'session.record': null,
        bad: 'w',
        'x y': 'mod+k',
        'pane.single': 42,
      }),
    ).toEqual({ 'tab.close': 'mod+shift+w', 'session.record': null });
    expect(sanitizeOverrides('nope')).toEqual({});
    const o = { a: 'mod+1', b: null };
    expect(effective('a', 'mod+2', o)).toBe('mod+1');
    expect(effective('b', 'mod+3', o)).toBeUndefined();
    expect(effective('c', 'mod+4', o)).toBe('mod+4');
  });
});
