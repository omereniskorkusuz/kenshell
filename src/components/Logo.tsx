/**
 * KenShell logo: a console port (RJ45 jack) with a `>_` prompt inside. Colours come
 * from tokens (`--brand`, `--text-primary`), so it follows the theme.
 * The same drawing, on a dark tile, is the app icon (`src-tauri/icons/source.svg`).
 */

const PINS = [14, 21, 28, 35, 45, 52, 59, 66];

export function LogoMark({ size = 20, className = '' }: { size?: number; className?: string }) {
  // Below ~20px the pins turn into noise; leave them out.
  const pins = size >= 28;
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 80 80"
      className={className}
      aria-hidden
      focusable="false"
    >
      <path
        d="M4 8Q4 4 8 4H72Q76 4 76 8V64H60V76H20V64H4Z"
        fill="none"
        stroke="var(--brand)"
        strokeWidth={size >= 28 ? 5 : 7}
        strokeLinejoin="round"
      />
      {pins && (
        <g stroke="var(--brand)" strokeWidth={3} strokeLinecap="round">
          {PINS.map((x) => (
            <line key={x} x1={x} y1={13} x2={x} y2={22} />
          ))}
        </g>
      )}
      <path
        d={pins ? 'M24 35L34 43L24 51' : 'M22 30L35 41L22 52'}
        fill="none"
        stroke="var(--text-primary)"
        strokeWidth={pins ? 5 : 8}
        strokeLinecap="round"
        strokeLinejoin="round"
      />
      <line
        x1={pins ? 40 : 41}
        y1={pins ? 52 : 53}
        x2={pins ? 55 : 58}
        y2={pins ? 52 : 53}
        stroke="var(--text-primary)"
        strokeWidth={pins ? 5 : 8}
        strokeLinecap="round"
      />
    </svg>
  );
}

/** "KenShell" with "Shell" in the brand colour. The name is a product name, not UI text. */
export function Wordmark({ className = '' }: { className?: string }) {
  return (
    <span className={`font-semibold tracking-tight ${className}`}>
      Ken<span className="text-brand">Shell</span>
    </span>
  );
}

export function Logo({ size = 18, className = '' }: { size?: number; className?: string }) {
  return (
    <span className={`inline-flex items-center gap-1.5 ${className}`}>
      <LogoMark size={size} />
      <Wordmark />
    </span>
  );
}
