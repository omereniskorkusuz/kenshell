import { describe, expect, it } from 'vitest';
import { LineTracker, storable, suggest } from './autocomplete';

describe('line tracking (P3-T12)', () => {
  it('follows typing, backspace and Enter', () => {
    const t = new LineTracker();
    expect(t.feed('show ver')).toEqual([]);
    expect(t.feed('\x7f\x7f\x7fversion')).toEqual([]);
    expect(t.line).toBe('show version');
    expect(t.feed('\r')).toEqual(['show version']);
    expect(t.line).toBe('');
    expect(t.feed('conf t\rint gi0/1\r')).toEqual(['conf t', 'int gi0/1']);
  });

  it('cursor keys, Tab and pastes make the line unknown until Enter', () => {
    const t = new LineTracker();
    t.feed('sh\x1b[Aip');
    expect(t.line).toBeNull();
    expect(t.feed('\r')).toEqual([]);
    t.feed('sh ip int\t');
    expect(t.line).toBeNull();
    t.feed('\r');
    t.feed('abc\x15show');
    expect(t.line).toBe('show');
    t.lose();
    expect(t.feed('\r')).toEqual([]);
  });

  it('secrets and control characters are not stored', () => {
    expect(storable('show run')).toBe(true);
    expect(storable('username admin secret 0 x')).toBe(false);
    expect(storable('snmp-server community public ro')).toBe(false);
    expect(storable('  ')).toBe(false);
  });
});

describe('suggestions', () => {
  it('history first (case-insensitive), then the vendor built-ins', () => {
    const history = ['show ip route vrf MGMT', 'show interfaces status'];
    expect(suggest('sh', history, 'cisco')).toBe('ow ip route vrf MGMT');
    expect(suggest('SHOW INT', history, 'cisco')).toBe('erfaces status');
    expect(suggest('show ip int', [], null)).toBe('erface brief');
    expect(suggest('disp', [], 'huawei')).toBe('lay current-configuration');
    expect(suggest('show mac', [], 'ruijie')).toBe('-address-table');
    expect(suggest('s', history, null)).toBeNull();
    expect(suggest('show interfaces status', history, null)).toBeNull();
    expect(suggest('zzz', history, null)).toBeNull();
  });
});
