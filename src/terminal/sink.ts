/**
 * Per-tab output sink: sits between the live session's byte stream and xterm.
 *
 * - visible + terminal attached → bytes are batched (once per animation frame, timer fallback)
 * - hidden or terminal not yet created → bytes are buffered (cap 8 MiB, oldest dropped) and
 *   flushed when the tab is shown, so hidden tabs never make xterm do work.
 * - hidden tabs drop their xterm instance after a few seconds; `history` keeps what it had
 *   shown so a new instance can replay it (P2-T23 memory budget).
 *
 * Kept free of xterm imports so it lives in the main chunk; the terminal chunk attaches a
 * writer once it has created the Terminal.
 */
import { ByteQueue, HIDDEN_BUFFER_CAP, trimToLineStart } from '../lib/bytes';
import { CHARS_PER_LINE, OutputHistory } from './history';
import {
  browserScheduler,
  FALLBACK_MS,
  FrameBatcher,
  type FrameScheduler,
} from '../lib/frameBatcher';

export type Writer = (data: Uint8Array | string) => void;

export interface SinkOptions {
  cap?: number;
  scheduler?: FrameScheduler;
  /** Text written before a flush when older output had to be dropped. */
  dropNotice?: (droppedBytes: number) => string;
}

export class TerminalSink {
  private readonly hidden: ByteQueue;
  private readonly batcher: FrameBatcher;
  private readonly dropNotice: ((n: number) => string) | undefined;
  private writer: Writer | null = null;
  private visible = false;

  /** Totals for the status bar (read by polling, never via React state). */
  rx = 0;
  tx = 0;
  /** Current terminal grid (set by the terminal view after each fit). */
  cols = 0;
  rows = 0;
  /**
   * Text the terminal has shown (replayed when a disposed terminal is recreated, carried
   * over when the tab moves to another window).
   */
  readonly history = new OutputHistory(10_000 * CHARS_PER_LINE);
  /** Terminal-view state that must survive the xterm instance (charset decoder, stamper). */
  textState: unknown = null;

  constructor(opts: SinkOptions = {}) {
    this.hidden = new ByteQueue(opts.cap ?? HIDDEN_BUFFER_CAP);
    this.batcher = new FrameBatcher(
      (data) => this.writer?.(data),
      opts.scheduler ?? browserScheduler,
      FALLBACK_MS,
    );
    this.dropNotice = opts.dropNotice;
  }

  get isVisible(): boolean {
    return this.visible;
  }

  /** Bytes waiting in the hidden buffer. */
  get buffered(): number {
    return this.hidden.size;
  }

  push(bytes: Uint8Array): void {
    if (bytes.length === 0) return;
    this.rx += bytes.length;
    if (this.writer && this.visible) this.batcher.push(bytes);
    else this.hidden.push(bytes);
  }

  /** Called by the terminal component once xterm exists. */
  attach(writer: Writer): void {
    this.writer = writer;
    if (this.visible) this.flushHidden();
  }

  detach(): void {
    this.park();
    this.writer = null;
  }

  setVisible(visible: boolean): void {
    if (visible === this.visible) return;
    this.visible = visible;
    if (!visible) this.park();
    else if (this.writer) this.flushHidden();
  }

  /** Writes local text (notices) through the same ordered path as device output. */
  writeLocal(text: string): void {
    const bytes = new TextEncoder().encode(text);
    if (this.writer && this.visible) this.batcher.push(bytes);
    else this.hidden.push(bytes);
  }

  /**
   * Removes and returns every byte not yet handed to xterm (batched or hidden), e.g. to
   * move the tab to another window. Dropped-output notices are not repeated.
   */
  takePending(): Uint8Array {
    this.park();
    return this.hidden.drain().bytes;
  }

  dispose(): void {
    this.batcher.dispose();
    this.hidden.clear();
    this.history.clear();
    this.textState = null;
    this.writer = null;
  }

  /** Moves not-yet-written batched bytes back into the hidden buffer (keeps order). */
  private park(): void {
    const pending = this.batcher.take();
    if (!pending) return;
    // Pending bytes are older than anything pushed after this point, but newer than what
    // is already in the hidden buffer (which is empty while visible), so appending is right.
    this.hidden.push(pending);
  }

  private flushHidden(): void {
    if (this.hidden.size === 0) return;
    const { bytes, dropped } = this.hidden.drain();
    if (dropped > 0) {
      if (this.dropNotice) this.writer?.(this.dropNotice(dropped));
      this.batcher.push(trimToLineStart(bytes));
    } else {
      this.batcher.push(bytes);
    }
  }
}

const sinks = new Map<string, TerminalSink>();

export function createSink(key: string, opts?: SinkOptions): TerminalSink {
  const sink = new TerminalSink(opts);
  sinks.set(key, sink);
  return sink;
}

export const getSink = (key: string): TerminalSink | undefined => sinks.get(key);

export function disposeSink(key: string): void {
  sinks.get(key)?.dispose();
  sinks.delete(key);
}
