import { describe, expect, it } from 'vitest';
import { normalizePaste, pasteChunks, wantsPacing } from '../lib/paste';
import { cleanCopy, decodeCp437, makeDecoder, Timestamper } from './transform';

describe('charset decoding', () => {
  it('utf-8 passes bytes through untouched', () => {
    const b = new Uint8Array([0xc5, 0x9f]);
    expect(makeDecoder('utf-8')(b)).toBe(b);
  });
  it('turkish iso-8859-9 and cp437 box drawing', () => {
    expect(makeDecoder('iso-8859-9')(new Uint8Array([0xfe, 0xf0, 0xfd]))).toBe('şğı');
    expect(decodeCp437(new Uint8Array([0x41, 0xc9, 0xcd, 0xbb]))).toBe('A╔═╗');
  });
});

describe('timestamps', () => {
  it('stamps each line once, lazily, across chunks', () => {
    const ts = new Timestamper(() => '12:00:00');
    const a = ts.feed('one\r\ntw');
    const b = ts.feed(new TextEncoder().encode('o\r\n'));
    const plain = (a + b).replaceAll('\x1b[2m', '').replaceAll('\x1b[22m', '');
    expect(plain).toBe('[12:00:00] one\r\n[12:00:00] two\r\n');
  });
});

describe('copy and paste', () => {
  it('copy trims trailing spaces of every line', () => {
    expect(cleanCopy('Gi0/1   up   \nGi0/2  down  \n\n')).toBe('Gi0/1   up\nGi0/2  down');
  });
  it('paste normalizes newlines to CR and splits per line', () => {
    expect(normalizePaste('a\r\nb\nc')).toBe('a\rb\rc');
    expect(pasteChunks('conf t\ninterface gi0/1\n')).toEqual(['conf t\r', 'interface gi0/1\r']);
    expect(pasteChunks('single')).toEqual(['single']);
  });
  it('paces only slow links with more than one line', () => {
    expect(wantsPacing('serial', 3, 50)).toBe(true);
    expect(wantsPacing('telnet', 3, 50)).toBe(true);
    expect(wantsPacing('ssh', 3, 50)).toBe(false);
    expect(wantsPacing('serial', 1, 50)).toBe(false);
    expect(wantsPacing('serial', 3, 0)).toBe(false);
  });
});

describe('Timestamper matches the per-character reference', () => {
  it('on random text split into random chunks', () => {
    const reference = (chunks: string[]) => {
      let lineStart = true;
      let out = '';
      for (const ch of chunks.join('')) {
        if (ch === '\n') {
          out += ch;
          lineStart = true;
          continue;
        }
        if (lineStart && ch !== '\r') {
          out += '\x1b[2m[T]\x1b[22m ';
          lineStart = false;
        }
        out += ch;
      }
      return out;
    };
    let seed = 7;
    const rnd = (n: number) => {
      seed = (seed * 1103515245 + 12345) % 2147483648;
      return seed % n;
    };
    const alphabet = ['a', 'b', ' ', '\r', '\n', '\r\n', '#', 'ğ'];
    for (let round = 0; round < 50; round++) {
      let text = '';
      for (let i = 0; i < 200; i++) text += alphabet[rnd(alphabet.length)];
      const chunks: string[] = [];
      for (let i = 0; i < text.length;) {
        const n = 1 + rnd(20);
        chunks.push(text.slice(i, i + n));
        i += n;
      }
      const t = new Timestamper(() => 'T');
      expect(chunks.map((c) => t.feed(c)).join('')).toBe(reference(chunks));
    }
  });
});
