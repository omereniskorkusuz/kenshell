import { describe, expect, it } from 'vitest';
import { applyPatch, parseTagList, rangeBetween } from './bulk';
import type { SessionInput } from './ipc';

const base = {
  folderId: null,
  name: 'core-sw1',
  protocol: 'ssh',
  host: '10.0.0.1',
  port: 22,
  username: 'admin',
  serial: null,
  envTag: 'lab',
  tags: ['core', 'dc1'],
  favorite: true,
  ssh: null,
  credentialProfileId: null,
  autoCommands: [],
} as unknown as SessionInput;

describe('bulk edit', () => {
  it('changes only the given fields', () => {
    const out = applyPatch(base, {
      folder: { value: 3 },
      envTag: { value: null },
      addTags: ['edge', 'CORE'],
      removeTags: ['DC1'],
      username: { value: 'netops' },
    });
    expect(out).toMatchObject({
      folderId: 3,
      envTag: null,
      tags: ['core', 'edge'],
      favorite: true,
      username: 'netops',
      host: '10.0.0.1',
    });
    expect(base.tags).toEqual(['core', 'dc1']);
  });

  it('never gives serial sessions a user name', () => {
    const serial = { ...base, protocol: 'serial', username: null } as SessionInput;
    expect(applyPatch(serial, { username: { value: 'x' } }).username).toBeNull();
  });

  it('parses tag lists and ranges', () => {
    expect(parseTagList(' core, edge  Core,, dc1 ')).toEqual(['core', 'edge', 'dc1']);
    expect(rangeBetween([5, 2, 9, 4], 2, 4)).toEqual([2, 9, 4]);
    expect(rangeBetween([5, 2, 9, 4], 4, 5)).toEqual([5, 2, 9, 4]);
    expect(rangeBetween([5, 2], 7, 2)).toEqual([2]);
  });
});
