/**
 * Output transforms between the session bytes and xterm (pure; tested):
 * - character set decoding (UTF-8 passes through as bytes so xterm handles split
 *   sequences; ISO-8859-9 / Windows-1254 via TextDecoder; CP437 via a table)
 * - optional per-line timestamps (dim `[HH:MM:SS] ` inserted at each line start)
 */

export type Charset = 'utf-8' | 'iso-8859-9' | 'windows-1254' | 'cp437';
export const CHARSETS: readonly Charset[] = ['utf-8', 'iso-8859-9', 'windows-1254', 'cp437'];

/** Upper half (0x80–0xFF) of IBM code page 437. */
const CP437_HIGH =
  'ÇüéâäàåçêëèïîìÄÅÉæÆôöòûùÿÖÜ¢£¥₧ƒáíóúñÑªº¿⌐¬½¼¡«»' +
  '░▒▓│┤╡╢╖╕╣║╗╝╜╛┐└┴┬├─┼╞╟╚╔╩╦╠═╬╧╨╤╥╙╘╒╓╫╪┘┌█▄▌▐▀' +
  'αßΓπΣσµτΦΘΩδ∞φε∩≡±≥≤⌠⌡÷≈°∙·√ⁿ²■ ';

export function decodeCp437(bytes: Uint8Array): string {
  let s = '';
  for (const b of bytes) s += b < 0x80 ? String.fromCharCode(b) : (CP437_HIGH[b - 0x80] ?? '?');
  return s;
}

export type Decoder = (bytes: Uint8Array) => Uint8Array | string;

export function makeDecoder(charset: Charset): Decoder {
  switch (charset) {
    case 'utf-8':
      return (b) => b;
    case 'cp437':
      return decodeCp437;
    default: {
      const d = new TextDecoder(charset);
      return (b) => d.decode(b, { stream: true });
    }
  }
}

const DIM = '\x1b[2m';
const UNDIM = '\x1b[22m';

/** Inserts a timestamp at the start of every line (lazily: when the line gets content). */
export class Timestamper {
  private lineStart = true;
  private readonly utf8 = new TextDecoder('utf-8');

  constructor(private readonly now: () => string = clock) {}

  feed(chunk: Uint8Array | string): string {
    const text = typeof chunk === 'string' ? chunk : this.utf8.decode(chunk, { stream: true });
    let out = '';
    for (const ch of text) {
      if (ch === '\n') {
        out += ch;
        this.lineStart = true;
        continue;
      }
      if (this.lineStart && ch !== '\r') {
        out += `${DIM}[${this.now()}]${UNDIM} `;
        this.lineStart = false;
      }
      out += ch;
    }
    return out;
  }
}

function clock(): string {
  const d = new Date();
  const p = (n: number) => String(n).padStart(2, '0');
  return `${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())}`;
}

/** Selection → clipboard text: trailing spaces removed from every line (PuTTY habit). */
export function cleanCopy(text: string): string {
  return text
    .split(/\r?\n/)
    .map((l) => l.replace(/[ \t]+$/, ''))
    .join('\n')
    .replace(/\n+$/, '');
}
