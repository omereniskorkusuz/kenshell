import { describe, expect, it } from 'vitest';
import { pingReply, pingStats, traceHops } from './diag';

const MAC_PING = `PING 127.0.0.1 (127.0.0.1): 56 data bytes
64 bytes from 127.0.0.1: icmp_seq=0 ttl=64 time=0.054 ms
Request timeout for icmp_seq 1
64 bytes from 127.0.0.1: icmp_seq=2 ttl=64 time=0.067 ms

--- 127.0.0.1 ping statistics ---
3 packets transmitted, 2 packets received, 33.3% packet loss
round-trip min/avg/max/stddev = 0.054/0.060/0.067/0.012 ms`.split('\n');

describe('ping', () => {
  it('reads replies of every OS and language', () => {
    expect(pingReply('64 bytes from 10.0.0.1: icmp_seq=1 ttl=64 time=12.3 ms')).toBe(12.3);
    expect(pingReply('Reply from 10.0.0.1: bytes=32 time=4ms TTL=255')).toBe(4);
    expect(pingReply('Reply from 10.0.0.1: bytes=32 time<1ms TTL=64')).toBe(1);
    expect(pingReply('10.0.0.1 yanıtı: bayt=32 süre=7ms TTL=64')).toBe(7);
    expect(pingReply('Request timed out.')).toBeNull();
    expect(pingReply('round-trip min/avg/max/stddev = 0.054/0.060/0.067/0.012 ms')).toBeNull();
  });

  it('summarises a run', () => {
    expect(pingStats(MAC_PING)).toMatchObject({ replies: 2, min: 0.054, max: 0.067, last: 0.067 });
    expect(pingStats(['PING x'])).toBeNull();
  });
});

describe('traceHops', () => {
  it('parses macOS / Linux traceroute', () => {
    const hops = traceHops(
      `traceroute to 8.8.8.8 (8.8.8.8), 30 hops max, 60 byte packets
 1  192.168.1.1  0.512 ms  0.401 ms  0.399 ms
 2  * * *
 3  100.64.0.1  8.1 ms
    100.64.0.5  9.2 ms  9.0 ms
10  8.8.8.8  14.7 ms  14.2 ms  14.0 ms`.split('\n'),
    );
    expect(hops.map((h) => h.hop)).toEqual([1, 2, 3, 10]);
    expect(hops[0]).toMatchObject({
      text: '192.168.1.1',
      rtts: [0.512, 0.401, 0.399],
      timeouts: 0,
    });
    expect(hops[1]).toMatchObject({ text: '', rtts: [], timeouts: 3 });
    expect(hops[2]).toMatchObject({ text: '100.64.0.1 · 100.64.0.5', rtts: [8.1, 9.2, 9.0] });
  });

  it('parses Windows tracert and Linux tracepath', () => {
    const win = traceHops(
      `Tracing route to 8.8.8.8 over a maximum of 30 hops

  1    <1 ms    <1 ms    <1 ms  192.168.1.1
  2     *        *        *     Request timed out.
  3     9 ms     8 ms     8 ms  8.8.8.8

Trace complete.`.split('\n'),
    );
    expect(win.map((h) => [h.hop, h.text, h.rtts.length, h.timeouts])).toEqual([
      [1, '192.168.1.1', 3, 0],
      [2, 'Request timed out.', 0, 3],
      [3, '8.8.8.8', 3, 0],
    ]);
    const tp = traceHops(
      ` 1?: [LOCALHOST]                      pmtu 1500
 1:  192.168.1.1                                           0.512ms
 1:  192.168.1.1                                           0.420ms
 2:  10.10.0.1                                             3.100ms
     Resume: pmtu 1500 hops 2 back 2`.split('\n'),
    );
    expect(tp.map((h) => [h.hop, h.text, h.rtts])).toEqual([
      [1, '192.168.1.1', [0.512, 0.42]],
      [2, '10.10.0.1', [3.1]],
    ]);
  });
});
