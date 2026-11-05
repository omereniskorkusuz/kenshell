import { describe, expect, it } from 'vitest';
import { compileRules, highlightLine, type Span } from './highlight';

const words = (text: string, spans: Span[]) =>
  spans.map((s) => `${s.style}:${text.slice(s.start, s.end)}`);
const run = (text: string, family: string | null = null, off: string[] = []) =>
  words(text, highlightLine(text, compileRules(family, off)));

describe('output colouring rules (P3-T10)', () => {
  it('Cisco interface status', () => {
    const l = 'GigabitEthernet1/0/24 is up, line protocol is down (notconnect)';
    expect(run(l)).toEqual([
      'iface:GigabitEthernet1/0/24',
      'up:up',
      'down:down',
      'down:notconnect',
    ]);
    expect(run('Gi1/0/3   uplink   err-disabled 10   auto')).toEqual([
      'iface:Gi1/0/3',
      'warning:err-disabled',
    ]);
    expect(run('Fa0/1 is administratively down, line protocol is down')).toEqual([
      'iface:Fa0/1',
      'down:administratively down',
      'down:down',
    ]);
  });

  it('Ruijie spaced interface names and errors', () => {
    expect(run('GigabitEthernet 0/24   UP   1   1000M', 'ruijie')).toEqual([
      'iface:GigabitEthernet 0/24',
      'up:UP',
    ]);
    expect(run("% Invalid input detected at '^' marker.")).toEqual([
      "error:% Invalid input detected at '^' marker.",
    ]);
    expect(run('% Unknown command.', 'ruijie')).toEqual(['error:% Unknown command.']);
  });

  it('IPs (with prefix) and MACs, but not version numbers or longer dotted strings', () => {
    expect(run('Internet address is 10.1.20.1/24')).toEqual(['ip:10.1.20.1/24']);
    expect(run('Internet  192.168.1.10   4   0011.2233.4455  ARPA   Vlan10')).toEqual([
      'ip:192.168.1.10',
      'mac:0011.2233.4455',
      'iface:Vlan10',
    ]);
    expect(run('mac 00:1a:2b:3c:4d:5e, 00-1A-2B-3C-4D-5E')).toEqual([
      'mac:00:1a:2b:3c:4d:5e',
      'mac:00-1A-2B-3C-4D-5E',
    ]);
    expect(run('Version 15.2(7)E8, OID 1.3.6.1.4.1.9, 999.1.1.1')).toEqual([]);
  });

  it('vendor-specific rules apply only to their family', () => {
    const huawei = "Error: Unrecognized command found at '^' position.";
    expect(run(huawei, 'huawei')).toEqual([`error:${huawei}`]);
    expect(run(huawei, 'cisco')).toEqual([]);
    expect(run('GE0/0/1   *down   down', 'huawei')).toEqual([
      'iface:GE0/0/1',
      'down:*down',
      'down:down',
    ]);
    expect(run('Command fail. Return code -61', 'fortinet')).toEqual([
      'error:Command fail. Return code -61',
    ]);
  });

  it('categories can be switched off; words inside other words are not coloured', () => {
    const l = 'Gi0/1 is up 10.0.0.1';
    expect(run(l, null, ['status', 'interfaces'])).toEqual(['ip:10.0.0.1']);
    expect(run(l, null, ['status', 'errors', 'addresses', 'interfaces'])).toEqual([]);
    expect(run('setup backup download updated')).toEqual([]);
  });
});
