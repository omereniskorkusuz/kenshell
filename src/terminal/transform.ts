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

/** Code unit for every byte (all CP437 characters are in the BMP). */
const CP437_UNITS = Uint16Array.from({ length: 256 }, (_, b) =>
  b < 0x80 ? b : CP437_HIGH.charCodeAt(b - 0x80) || 0x3f,
);
const UTF16 = new TextDecoder('utf-16le');

/** Bulk decode (no per-byte string building: large outputs must not make garbage). */
export function decodeCp437(bytes: Uint8Array): string {
  const units = new Uint16Array(bytes.length);
  for (let i = 0; i < bytes.length; i++) units[i] = CP437_UNITS[bytes[i] as number] as number;
  return UTF16.decode(units);
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
    // Whole line segments at a time (no per-character string building).
    const parts: string[] = [];
    let i = 0;
    while (i < text.length) {
      if (this.lineStart) {
        let j = i;
        while (j < text.length && text.charCodeAt(j) === 13) j++; // CRs never get a stamp
        if (j > i) parts.push(text.slice(i, j));
        i = j;
        if (i >= text.length) break;
        if (text.charCodeAt(i) === 10) {
          parts.push('\n'); // empty line: still at a line start
          i++;
          continue;
        }
        parts.push(`${DIM}[${this.now()}]${UNDIM} `);
        this.lineStart = false;
      }
      const nl = text.indexOf('\n', i);
      if (nl < 0) {
        parts.push(text.slice(i));
        break;
      }
      parts.push(text.slice(i, nl + 1));
      this.lineStart = true;
      i = nl + 1;
    }
    return parts.join('');
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
