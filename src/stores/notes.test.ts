import { describe, expect, it } from 'vitest';
import { ownerKey, pinnedFor } from './notes';

describe('note lookups (P3-T09)', () => {
  const pinned = [
    { sessionId: null, deviceId: 7, text: 'core uplink' },
    { sessionId: 5, deviceId: null, text: 'lab box' },
  ];
  it('finds a session’s own or its device’s pinned note, newest first', () => {
    expect(pinnedFor(pinned, 5, null)).toBe('lab box');
    expect(pinnedFor(pinned, 9, 7)).toBe('core uplink');
    expect(pinnedFor(pinned, 5, 7)).toBe('core uplink');
    expect(pinnedFor(pinned, 9, null)).toBeNull();
  });
  it('owner keys separate session-only and device notes', () => {
    expect(ownerKey(1, null)).not.toBe(ownerKey(null, 1));
  });
});
