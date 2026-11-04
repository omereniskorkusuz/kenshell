/** Byte helpers for the terminal data path. */

/** Concatenates chunks into one buffer. Returns the single chunk as-is when there is only one. */
export function concatChunks(chunks: readonly Uint8Array[], total?: number): Uint8Array {
  if (chunks.length === 1) return chunks[0] as Uint8Array;
  const size = total ?? chunks.reduce((n, c) => n + c.length, 0);
  const out = new Uint8Array(size);
  let offset = 0;
  for (const c of chunks) {
    out.set(c, offset);
    offset += c.length;
  }
  return out;
}

/** Default cap for output buffered while a tab is hidden (8 MiB, drop oldest). */
export const HIDDEN_BUFFER_CAP = 8 * 1024 * 1024;

/**
 * FIFO of byte chunks with a hard size cap. When the cap is exceeded the OLDEST bytes are
 * dropped (at byte granularity) so the newest output is always kept.
 *
 * Chunks are stored by reference: callers must not reuse a pushed buffer (Tauri channels
 * hand out a fresh ArrayBuffer per message).
 */
export class ByteQueue {
  private chunks: Uint8Array[] = [];
  private head = 0;
  private bytes = 0;
  private droppedBytes = 0;

  constructor(readonly cap: number = HIDDEN_BUFFER_CAP) {
    if (!(cap > 0)) throw new RangeError('cap must be > 0');
  }

  /** Bytes currently buffered. */
  get size(): number {
    return this.bytes;
  }

  /** Bytes dropped since the last drain. */
  get dropped(): number {
    return this.droppedBytes;
  }

  push(chunk: Uint8Array): void {
    if (chunk.length === 0) return;
    if (chunk.length >= this.cap) {
      // The new chunk alone fills the queue: everything older goes.
      this.droppedBytes += this.bytes + (chunk.length - this.cap);
      this.chunks = [chunk.subarray(chunk.length - this.cap)];
      this.head = 0;
      this.bytes = this.cap;
      return;
    }
    this.chunks.push(chunk);
    this.bytes += chunk.length;
    while (this.bytes > this.cap) {
      const first = this.chunks[this.head] as Uint8Array;
      const excess = this.bytes - this.cap;
      if (first.length <= excess) {
        this.head++;
        this.bytes -= first.length;
        this.droppedBytes += first.length;
      } else {
        this.chunks[this.head] = first.subarray(excess);
        this.bytes -= excess;
        this.droppedBytes += excess;
      }
    }
    // Keep the backing array from growing without bound when many chunks were dropped.
    if (this.head > 64 && this.head * 2 > this.chunks.length) {
      this.chunks = this.chunks.slice(this.head);
      this.head = 0;
    }
  }

  /** Returns everything buffered (concatenated) plus the dropped count, and resets. */
  drain(): { bytes: Uint8Array; dropped: number } {
    const live = this.head === 0 ? this.chunks : this.chunks.slice(this.head);
    const bytes = live.length === 0 ? new Uint8Array(0) : concatChunks(live, this.bytes);
    const dropped = this.droppedBytes;
    this.clear();
    return { bytes, dropped };
  }

  clear(): void {
    this.chunks = [];
    this.head = 0;
    this.bytes = 0;
    this.droppedBytes = 0;
  }
}

/**
 * After older bytes were dropped the buffer may start in the middle of a line, an escape
 * sequence or a UTF-8 sequence. Skip to just after the first line feed if one exists in the
 * first `window` bytes; otherwise return the data unchanged.
 */
export function trimToLineStart(data: Uint8Array, window = 64 * 1024): Uint8Array {
  const limit = Math.min(data.length, window);
  for (let i = 0; i < limit; i++) {
    if (data[i] === 0x0a) return data.subarray(i + 1);
  }
  return data;
}
