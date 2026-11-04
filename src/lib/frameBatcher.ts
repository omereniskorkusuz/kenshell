/**
 * Coalesces incoming byte chunks and hands them to `sink` at most once per animation frame.
 *
 * requestAnimationFrame does not fire while the window is hidden/occluded (WKWebView,
 * minimized windows), so a timer fallback (~50 ms) guarantees data never gets stuck:
 * whichever fires first flushes and cancels the other.
 */
import { concatChunks } from './bytes';

export interface FrameScheduler {
  requestFrame(cb: () => void): number;
  cancelFrame(id: number): void;
  setTimer(cb: () => void, ms: number): number;
  clearTimer(id: number): void;
}

export const browserScheduler: FrameScheduler = {
  requestFrame: (cb) =>
    typeof requestAnimationFrame === 'function'
      ? requestAnimationFrame(() => cb())
      : (setTimeout(cb, 16) as unknown as number),
  cancelFrame: (id) => {
    if (typeof cancelAnimationFrame === 'function') cancelAnimationFrame(id);
    else clearTimeout(id);
  },
  setTimer: (cb, ms) => setTimeout(cb, ms) as unknown as number,
  clearTimer: (id) => clearTimeout(id),
};

export const FALLBACK_MS = 50;

export class FrameBatcher {
  private queue: Uint8Array[] = [];
  private queued = 0;
  private frameId: number | null = null;
  private timerId: number | null = null;
  private disposed = false;

  constructor(
    private readonly sink: (data: Uint8Array) => void,
    private readonly scheduler: FrameScheduler = browserScheduler,
    private readonly fallbackMs: number = FALLBACK_MS,
  ) {}

  /** Bytes waiting for the next flush. */
  get pending(): number {
    return this.queued;
  }

  push(chunk: Uint8Array): void {
    if (this.disposed || chunk.length === 0) return;
    this.queue.push(chunk);
    this.queued += chunk.length;
    if (this.frameId === null && this.timerId === null) {
      this.frameId = this.scheduler.requestFrame(this.run);
      this.timerId = this.scheduler.setTimer(this.run, this.fallbackMs);
    }
  }

  /** Flushes synchronously (e.g. before the tab is hidden). */
  flushNow(): void {
    this.run();
  }

  /** Takes the queued bytes without writing them (e.g. to move them to a hidden buffer). */
  take(): Uint8Array | null {
    this.cancel();
    if (this.queued === 0) return null;
    const data = concatChunks(this.queue, this.queued);
    this.queue = [];
    this.queued = 0;
    return data;
  }

  /** Drops queued data and cancels pending callbacks. */
  dispose(): void {
    this.cancel();
    this.queue = [];
    this.queued = 0;
    this.disposed = true;
  }

  private cancel(): void {
    if (this.frameId !== null) this.scheduler.cancelFrame(this.frameId);
    if (this.timerId !== null) this.scheduler.clearTimer(this.timerId);
    this.frameId = null;
    this.timerId = null;
  }

  private readonly run = (): void => {
    const data = this.take();
    if (data) this.sink(data);
  };
}
