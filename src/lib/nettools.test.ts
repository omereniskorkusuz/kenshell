import { describe, expect, it } from 'vitest';
import { macFormats, maskToPrefix, prefixToMask, subnet } from './nettools';

describe('subnet', () => {
  it('computes a /24 in every input spelling', () => {
    for (const input of [
      '192.168.10.77/24',
      '192.168.10.77 255.255.255.0',
      '192.168.10.77/255.255.255.0',
      '192.168.10.77 0.0.0.255',
    ]) {
      const s = subnet(input);
      expect(s, input).toMatchObject({
        prefix: 24,
        mask: '255.255.255.0',
        wildcard: '0.0.0.255',
        network: '192.168.10.0',
        broadcast: '192.168.10.255',
        firstHost: '192.168.10.1',
        lastHost: '192.168.10.254',
        hosts: 254,
        total: 256,
        scope: 'private',
      });
    }
  });

  it('handles odd prefixes and the edges', () => {
    expect(subnet('10.20.30.40/19')).toMatchObject({
      network: '10.20.0.0',
      broadcast: '10.20.31.255',
      mask: '255.255.224.0',
      hosts: 8190,
    });
    expect(subnet('10.0.0.1/31')).toMatchObject({
      firstHost: '10.0.0.0',
      lastHost: '10.0.0.1',
      hosts: 2,
    });
    expect(subnet('8.8.8.8')).toMatchObject({ prefix: 32, hosts: 1, scope: 'public' });
    expect(subnet('1.2.3.4/0')).toMatchObject({
      network: '0.0.0.0',
      broadcast: '255.255.255.255',
      total: 2 ** 32,
    });
    expect(subnet('100.64.1.1/10')?.scope).toBe('cgnat');
    expect(subnet('169.254.3.3/16')?.scope).toBe('linkLocal');
    expect(subnet('10.0.0.0/22')?.binaryMask).toBe('11111111.11111111.11111100.00000000');
  });

  it('rejects junk', () => {
    for (const bad of [
      '',
      '10.0.0',
      '10.0.0.256/24',
      '10.0.0.1/33',
      '10.0.0.1 255.0.255.0',
      'a/b',
      '1.1.1.1 2 3',
    ])
      expect(subnet(bad), bad).toBeNull();
  });

  it('converts masks', () => {
    expect(maskToPrefix('255.255.255.252')).toBe(30);
    expect(maskToPrefix('0.0.0.0')).toBe(0);
    expect(maskToPrefix('255.255.0.255')).toBeNull();
    expect(prefixToMask(20)).toBe('255.255.240.0');
    expect(prefixToMask(0)).toBe('0.0.0.0');
  });
});

describe('macFormats', () => {
  it('converts between Cisco, colon and dash forms', () => {
    for (const input of [
      'AABB.CCDD.EEFF',
      'aa:bb:cc:dd:ee:ff',
      'AA-BB-CC-DD-EE-FF',
      'aabbccddeeff',
    ]) {
      expect(macFormats(input), input).toMatchObject({
        cisco: 'aabb.ccdd.eeff',
        colon: 'aa:bb:cc:dd:ee:ff',
        dash: 'AA-BB-CC-DD-EE-FF',
        bare: 'aabbccddeeff',
      });
    }
  });

  it('reads the I/G and U/L bits', () => {
    expect(macFormats('0100.5e00.0001')).toMatchObject({ multicast: true, local: false });
    expect(macFormats('02:42:ac:11:00:02')).toMatchObject({ multicast: false, local: true });
    expect(macFormats('00:1b:54:aa:bb:cc')).toMatchObject({ multicast: false, local: false });
  });

  it('rejects junk', () => {
    for (const bad of ['', 'aa:bb:cc', 'gg:bb:cc:dd:ee:ff', 'aabb.ccdd.eeff.0011'])
      expect(macFormats(bad)).toBeNull();
  });
});
