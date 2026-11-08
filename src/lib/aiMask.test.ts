import { describe, expect, it } from 'vitest';
import { maskLine, maskSecrets } from './aiMask';

const m = (s: string) => maskLine(s).text;

describe('aiMask', () => {
  it('masks Cisco and Ruijie secrets', () => {
    expect(m('enable secret 9 $9$abcdEFGH')).toBe('enable secret 9 <masked>');
    expect(m('enable password cisco123')).toBe('enable password <masked>');
    expect(m('username admin privilege 15 secret 5 $1$xyz')).toBe(
      'username admin privilege 15 secret 5 <masked>',
    );
    expect(m('enable secret level 15 5 $1$ruijie')).toBe('enable secret level 15 5 <masked>');
    expect(m(' password 7 0822455D0A16')).toBe(' password 7 <masked>');
    expect(m('snmp-server community Publ1c RO')).toBe('snmp-server community <masked> RO');
    expect(m('tacacs-server host 10.0.0.5 key S3cr3t')).toBe(
      'tacacs-server host 10.0.0.5 key <masked>',
    );
    expect(m('crypto isakmp key vpnkey address 1.2.3.4')).toBe(
      'crypto isakmp key <masked> address 1.2.3.4',
    );
    expect(m(' key-string 7 1234ABCD')).toBe(' key-string 7 <masked>');
    expect(m('snmp-server user u1 g1 v3 auth sha AuthPass priv aes 128 PrivPass')).toBe(
      'snmp-server user u1 g1 v3 auth sha <masked> priv aes 128 <masked>',
    );
    expect(m(' neighbor 10.1.1.1 password bgpPass')).toBe(' neighbor 10.1.1.1 password <masked>');
  });

  it('leaves ordinary lines and key names alone', () => {
    for (const keep of [
      'key chain OSPF-KEYS',
      'crypto key generate rsa modulus 2048',
      'GigabitEthernet0/1 is up, line protocol is up',
      'Password:',
      'show running-config | include username',
    ]) {
      expect(maskLine(keep)).toEqual({ text: keep, count: 0 });
    }
  });

  it('masks PEM private keys and counts', () => {
    const r = maskSecrets(
      'a\n-----BEGIN RSA PRIVATE KEY-----\nMIIabc\n-----END RSA PRIVATE KEY-----\nenable secret 5 x',
    );
    expect(r.text).toBe('a\n<masked>\nenable secret 5 <masked>');
    expect(r.count).toBe(2);
  });
});
