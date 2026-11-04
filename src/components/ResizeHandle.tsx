/**
 * Draggable/keyboard resize handle for the sidebar and side panel. During a drag only the CSS
 * variable changes (no React re-render); the final width is stored in settings on release.
 */
import { useRef, useState, type KeyboardEvent, type PointerEvent } from 'react';
import { getSetting, useSettings, type SettingKey } from '../stores/settings';

type WidthKey = Extract<SettingKey, 'ui.sidebarWidth' | 'ui.sidePanelWidth'>;

interface Props {
  /** Edge of the panel the handle sits on. */
  side: 'left' | 'right';
  cssVar: string;
  settingKey: WidthKey;
  min: number;
  max: number;
  label: string;
}

const STEP = 16;

export function ResizeHandle({ side, cssVar, settingKey, min, max, label }: Props) {
  const [active, setActive] = useState(false);
  const drag = useRef<{ x: number; width: number } | null>(null);
  const clamp = (w: number) => Math.round(Math.min(max, Math.max(min, w)));
  const setVar = (w: number) => document.documentElement.style.setProperty(cssVar, `${w}px`);
  const current = () => getSetting(settingKey);

  const onPointerDown = (e: PointerEvent<HTMLDivElement>) => {
    if (e.button !== 0) return;
    e.preventDefault();
    e.currentTarget.setPointerCapture(e.pointerId);
    drag.current = { x: e.clientX, width: current() };
    setActive(true);
  };
  const widthAt = (clientX: number) => {
    const d = drag.current;
    if (!d) return current();
    const delta = clientX - d.x;
    return clamp(d.width + (side === 'right' ? delta : -delta));
  };
  const onPointerMove = (e: PointerEvent<HTMLDivElement>) => {
    if (drag.current) setVar(widthAt(e.clientX));
  };
  const onPointerUp = (e: PointerEvent<HTMLDivElement>) => {
    if (!drag.current) return;
    const w = widthAt(e.clientX);
    drag.current = null;
    setActive(false);
    useSettings.getState().set(settingKey, w);
    setVar(w);
  };
  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    const grow = side === 'right' ? 'ArrowRight' : 'ArrowLeft';
    const shrink = side === 'right' ? 'ArrowLeft' : 'ArrowRight';
    let w: number | null = null;
    if (e.key === grow) w = clamp(current() + STEP);
    else if (e.key === shrink) w = clamp(current() - STEP);
    else if (e.key === 'Home') w = min;
    else if (e.key === 'End') w = max;
    if (w === null) return;
    e.preventDefault();
    useSettings.getState().set(settingKey, w);
  };

  const value = useSettings((s) => s.values[settingKey]);

  return (
    <div
      role="separator"
      aria-orientation="vertical"
      aria-label={label}
      aria-valuemin={min}
      aria-valuemax={max}
      aria-valuenow={value}
      tabIndex={0}
      data-active={active || undefined}
      className="resize-handle"
      style={side === 'right' ? { right: -3 } : { left: -3 }}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerUp}
      onPointerCancel={onPointerUp}
      onKeyDown={onKeyDown}
    />
  );
}
