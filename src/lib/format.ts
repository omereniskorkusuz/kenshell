/** Small, locale-independent formatters used in the chrome (status bar, identity bar). */

/** 999 → "999", 12_400 → "12.1K", 3_500_000 → "3.3M" (binary units, one decimal). */
export function formatBytes(n: number): string {
  if (!Number.isFinite(n) || n < 0) return '0';
  if (n < 1024) return String(Math.floor(n));
  const units = ['K', 'M', 'G', 'T'];
  let v = n;
  let i = -1;
  do {
    v /= 1024;
    i++;
  } while (v >= 1024 && i < units.length - 1);
  return `${v >= 100 ? Math.round(v) : v.toFixed(1)}${units[i]}`;
}

/** Milliseconds → "02:14:07" (hours grow past 99 without wrapping). */
export function formatUptime(ms: number): string {
  const total = Math.max(0, Math.floor(ms / 1000));
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  const pad = (x: number) => String(x).padStart(2, '0');
  return `${pad(h)}:${pad(m)}:${pad(s)}`;
}

/** Serial framing label, e.g. 8N1. */
export function framingLabel(
  dataBits: number,
  parity: 'none' | 'odd' | 'even',
  stopBits: number,
): string {
  const p = parity === 'odd' ? 'O' : parity === 'even' ? 'E' : 'N';
  return `${dataBits}${p}${stopBits}`;
}

/** Last path segment of a serial device path: /dev/cu.usbserial-A1 → cu.usbserial-A1. */
export function shortPortName(path: string): string {
  const i = path.lastIndexOf('/');
  return i >= 0 ? path.slice(i + 1) : path;
}
