import { describe, expect, it } from 'vitest';
import {
  fillParts,
  fillSnippet,
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

describe('variable templates (P3-T16)', () => {
  it('fills every use of a variable and keeps the rest', () => {
    const body = 'interface {{interface}}\n description {{ description }}\n! {{interface}}';
    expect(fillSnippet(body, { interface: 'Gi1/0/24', description: 'Kamera' })).toBe(
      'interface Gi1/0/24\n description Kamera\n! Gi1/0/24',
    );
    expect(fillSnippet('vlan {{vlan_id}}', { vlan_id: '10\n20' })).toBe('vlan 10 20');
    expect(fillParts('a {{x}} b', {})).toEqual([
      { text: 'a ' },
      { name: 'x', value: '' },
      { text: ' b' },
    ]);
  });
});
