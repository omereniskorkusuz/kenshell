/**
 * One tab's pane: mode stripe (full height) + identity bar + terminal. Panes of inactive tabs
 * stay mounted but hidden so their xterm instance survives tab switches.
 *
 * The pane carries data-mode / data-env; Phase 3 only has to change data-mode.
 */
import { lazy, Suspense, useEffect, useRef, useState, type CSSProperties } from 'react';
import { compactParts, isCompact } from '../lib/compact';
import type { LiveTab } from '../lib/sessionState';
import { envKind } from '../lib/tree';
import { useTabs } from '../stores/tabs';
import { useUi } from '../stores/ui';
import { IdentityBar } from './IdentityBar';
import { SearchBar } from './SearchBar';

export const loadTerminalView = () => import('../terminal/TerminalView');
const TerminalView = lazy(loadTerminalView);

interface Props {
  tab: LiveTab;
  visible: boolean;
  /** The active tab (keyboard focus; highlighted in split view). */
  focused: boolean;
  /** Box in percent of the workspace when shown in split view, else full size. */
  rect: { left: number; top: number; width: number; height: number } | null;
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

export function Pane({ tab, visible, focused, rect }: Props) {
  const ref = useRef<HTMLElement>(null);
  const [layout, setLayout] = useState<Layout>({ compact: false, width: 0 });
  const searching = useUi((s) => s.findKey === tab.key);
  const dropZone = useUi((s) => (s.tabDrag?.pane === tab.key ? s.tabDrag.zone : null));
  const box: CSSProperties | undefined = rect
    ? {
        left: `${rect.left}%`,
        top: `${rect.top}%`,
        width: `${rect.width}%`,
        height: `${rect.height}%`,
      }
    : undefined;

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
      data-vendor={tab.device.vendor?.family}
      data-env={envKind(tab.envTag) ?? undefined}
      data-state={tab.state}
      data-pane={tab.key}
      data-split={rect ? 'true' : undefined}
      data-focused={focused}
      style={box}
      className={`pane absolute flex bg-terminal ${rect ? '' : 'inset-0'}`}
      // In split view a click anywhere in a pane makes it the active one.
      onPointerDownCapture={() => {
        if (rect && !focused) useTabs.getState().activate(tab.key);
      }}
    >
      <div className="mode-stripe shrink-0" aria-hidden />
      <div className="flex min-w-0 flex-1 flex-col">
        <IdentityBar tab={tab} compact={layout.compact} width={layout.width} visible={visible} />
        <div className="relative flex min-h-0 flex-1 flex-col">
          {searching && visible && <SearchBar tabKey={tab.key} />}
          <Suspense fallback={<div className="min-h-0 flex-1 bg-terminal" />}>
            <TerminalView tabKey={tab.key} visible={visible} focused={visible && focused} />
          </Suspense>
        </div>
      </div>
      {dropZone && <div className="drop-hint" data-zone={dropZone} aria-hidden />}
    </section>
  );
}
