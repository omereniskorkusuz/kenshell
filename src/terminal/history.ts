/**
 * What a tab's terminal has shown: the exact chunks written to xterm (text with escape
 * sequences and timestamps, or raw UTF-8 bytes that xterm decodes itself), capped at roughly
 * the scrollback size. Hidden tabs dispose their xterm instance (in WebKit one with 10 000
 * lines costs ~100 MB); when the tab is shown again a fresh terminal replays these chunks
 * (≈1 byte per character of device output).
 *
 * Trimming cuts at a line start, so no escape sequence or UTF-8 character is split
 * (neither ever contains "\n").
 */

export type Chunk = string | Uint8Array;

/** Average characters kept per scrollback line (device output incl. colour codes). */
export const CHARS_PER_LINE = 160;

const NL = 0x0a;

export class OutputHistory {
  private chunks: Chunk[] = [];
  private size = 0;
  private limit: number;

  constructor(maxChars: number) {
    this.limit = Math.max(1024, maxChars);
  }

  /** Characters + bytes kept. */
  get length(): number {
    return this.size;
  }

  setLimit(maxChars: number): void {
    this.limit = Math.max(1024, maxChars);
    this.trim();
  }

  push(chunk: Chunk): void {
    if (chunk.length === 0) return;
    this.chunks.push(chunk);
    this.size += chunk.length;
    if (this.size > this.limit) this.trim();
  }

  /** The chunks in order, adjacent ones of the same kind merged (cheap to replay). */
  replay(): readonly Chunk[] {
    const out: Chunk[] = [];
    for (const c of this.chunks) {
      const last = out[out.length - 1];
      if (typeof c === 'string' && typeof last === 'string') out[out.length - 1] = last + c;
      else if (typeof c !== 'string' && last !== undefined && typeof last !== 'string') {
        const both = new Uint8Array(last.length + c.length);
        both.set(last);
        both.set(c, last.length);
        out[out.length - 1] = both;
      } else out.push(c);
    }
    this.chunks = out;
    return out;
  }

  /** Everything as text (byte chunks are UTF-8). */
  text(): string {
    const utf8 = new TextDecoder('utf-8');
    let s = '';
    for (const c of this.replay())
      s += typeof c === 'string' ? c : utf8.decode(c, { stream: true });
    return s + utf8.decode();
  }

  clear(): void {
    this.chunks = [];
    this.size = 0;
  }

  private trim(): void {
    let excess = this.size - this.limit;
    while (excess > 0 && this.chunks.length > 0) {
      const first = this.chunks[0] as Chunk;
      if (first.length <= excess) {
        this.chunks.shift();
        this.size -= first.length;
        excess -= first.length;
        continue;
      }
      // Cut inside this chunk at the next line start after `excess`.
      const nl =
        typeof first === 'string' ? first.indexOf('\n', excess) : first.indexOf(NL, excess);
      const cut = nl < 0 ? first.length : nl + 1;
      const rest = first.slice(cut);
      if (rest.length > 0) this.chunks[0] = rest;
      else this.chunks.shift();
      this.size -= cut;
      excess = 0;
    }
  }
}
