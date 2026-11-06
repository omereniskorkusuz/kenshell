/** Star layout for the neighbour view (P4-T10): the device in the middle, neighbours on a circle. */
export interface Point {
  x: number;
  y: number;
}

export function starLayout(count: number, size: number, radiusRatio = 0.36): Point[] {
  const c = size / 2;
  const r = size * radiusRatio;
  return Array.from({ length: count }, (_, i) => {
    // Start at the top, clockwise.
    const a = -Math.PI / 2 + (2 * Math.PI * i) / Math.max(1, count);
    return { x: c + r * Math.cos(a), y: c + r * Math.sin(a) };
  });
}

/** Short label: the host name without its domain, at most `max` characters. */
export function shortName(name: string, max = 16): string {
  const host = name.split('.')[0] ?? name;
  return host.length > max ? `${host.slice(0, max - 1)}…` : host;
}
