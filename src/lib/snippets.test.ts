import { describe, expect, it } from 'vitest';
import {
  filterSnippets,
  groupSnippets,
  snippetPayload,
  snippetVariables,
  STARTER,
  toItem,
} from './snippets';

const mine = toItem({
  id: 1,
  folder: 'Ops',
  name: 'Save',
  body: 'write memory',
  vendor: null,
  createdAt: 0,
  updatedAt: 0,
});

describe('snippet library (P3-T15)', () => {
  it('starter sets cover Cisco and Ruijie basics', () => {
    for (const vendor of ['cisco', 'ruijie']) {
      const names = STARTER.filter((s) => s.vendor === vendor).map((s) => s.name);
      expect(names).toEqual([
        'Access port',
        'Trunk port',
        'Create VLAN',
        'Enable SSH',
        'NTP and syslog',
      ]);
    }
    expect(STARTER.every((s) => s.builtin && s.id === null)).toBe(true);
  });

  it('vendor filter keeps neutral snippets; search looks at body too', () => {
    const all = [mine, ...STARTER];
    expect(filterSnippets(all, 'ruijie', '').every((s) => s.vendor !== 'cisco')).toBe(true);
    expect(filterSnippets(all, 'ruijie', '').some((s) => s.id === 1)).toBe(true);
    expect(filterSnippets(all, 'all', 'allowed vlan only').map((s) => s.key)).toEqual([
      'b:ruijie-trunk-port',
    ]);
    expect(groupSnippets(all).map(([f]) => f)).toEqual(['Ops', 'Cisco', 'Ruijie']);
  });

  it('variables and payload', () => {
    expect(snippetVariables('int {{ interface }}\n vlan {{vlan_id}} {{interface}}')).toEqual([
      'interface',
      'vlan_id',
    ]);
    expect(snippetPayload('conf t\r\nvlan 10\n\n')).toBe('conf t\rvlan 10\r');
  });
});
