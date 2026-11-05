import { describe, expect, it } from 'vitest';
import { draftFrom, draftToInput, validateDraft } from './sessionDraft';

describe('session draft (P2-T12)', () => {
  it('round-trips DTR/RTS line levels on serial sessions', () => {
    const d = draftFrom({
      name: 'console',
      protocol: 'serial',
      serial: {
        port: 'COM4',
        baud: 9600,
        dataBits: 8,
        parity: 'none',
        stopBits: 1,
        flowControl: 'none',
        dtr: false,
        rts: null,
      },
    });
    expect([d.dtr, d.rts]).toEqual(['off', 'default']);
    const input = draftToInput({ ...d, rts: 'on' });
    expect(input.serial).toMatchObject({ dtr: false, rts: true });
  });

  it('validates and converts the SSH keepalive', () => {
    const d = draftFrom({ name: 'core', protocol: 'ssh', host: '10.0.0.1' });
    expect(d.keepalive).toBe('');
    expect(draftToInput(d).ssh?.keepaliveSecs).toBeNull();
    expect(draftToInput({ ...d, keepalive: '0' }).ssh?.keepaliveSecs).toBe(0);
    expect(validateDraft({ ...d, keepalive: '15' }).keepalive).toBeUndefined();
    expect(validateDraft({ ...d, keepalive: '3601' }).keepalive).toBe('range');
    expect(validateDraft({ ...d, keepalive: 'x' }).keepalive).toBe('range');
    // Keepalive belongs to SSH only.
    expect(validateDraft({ ...d, protocol: 'telnet', keepalive: 'x' }).keepalive).toBeUndefined();
  });
});
