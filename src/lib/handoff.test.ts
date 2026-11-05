import { describe, expect, it } from 'vitest';
import { decodeHandoff, encodeHandoff } from './handoff';
import type { LiveTab } from './sessionState';

const tab: LiveTab = {
  key: 't3',
  runtimeId: 9,
  generation: 2,
  sessionId: 4,
  name: 'core-sw1',
  envTag: 'prod',
  target: {
    type: 'serial',
    settings: {
      port: 'COM4',
      baud: 9600,
      dataBits: 8,
      parity: 'none',
      stopBits: 1,
      flowControl: 'none',
    },
  } as LiveTab['target'],
  state: 'connected',
  error: null,
  autobaud: null,
  link: null,
  netLink: null,
  prompt: null,
  logPath: null,
  health: null,
  connectedAt: 1000,
  mode: 'config',
  submode: 'config-if',
  device: {
    hostname: 'core-sw1',
    vendor: { id: 'cisco-ios', label: 'Cisco IOS / IOS-XE', family: 'cisco' },
    facts: null,
    deviceId: null,
  },
  reloadAt: null,
};

describe('tab handoff', () => {
  it('round-trips the tab, screen and raw pending bytes', () => {
    const pending = new Uint8Array(70_000).map((_, i) => i % 256);
    const snap = decodeHandoff(encodeHandoff(tab, 'Switch>\r\nSwitch#', pending));
    expect(snap?.screen).toBe('Switch>\r\nSwitch#');
    expect(snap?.pending).toEqual(pending);
    expect(snap?.tab.name).toBe('core-sw1');
    expect(snap?.tab.connectedAt).toBe(1000);
    // Window-local fields stay behind.
    expect(snap?.tab).not.toHaveProperty('key');
    expect(snap?.tab).not.toHaveProperty('runtimeId');
    expect(snap?.tab).not.toHaveProperty('prompt');
  });

  it('rejects foreign or broken payloads', () => {
    expect(decodeHandoff('not json')).toBeNull();
    expect(decodeHandoff('{"v":2}')).toBeNull();
    expect(
      decodeHandoff('{"v":1,"tab":{"name":"x","target":{"type":"ftp"}},"screen":"","pending":""}'),
    ).toBeNull();
  });
});
