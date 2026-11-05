/**
 * Tiny Markdown subset for notes (P3-T08): headings, paragraphs, line breaks, bullet and
 * numbered lists, fenced code blocks, inline code, bold, italic, links. Output is React
 * elements only (never HTML strings), so note text cannot inject markup; links open only
 * `https://` targets, through the backend's validated opener.
 */
import type { ReactNode } from 'react';
import { openExternal } from './ipc';

type Block =
  | { kind: 'h'; level: 1 | 2 | 3; text: string }
  | { kind: 'p'; lines: string[] }
  | { kind: 'ul' | 'ol'; items: string[] }
  | { kind: 'code'; text: string };

const BULLET = /^\s*[-*]\s+(.*)$/;
const NUMBERED = /^\s*\d+[.)]\s+(.*)$/;
const HEADING = /^(#{1,3})\s+(.*)$/;
const isFence = (l: string) => l.trim().startsWith('```');

export function parseBlocks(src: string): Block[] {
  const lines = src.replace(/\r\n?/g, '\n').split('\n');
  const out: Block[] = [];
  let i = 0;
  const at = (k: number) => lines[k] as string;
  while (i < lines.length) {
    const line = at(i);
    if (isFence(line)) {
      const code: string[] = [];
      i++;
      while (i < lines.length && !isFence(at(i))) code.push(at(i++));
      i++; // closing fence (or end of text)
      out.push({ kind: 'code', text: code.join('\n') });
      continue;
    }
    const h = HEADING.exec(line);
    if (h) {
      out.push({ kind: 'h', level: (h[1] as string).length as 1 | 2 | 3, text: h[2] as string });
      i++;
      continue;
    }
    const listRe = BULLET.test(line) ? BULLET : NUMBERED.test(line) ? NUMBERED : null;
    if (listRe) {
      const items: string[] = [];
      while (i < lines.length && listRe.test(at(i))) {
        items.push((listRe.exec(at(i)) as RegExpExecArray)[1] as string);
        i++;
      }
      out.push({ kind: listRe === BULLET ? 'ul' : 'ol', items });
      continue;
    }
    if (!line.trim()) {
      i++;
      continue;
    }
    const para: string[] = [];
    while (
      i < lines.length &&
      at(i).trim() &&
      !HEADING.test(at(i)) &&
      !isFence(at(i)) &&
      !BULLET.test(at(i)) &&
      !NUMBERED.test(at(i))
    ) {
      para.push(at(i++));
    }
    out.push({ kind: 'p', lines: para });
  }
  return out;
}

const INLINE =
  /(`[^`]+`)|(\*\*[^*]+\*\*)|(\*[^*\s][^*]*\*)|(\[[^\]]+\]\(https:\/\/[^\s)]+\))|(https:\/\/[^\s<>()]+)/g;

export function renderInline(text: string, keyBase = ''): ReactNode[] {
  const out: ReactNode[] = [];
  let last = 0;
  let n = 0;
  for (const m of text.matchAll(INLINE)) {
    const at = m.index ?? 0;
    if (at > last) out.push(text.slice(last, at));
    const tok = m[0];
    const key = `${keyBase}-${n++}`;
    if (m[1]) out.push(<code key={key}>{tok.slice(1, -1)}</code>);
    else if (m[2]) out.push(<strong key={key}>{tok.slice(2, -2)}</strong>);
    else if (m[3]) out.push(<em key={key}>{tok.slice(1, -1)}</em>);
    else {
      const link = m[4] ? /^\[([^\]]+)\]\((https:\/\/[^\s)]+)\)$/.exec(tok) : null;
      const label = link ? (link[1] as string) : tok;
      const href = link ? (link[2] as string) : tok;
      out.push(
        <button
          key={key}
          type="button"
          className="underline decoration-strong underline-offset-2 hover:text-primary"
          title={href}
          onClick={() => openExternal(href)}
        >
          {label}
        </button>,
      );
    }
    last = at + tok.length;
  }
  if (last < text.length) out.push(text.slice(last));
  return out;
}

export function Markdown({ text }: { text: string }) {
  return (
    <div className="md flex flex-col gap-1.5">
      {parseBlocks(text).map((b, i) => {
        const k = String(i);
        switch (b.kind) {
          case 'h':
            return (
              <p key={k} className="font-semibold text-primary">
                {renderInline(b.text, k)}
              </p>
            );
          case 'code':
            return (
              <pre key={k} className="mono overflow-x-auto rounded-sm bg-app p-1.5 text-xs">
                {b.text}
              </pre>
            );
          case 'ul':
          case 'ol': {
            const List = b.kind;
            return (
              <List key={k} className={`${b.kind === 'ul' ? 'list-disc' : 'list-decimal'} pl-4`}>
                {b.items.map((it, j) => (
                  <li key={j}>{renderInline(it, `${k}.${j}`)}</li>
                ))}
              </List>
            );
          }
          case 'p':
            return (
              <p key={k} className="break-words">
                {b.lines.flatMap((l, j) => [
                  ...(j ? [<br key={`br${j}`} />] : []),
                  ...renderInline(l, `${k}.${j}`),
                ])}
              </p>
            );
        }
      })}
    </div>
  );
}
