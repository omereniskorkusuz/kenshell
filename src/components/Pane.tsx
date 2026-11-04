/**
 * One tab's pane: mode stripe (full height) + identity bar + terminal. Panes of inactive tabs
 * stay mounted but hidden so their xterm instance survives tab switches.
 *
 * The pane carries data-mode / data-env; Phase 3 only has to change data-mode.
 */
import { lazy, Suspense, useEffect, useRef, useState } from 'react';
import { compactParts, isCompact } from '../lib/compact';
import type { LiveTab } from '../lib/sessionState';
import { envKind } from '../lib/tree';
import { IdentityBar } from './IdentityBar';

export const loadTerminalView = () => import('../terminal/TerminalView');
const TerminalView = lazy(loadTerminalView);

interface Props {
  tab: LiveTab;
  visible: boolean;
}

interface Layout {
  compact: boolean;
  /** Width bucket that changes compact parts; avoids re-rendering on every pixel. */
  width: number;
}

function layoutFor(width: number, height: number): Layout {
  const compact = isCompact({ width, height });
  if (!compact) return { compact, width: 0 };
  const p = compactParts(width);
  // Representative width per parts combination (only these matter for rendering).
  const bucket = p.uptime ? 960 : p.chip ? 820 : p.protocol ? 640 : 0;
  return { compact, width: bucket };
}

export function Pane({ tab, visible }: Props) {
  const ref = useRef<HTMLElement>(null);
  const [layout, setLayout] = useState<Layout>({ compact: false, width: 0 });

  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const ro = new ResizeObserver(([entry]) => {
      if (!entry) return;
      const { width, height } = entry.contentRect;
      if (width === 0 || height === 0) return; // hidden pane: keep the last layout
      const next = layoutFor(width, height);
      setLayout((prev) =>
        prev.compact === next.compact && prev.width === next.width ? prev : next,
      );
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  return (
    <section
      ref={ref}
      id={`pane-${tab.key}`}
      role="tabpanel"
      aria-labelledby={`tab-${tab.key}`}
      hidden={!visible}
      data-mode={tab.mode}
      data-env={envKind(tab.envTag) ?? undefined}
      data-state={tab.state}
      className="pane absolute inset-0 flex bg-terminal"
    >
      <div className="mode-stripe shrink-0" aria-hidden />
      <div className="flex min-w-0 flex-1 flex-col">
        <IdentityBar tab={tab} compact={layout.compact} width={layout.width} visible={visible} />
        <Suspense fallback={<div className="min-h-0 flex-1 bg-terminal" />}>
          <TerminalView tabKey={tab.key} visible={visible} />
        </Suspense>
      </div>
    </section>
  );
}
