import { describe, expect, it } from 'vitest';
import { feedPrompt, promptIntro, startPrompt } from './prompt';

const login = {
  instruction: '',
  items: [
    { text: 'login as: ', echo: true },
    { text: 'Password: ', echo: false },
  ],
};

describe('terminal login prompt', () => {
  it('echoes the user name, hides the password and submits both', () => {
    let p = startPrompt(login);
    expect(promptIntro(login)).toBe('\r\nlogin as: ');
    let step = feedPrompt(p, 'admn\x7fin\r');
    expect(step.echo).toBe('admn\b \bin\r\nPassword: ');
    expect(step.submit).toBeNull();
    if (!step.prompt) throw new Error('prompt expected');
    p = step.prompt;
    step = feedPrompt(p, 's3cr3t');
    expect(step.echo).toBe('');
    if (!step.prompt) throw new Error('prompt expected');
    step = feedPrompt(step.prompt, '\r');
    expect(step.submit).toEqual(['admin', 's3cr3t']);
    expect(step.prompt).toBeNull();
  });

  it('a pasted line with Enter is handled in one chunk', () => {
    const step = feedPrompt(
      startPrompt({ instruction: '', items: [{ text: 'P: ', echo: false }] }),
      'pw\r',
    );
    expect(step.submit).toEqual(['pw']);
  });

  it('Ctrl-C cancels, escape sequences and Ctrl-U are handled', () => {
    expect(feedPrompt(startPrompt(login), 'ad\x03').cancel).toBe(true);
    const step = feedPrompt(startPrompt(login), 'ab\x1b[Dc\x15xy');
    expect(step.prompt?.buffer).toBe('xy');
  });

  it('prints the instruction before the first item', () => {
    expect(
      promptIntro({ instruction: 'Welcome\nline2', items: [{ text: 'Q: ', echo: true }] }),
    ).toBe('\r\nWelcome\r\nline2\r\nQ: ');
  });
});
