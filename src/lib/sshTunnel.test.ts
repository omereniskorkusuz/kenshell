import { describe, expect, it } from 'vitest';
import { draftFrom, draftToInput, validateDraft } from './sessionDraft';
import {
  formatForwards,
  formatJump,
  MAX_FORWARDS,
  parseForwards,
  parseHostPort,
  parseJump,
} from './sshTunnel';

describe('parseHostPort', () => {
  it('accepts names, IPv4 and bracketed IPv6', () => {
    expect(parseHostPort('bastion:2222', true)).toEqual({ host: 'bastion', port: 2222 });
    expect(parseHostPort('10.0.0.1', false)).toEqual({ host: '10.0.0.1', port: null });
    expect(parseHostPort('[fe80::1]:22', true)).toEqual({ host: 'fe80::1', port: 22 });
    expect(parseHostPort('fe80::1', false)).toEqual({ host: 'fe80::1', port: null });
  });
  it('rejects bad input', () => {
    for (const bad of ['', 'a b', 'host:', 'host:0', 'host:70000', ':22', '[]:22', '[::1]x']) {
      expect(parseHostPort(bad, false)).toBeNull();
    }
    expect(parseHostPort('host', true)).toBeNull();
  });
});

describe('jump host', () => {
  it('parses and formats round trip', () => {
    expect(parseJump('')).toBeNull();
    expect(parseJump('admin@bastion:2222')).toEqual({
      host: 'bastion',
      port: 2222,
      username: 'admin',
    });
    expect(parseJump('bastion')).toEqual({ host: 'bastion', port: null, username: null });
    expect(parseJump('@bastion')).toBe('invalid');
    expect(parseJump('a@b:x')).toBe('invalid');
    for (const text of ['admin@bastion:2222', 'bastion', 'ops@[2001:db8::1]:22']) {
      const j = parseJump(text);
      expect(j).not.toBe('invalid');
      expect(formatJump(j === 'invalid' ? null : j)).toBe(text);
    }
  });
});

describe('forwards', () => {
  it('parses the three kinds', () => {
    expect(parseForwards('L 8080:10.0.0.10:80\n\n d 1080 \nR 9000:[::1]:22')).toEqual([
      { kind: 'local', listenPort: 8080, targetHost: '10.0.0.10', targetPort: 80 },
      { kind: 'dynamic', listenPort: 1080, targetHost: null, targetPort: null },
      { kind: 'remote', listenPort: 9000, targetHost: '::1', targetPort: 22 },
    ]);
  });
  it('reports bad lines (1-based)', () => {
    expect(parseForwards('L 8080:host\nD 1080\nX 1:a:2\nD 0')).toEqual({ badLines: [1, 3, 4] });
  });
  it('limits the count', () => {
    const many = Array.from({ length: MAX_FORWARDS + 1 }, (_, i) => `D ${1000 + i}`).join('\n');
    expect(parseForwards(many)).toEqual({ badLines: [] });
  });
  it('formats back to the same text', () => {
    const text = 'L 8080:10.0.0.10:80\nD 1080\nR 9000:[::1]:22';
    const parsed = parseForwards(text);
    expect(Array.isArray(parsed) && formatForwards(parsed)).toBe(text);
  });
});

describe('session draft with tunnels', () => {
  const base = draftFrom({ protocol: 'ssh' });
  const draft = { ...base, name: 'core', host: '10.1.1.1' };

  it('builds ssh options', () => {
    const input = draftToInput({
      ...draft,
      jump: 'admin@bastion',
      proxyKind: 'socks5',
      proxy: '127.0.0.1:1080',
      forwards: 'D 1081',
    });
    expect(input.ssh).toMatchObject({
      jump: { host: 'bastion', port: null, username: 'admin' },
      proxy: { kind: 'socks5', host: '127.0.0.1', port: 1080 },
      forwards: [{ kind: 'dynamic', listenPort: 1081 }],
    });
    const back = draftFrom(input);
    expect([back.jump, back.proxyKind, back.proxy, back.forwards]).toEqual([
      'admin@bastion',
      'socks5',
      '127.0.0.1:1080',
      'D 1081',
    ]);
  });
  it('validates only for ssh', () => {
    const bad = { ...draft, jump: '@x', proxyKind: 'http' as const, proxy: 'x', forwards: 'L 1' };
    expect(validateDraft(bad)).toEqual({ jump: 'format', proxy: 'format', forwards: 'format' });
    expect(validateDraft({ ...bad, protocol: 'telnet' })).toEqual({});
  });
  it('drops the proxy address when the type is none', () => {
    const input = draftToInput({ ...draft, proxyKind: 'none', proxy: 'garbage' });
    expect(input.ssh?.proxy).toBeNull();
    expect(validateDraft({ ...draft, proxyKind: 'none', proxy: 'garbage' })).toEqual({});
  });
});
