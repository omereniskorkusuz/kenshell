/**
 * A tiny Cisco-IOS-like CLI used by the browser mock: echo, prompt, mode changes and a few
 * canned `show` outputs. Demo only — never shipped in the Tauri app's code path.
 */

type Level = 'user' | 'priv' | 'config' | 'configIf';

const VERSION = [
  'Cisco IOS XE Software, Version 17.09.04a',
  'Cisco IOS Software [Cupertino], Catalyst L3 Switch Software (CAT9K_IOSXE), Version 17.9.4a',
  '',
  'ROM: IOS-XE ROMMON',
  'BOOTLDR: System Bootstrap, Version 17.9.1r',
  '',
  '{host} uptime is 12 weeks, 3 days, 4 hours, 18 minutes',
  'System image file is "flash:packages.conf"',
  '',
  'cisco C9300-48P (X86) processor with 1300000K/6147K bytes of memory.',
  '48 Gigabit Ethernet interfaces',
  '4 Ten Gigabit Ethernet interfaces',
  '',
  'Configuration register is 0x102',
];

const IP_INT_BRIEF = [
  'Interface              IP-Address      OK? Method Status                Protocol',
  'Vlan1                  unassigned      YES NVRAM  administratively down down',
  'Vlan10                 10.0.10.2       YES NVRAM  up                    up',
  'GigabitEthernet1/0/1   unassigned      YES unset  up                    up',
  'GigabitEthernet1/0/2   unassigned      YES unset  down                  down',
  'GigabitEthernet1/0/24  unassigned      YES unset  up                    up',
  'TenGigabitEthernet1/1/1 unassigned     YES unset  up                    up',
];

const HELP_USER = [
  '  enable      Turn on privileged commands',
  '  ping        Send echo messages',
  '  show        Show running system information',
  '  exit        Exit from the EXEC',
];
const HELP_PRIV = [
  ...HELP_USER,
  '  configure   Enter configuration mode',
  '  disable     Turn off privileged commands',
  '  reload      Halt and perform a cold restart',
];
const HELP_CONFIG = [
  '  interface   Select an interface to configure',
  "  hostname    Set system's network name",
  '  end         Exit from configure mode',
  '  exit        Exit from configure mode',
];

export class FakeCli {
  private level: Level = 'user';
  private line = '';

  constructor(private host: string) {}

  prompt(): string {
    switch (this.level) {
      case 'user':
        return `${this.host}>`;
      case 'priv':
        return `${this.host}#`;
      case 'config':
        return `${this.host}(config)#`;
      case 'configIf':
        return `${this.host}(config-if)#`;
    }
  }

  banner(): string {
    return `\r\n\r\n${this.host} con0 is now available\r\n\r\nPress RETURN to get started.\r\n\r\n${this.prompt()}`;
  }

  /** Feeds terminal input; returns what the device echoes/prints. */
  input(data: string): string {
    let out = '';
    for (const ch of data) {
      if (ch === '\r' || ch === '\n') {
        out += '\r\n' + this.execute(this.line.trim());
        this.line = '';
      } else if (ch === '\x7f' || ch === '\b') {
        if (this.line.length > 0) {
          this.line = this.line.slice(0, -1);
          out += '\b \b';
        }
      } else if (ch === '\x03') {
        this.line = '';
        out += '^C\r\n' + this.prompt();
      } else if (ch === '\x1a') {
        this.line = '';
        if (this.level === 'config' || this.level === 'configIf') this.level = 'priv';
        out += '^Z\r\n' + this.prompt();
      } else if (ch === '?') {
        out += '?\r\n' + this.help().join('\r\n') + '\r\n\r\n' + this.prompt() + this.line;
      } else if (ch === '\x1b') {
        // Ignore escape sequences (arrow keys) in this demo.
      } else if (ch >= ' ') {
        this.line += ch;
        out += ch;
      }
    }
    return out;
  }

  private help(): string[] {
    if (this.level === 'user') return HELP_USER;
    if (this.level === 'priv') return HELP_PRIV;
    return HELP_CONFIG;
  }

  private execute(cmd: string): string {
    const words = cmd.toLowerCase().split(/\s+/).filter(Boolean);
    const w0 = words[0] ?? '';
    const lines: string[] = [];
    const is = (word: string, min: number) => w0.length >= min && word.startsWith(w0);

    if (cmd === '') {
      // just a new prompt
    } else if (this.level === 'user' && is('enable', 2)) {
      this.level = 'priv';
    } else if (this.level === 'priv' && is('disable', 4)) {
      this.level = 'user';
    } else if (this.level === 'priv' && is('configure', 4)) {
      lines.push('Enter configuration commands, one per line.  End with CNTL/Z.');
      this.level = 'config';
    } else if ((this.level === 'config' || this.level === 'configIf') && is('interface', 3)) {
      this.level = 'configIf';
    } else if ((this.level === 'config' || this.level === 'configIf') && is('hostname', 4)) {
      const name = cmd.split(/\s+/)[1];
      if (name) this.host = name.replace(/[^\w-]/g, '').slice(0, 32) || this.host;
    } else if (is('end', 3) && this.level !== 'user' && this.level !== 'priv') {
      this.level = 'priv';
    } else if (is('exit', 3)) {
      if (this.level === 'configIf') this.level = 'config';
      else if (this.level === 'config') this.level = 'priv';
      else {
        this.level = 'user';
        return `\r\n${this.host} con0 is now available\r\n\r\nPress RETURN to get started.\r\n\r\n${this.prompt()}`;
      }
    } else if (is('show', 2) && (this.level === 'user' || this.level === 'priv')) {
      const what = words.slice(1).join(' ');
      if (/^ver/.test(what)) lines.push(...VERSION.map((l) => l.replace('{host}', this.host)));
      else if (/^ip int/.test(what)) lines.push(...IP_INT_BRIEF);
      else if (/^clock/.test(what)) lines.push(`*${new Date().toUTCString()}`);
      else lines.push('% Incomplete command.');
    } else if (is('ping', 2)) {
      lines.push(
        'Type escape sequence to abort.',
        `Sending 5, 100-byte ICMP Echos to ${words[1] ?? '0.0.0.0'}, timeout is 2 seconds:`,
        '!!!!!',
        'Success rate is 100 percent (5/5), round-trip min/avg/max = 1/1/2 ms',
      );
    } else if (this.level !== 'user' && is('terminal', 3)) {
      // terminal length 0 etc.
    } else {
      lines.push(
        `${' '.repeat(this.prompt().length)}^`,
        "% Invalid input detected at '^' marker.",
        '',
      );
    }
    return lines.map((l) => l + '\r\n').join('') + this.prompt();
  }
}
