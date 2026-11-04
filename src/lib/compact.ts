/**
 * Identity bar density (Concept: "Kompakt mod"). Decided from the PANE size, measured with a
 * ResizeObserver, so future splits behave correctly too.
 */
export const COMPACT_MAX_HEIGHT = 800;
export const COMPACT_MAX_WIDTH = 1100;

export interface PaneSize {
  width: number;
  height: number;
}

/** Single 32px row when the pane is < 800px tall or < 1100px wide. */
export function isCompact({ width, height }: PaneSize): boolean {
  // A 0×0 pane is hidden (display: none); keep the current layout decision neutral.
  if (width <= 0 || height <= 0) return false;
  return height < COMPACT_MAX_HEIGHT || width < COMPACT_MAX_WIDTH;
}

export interface IdentityParts {
  uptime: boolean;
  chip: boolean;
  protocol: boolean;
}

/**
 * Which optional identity facts fit in the compact row. Drop order (Concept): uptime →
 * chip/model/vendor → protocol. Mode, env, name and status never drop.
 */
export function compactParts(width: number): IdentityParts {
  return {
    uptime: width >= 960,
    chip: width >= 820,
    protocol: width >= 640,
  };
}
