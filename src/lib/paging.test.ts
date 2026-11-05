import { describe, expect, it } from 'vitest';
import { pagingDisableCommand } from './paging';

describe('vendor paging profile (P3-T14)', () => {
  it('uses the vendor command, Cisco-like by default', () => {
    expect(pagingDisableCommand(null)).toBe('terminal length 0');
    expect(pagingDisableCommand('huawei-vrp')).toMatch(/^screen-length 0/);
    expect(pagingDisableCommand('juniper-junos')).toMatch(/screen-length 0/);
    expect(pagingDisableCommand('nope')).toBe('terminal length 0');
    expect(pagingDisableCommand('fortinet-fortios')).toBeNull();
  });
});
