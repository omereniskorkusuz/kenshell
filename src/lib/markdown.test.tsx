import { describe, expect, it } from 'vitest';
import { isValidElement } from 'react';
import { parseBlocks, renderInline } from './markdown';

describe('notes markdown', () => {
  it('splits blocks', () => {
    expect(
      parseBlocks('# Rack 3\nline a\nline b\n\n- one\n- two\n1. first\n```\nshow run\n```\nend'),
    ).toEqual([
      { kind: 'h', level: 1, text: 'Rack 3' },
      { kind: 'p', lines: ['line a', 'line b'] },
      { kind: 'ul', items: ['one', 'two'] },
      { kind: 'ol', items: ['first'] },
      { kind: 'code', text: 'show run' },
      { kind: 'p', lines: ['end'] },
    ]);
  });

  it('renders inline marks as elements, never HTML', () => {
    const nodes = renderInline('**PROD** uplink `Gi1/0/1` see https://example.com/x <b>raw</b>');
    const kinds = nodes.map((n) => (isValidElement(n) ? n.type : typeof n));
    expect(kinds).toEqual(['strong', 'string', 'code', 'string', 'button', 'string']);
    expect(nodes[nodes.length - 1]).toBe(' <b>raw</b>');
    // Only https links are links.
    expect(
      renderInline('[x](javascript:alert(1)) http://a.b').every((n) => typeof n === 'string'),
    ).toBe(true);
  });
});
