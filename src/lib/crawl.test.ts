import { describe, expect, it } from 'vitest';
import { crawlable, nextCandidates, type Known } from './crawl';
import type { Discovery, Neighbor } from './ipc';

const nb = (name: string, ip: string | null, capability: string | null = 'Bridge'): Neighbor => ({
  name,
  ip,
  platform: null,
  capability,
  localInterface: 'Gi0/1',
  remoteInterface: null,
  cdp: false,
  lldp: true,
  chassis: null,
  vendor: null,
});
const disc = (neighbors: Neighbor[], gateway: string | null = null): Discovery => ({
  neighbors,
  macs: [],
  arp: [],
  ownMac: null,
  ownVendor: null,
  gateway,
  sources: [],
});

describe('automatic discovery', () => {
  it('only private, ordinary addresses', () => {
    for (const ip of [
      '10.0.100.12',
      '172.16.0.1',
      '172.31.255.254',
      '192.168.1.1',
      '100.64.0.9',
    ]) {
      expect(crawlable(ip)).toBe(true);
    }
    for (const ip of [
      '8.8.8.8',
      '172.32.0.1',
      '127.0.0.1',
      '10.0.0.0',
      '10.0.0.255',
      '1.2.3',
      null,
      '',
      '10.0.0.300',
    ]) {
      expect(crawlable(ip)).toBe(false);
    }
  });

  it('follows reported switches and the gateway, never phones, stations or what was tried', () => {
    const known: Known[] = [
      {
        name: 'core',
        depth: 0,
        discovery: disc(
          [
            nb('dist-1', '10.0.100.16'),
            nb('dist-2', '10.0.100.15', 'Bridge, Router'),
            nb('ap', '10.0.100.30', 'WLAN Access Point'),
            nb('phone', '10.0.100.40', 'Telephone'),
            nb('pc', '10.0.100.41', 'Station'),
            nb('internet', '203.0.113.5', 'Router'),
            nb('no-ip', null),
          ],
          '10.0.100.1',
        ),
      },
      { name: 'not-read-yet', depth: 1, discovery: null },
    ];
    const seen = new Set(['10.0.100.15']);
    const next = nextCandidates(known, seen, 10);
    expect(next.map((c) => c.ip).sort()).toEqual(['10.0.100.1', '10.0.100.16', '10.0.100.30']);
    expect(next.every((c) => c.depth === 1 && c.from === 'core')).toBe(true);
    expect(nextCandidates(known, seen, 2)).toHaveLength(2);
    expect(nextCandidates(known, seen, 0)).toHaveLength(0);
  });

  it('nearest first, one entry per address', () => {
    const known: Known[] = [
      { name: 'far', depth: 3, discovery: disc([nb('x', '10.0.0.9'), nb('y', '10.0.0.8')]) },
      { name: 'near', depth: 0, discovery: disc([nb('y', '10.0.0.8')]) },
    ];
    const next = nextCandidates(known, new Set(), 10);
    expect(next.map((c) => `${c.ip}@${c.depth}`)).toEqual(['10.0.0.8@1', '10.0.0.9@4']);
  });
});
