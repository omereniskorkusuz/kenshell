import { describe, expect, it } from 'vitest';
import { applySessionEvent, NO_DEVICE, type LiveTab } from './sessionState';

const tab: LiveTab = {
  key: 't1',
  runtimeId: 1,
  generation: 1,
  sessionId: null,
  name: '10.0.0.1',
  envTag: null,
  target: {
    type: 'network',
    protocol: 'telnet',
    host: '10.0.0.1',
    port: 23,
    username: null,
    ssh: null,
  } as unknown as LiveTab['target'],
  state: 'connected',
  error: null,
  autobaud: null,
  link: null,
  netLink: null,
  prompt: null,
  logPath: null,
  health: null,
  connectedAt: 1,
  mode: 'unknown',
  submode: null,
  device: NO_DEVICE,
  reloadAt: null,
};

describe('device events (P3-T03)', () => {
  it('mode follows the prompt and identity accumulates', () => {
    let t = applySessionEvent(tab, { type: 'mode', mode: 'config', sub: 'config-if' }, 5);
    expect([t.mode, t.submode]).toEqual(['config', 'config-if']);
    t = applySessionEvent(
      t,
      { type: 'identity', hostname: 'core-sw1', vendor: null, facts: null, deviceId: null },
      6,
    );
    t = applySessionEvent(
      t,
      {
        type: 'identity',
        hostname: null,
        vendor: { id: 'ruijie-rgos', label: 'Ruijie RGOS', family: 'ruijie' },
        facts: null,
        deviceId: null,
      },
      7,
    );
    const facts = {
      hostname: 'core-sw1',
      model: 'S5750',
      version: '11.4',
      serial: 'G1',
      uptime: null,
    };
    t = applySessionEvent(
      t,
      { type: 'identity', hostname: null, vendor: null, facts, deviceId: 4 },
      8,
    );
    expect(t.device).toEqual({
      hostname: 'core-sw1',
      vendor: { id: 'ruijie-rgos', label: 'Ruijie RGOS', family: 'ruijie' },
      facts,
      deviceId: 4,
    });
  });

  it('reload shows until the next prompt; a drop clears the mode but keeps the device', () => {
    let t = applySessionEvent(tab, { type: 'mode', mode: 'privileged', sub: null }, 1);
    t = applySessionEvent(t, { type: 'reload' }, 100);
    expect([t.mode, t.reloadAt]).toEqual(['unknown', 100]);
    t = applySessionEvent(t, { type: 'reload' }, 200);
    expect(t.reloadAt).toBe(100);
    t = applySessionEvent(t, { type: 'mode', mode: 'user', sub: null }, 300);
    expect([t.mode, t.reloadAt]).toEqual(['user', null]);
    t = applySessionEvent(
      t,
      { type: 'identity', hostname: 'sw', vendor: null, facts: null, deviceId: null },
      301,
    );
    t = applySessionEvent(t, { type: 'state', state: 'disconnected', error: null }, 400);
    expect([t.mode, t.device.hostname]).toEqual(['unknown', 'sw']);
  });
});
