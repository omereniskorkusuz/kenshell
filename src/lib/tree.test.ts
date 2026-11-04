import { describe, expect, it } from 'vitest';
import type { Folder, SessionRecord } from './ipc';
import { favoriteSessions, searchSessions } from './tree';

const s = (id: number, name: string, extra: Partial<SessionRecord> = {}): SessionRecord => ({
  id,
  folderId: null,
  name,
  protocol: 'ssh',
  host: null,
  port: null,
  username: null,
  serial: null,
  envTag: null,
  tags: [],
  favorite: false,
  ssh: null,
  credentialProfileId: null,
  autoCommands: [],
  sortOrder: id,
  createdAt: 0,
  updatedAt: 0,
  lastConnectedAt: null,
  ...extra,
});
const folders: Folder[] = [{ id: 1, parentId: null, name: 'İstanbul DC', sortOrder: 0 }];
const all = [
  s(1, 'core-sw-01', { host: '10.0.0.1', tags: ['core'], folderId: 1 }),
  s(2, 'access-sw-02', { host: '10.0.1.2', envTag: 'prod', favorite: true }),
  s(3, 'fw-edge', { username: 'netadmin', tags: ['firewall'], favorite: true }),
  s(4, 'console', { protocol: 'serial', serial: null }),
];

describe('session search', () => {
  it('matches name, host, user, tags and folder; all terms must match', () => {
    expect(searchSessions(all, folders, 'sw').map((x) => x.id)).toEqual([2, 1]);
    expect(searchSessions(all, folders, '10.0.1').map((x) => x.id)).toEqual([2]);
    expect(searchSessions(all, folders, 'netadmin').map((x) => x.id)).toEqual([3]);
    expect(searchSessions(all, folders, 'firewall').map((x) => x.id)).toEqual([3]);
    expect(searchSessions(all, folders, 'core 10.0.0').map((x) => x.id)).toEqual([1]);
    expect(searchSessions(all, folders, 'core 10.9')).toEqual([]);
    expect(searchSessions(all, folders, '  ')).toEqual([]);
  });

  it('is case and Turkish-dotted-i insensitive', () => {
    expect(searchSessions(all, folders, 'istanbul').map((x) => x.id)).toEqual([1]);
    expect(searchSessions(all, folders, 'CORE-SW').map((x) => x.id)).toEqual([1]);
  });

  it('name prefix ranks first', () => {
    expect(searchSessions(all, folders, 'co').map((x) => x.id)).toEqual([4, 1]);
  });

  it('favorites list PROD first', () => {
    expect(favoriteSessions(all).map((x) => x.id)).toEqual([2, 3]);
  });
});
