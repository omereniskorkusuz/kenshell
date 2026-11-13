import { describe, expect, it } from 'vitest';
import { deviceGuide } from './deviceGuide';

describe('device setup guide', () => {
  it('fills this network in, never a password', () => {
    const g = deviceGuide({ collector: '10.0.100.54', community: 'lab-ro', v3User: 'noc' });
    expect(g.map((f) => f.family)).toEqual(['cisco', 'ruijie', 'aruba', 'alcatel', 'juniper']);
    for (const f of g) {
      expect(f.blocks.map((b) => b.what)).toEqual(['lldp', 'snmp2', 'snmp3', 'flow']);
      const text = f.blocks.flatMap((b) => b.lines).join('\n');
      expect(text).not.toMatch(/\{(collector|community|v3user)\}/);
      expect(text).toContain('10.0.100.54');
      expect(text).toContain('<auth-password>');
    }
    const empty = deviceGuide({ collector: '', community: '', v3User: '' });
    expect(empty[0]?.blocks[1]?.lines[0]).toBe('snmp-server community <community> RO');
  });
});
