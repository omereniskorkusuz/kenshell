/**
 * Virtualised table (P4-T01): only the rows in view (+ a margin) are in the DOM, so 50 000
 * rows scroll smoothly. Fixed row height; sticky header; click a header to sort.
 */
import { useRef, useState, type ReactNode } from 'react';
import { ArrowDown, ArrowUp } from 'lucide-react';

const ROW_H = 24;
const OVERSCAN = 8;

export interface VColumn {
  label: string;
  /** Initial width in px. */
  width: number;
}

export function VirtualTable({
  columns,
  rows,
  sort,
  onSort,
  cell,
  label,
}: {
  columns: VColumn[];
  rows: string[][];
  sort: { col: number; dir: 1 | -1 } | null;
  onSort: (col: number) => void;
  /** Renders one cell (default: the text). */
  cell?: (value: string, col: number) => ReactNode;
  label: string;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const [top, setTop] = useState(0);
  const [height, setHeight] = useState(400);
  const start = Math.max(0, Math.floor(top / ROW_H) - OVERSCAN);
  const end = Math.min(rows.length, Math.ceil((top + height) / ROW_H) + OVERSCAN);
  const template = columns.map((c) => `minmax(${c.width}px, 1fr)`).join(' ');
  const minWidth = columns.reduce((n, c) => n + c.width, 0);

  return (
    <div
      ref={(el) => {
        ref.current = el;
        if (el && el.clientHeight && el.clientHeight !== height) setHeight(el.clientHeight);
      }}
      role="table"
      aria-label={label}
      aria-rowcount={rows.length + 1}
      className="vtable min-h-0 flex-1 overflow-auto"
      onScroll={(e) => {
        setTop(e.currentTarget.scrollTop);
        setHeight(e.currentTarget.clientHeight);
      }}
    >
      <div style={{ minWidth }}>
        <div
          role="row"
          aria-rowindex={1}
          className="vtable-head sticky top-0 z-[1] grid"
          style={{ gridTemplateColumns: template, height: ROW_H }}
        >
          {columns.map((c, i) => (
            <button
              key={i}
              type="button"
              role="columnheader"
              aria-sort={
                sort?.col === i ? (sort.dir === 1 ? 'ascending' : 'descending') : undefined
              }
              className="flex min-w-0 items-center gap-1 px-2 text-left text-xs font-medium text-secondary"
              onClick={() => onSort(i)}
            >
              <span className="truncate">{c.label}</span>
              {sort?.col === i &&
                (sort.dir === 1 ? (
                  <ArrowUp size={11} aria-hidden />
                ) : (
                  <ArrowDown size={11} aria-hidden />
                ))}
            </button>
          ))}
        </div>
        <div style={{ height: rows.length * ROW_H, position: 'relative' }}>
          {rows.slice(start, end).map((r, n) => {
            const i = start + n;
            return (
              <div
                key={i}
                role="row"
                aria-rowindex={i + 2}
                className="vtable-row absolute grid w-full"
                style={{ gridTemplateColumns: template, height: ROW_H, top: i * ROW_H }}
              >
                {columns.map((_, c) => (
                  <div
                    key={c}
                    role="cell"
                    className="mono truncate px-2 text-xs leading-6"
                    title={r[c] ?? ''}
                  >
                    {cell ? cell(r[c] ?? '', c) : r[c]}
                  </div>
                ))}
              </div>
            );
          })}
        </div>
      </div>
    </div>
  );
}
