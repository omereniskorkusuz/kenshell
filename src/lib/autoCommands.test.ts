import { describe, expect, it } from 'vitest';
import { appendTail, atLoginPrompt, QUIET_MS, readyToSend } from './autoCommands';

const enc = (s: string) => new TextEncoder().encode(s);

describe('auto commands timing', () => {
  it('waits at login prompts', () => {
    expect(atLoginPrompt('\r\nUser Access Verification\r\n\r\nUsername: ')).toBe(true);
    expect(atLoginPrompt('Password:')).toBe(true);
    expect(atLoginPrompt('core-sw-01>')).toBe(false);
    expect(atLoginPrompt('\x1b[1mlogin: \x1b[0m')).toBe(true);
  });
  it('sends only after a quiet period at a shell prompt', () => {
    const tail = appendTail('', enc('banner\r\ncore-sw-01#'));
    expect(readyToSend(tail, 1000, 1000 + QUIET_MS - 1)).toBe(false);
    expect(readyToSend(tail, 1000, 1000 + QUIET_MS)).toBe(true);
    expect(readyToSend('Username: ', 0, 10_000)).toBe(false);
  });
  it('tail stays bounded', () => {
    expect(appendTail('x'.repeat(300), enc('yz'), 256)).toHaveLength(256);
  });
});
