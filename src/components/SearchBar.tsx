/**
 * Scrollback search (Cmd/Ctrl+F): plain or regex, case toggle, Enter = next,
 * Shift+Enter = previous, Esc = close. Lives over the terminal's top-right corner.
 */
import { ChevronDown, ChevronUp, X } from 'lucide-react';
import { useEffect, useRef, useState } from 'react';
import { useT } from '../i18n';
import { getTerminal, type SearchOptions } from '../terminal/registry';
import { useUi } from '../stores/ui';

export function SearchBar({ tabKey }: { tabKey: string }) {
  const t = useT();
  const [query, setQuery] = useState('');
  const [opts, setOpts] = useState<SearchOptions>({ regex: false, caseSensitive: false });
  const [missing, setMissing] = useState(false);
  const input = useRef<HTMLInputElement>(null);

  useEffect(() => {
    input.current?.focus();
    input.current?.select();
  }, []);

  const validRegex = (() => {
    if (!opts.regex || !query) return true;
    try {
      new RegExp(query);
      return true;
    } catch {
      return false;
    }
  })();

  const run = (back: boolean) => {
    const term = getTerminal(tabKey);
    if (!term || !query || !validRegex) return;
    const found = back ? term.findPrevious(query, opts) : term.findNext(query, opts);
    setMissing(!found);
  };

  const close = () => {
    getTerminal(tabKey)?.clearSearch();
    useUi.getState().closeFind();
    getTerminal(tabKey)?.focus();
  };

  const toggle = (k: keyof SearchOptions) => setOpts((o) => ({ ...o, [k]: !o[k] }));

  return (
    <div
      role="search"
      className="absolute right-3 top-2 z-10 flex items-center gap-1 rounded-md border border-subtle bg-elevated p-1 text-sm shadow-[var(--shadow-elevated)]"
    >
      <input
        ref={input}
        className="field mono h-7 w-56"
        placeholder={t('search.placeholder')}
        aria-label={t('search.placeholder')}
        aria-invalid={!validRegex || missing}
        value={query}
        spellCheck={false}
        onChange={(e) => {
          setQuery(e.target.value);
          setMissing(false);
        }}
        onKeyDown={(e) => {
          if (e.key === 'Enter') {
            e.preventDefault();
            run(e.shiftKey);
          } else if (e.key === 'Escape') {
            e.preventDefault();
            close();
          }
        }}
      />
      <button
        type="button"
        className="btn btn-sm mono"
        aria-pressed={opts.caseSensitive}
        title={t('search.caseSensitive')}
        onClick={() => toggle('caseSensitive')}
      >
        Aa
      </button>
      <button
        type="button"
        className="btn btn-sm mono"
        aria-pressed={opts.regex}
        title={t('search.regex')}
        onClick={() => toggle('regex')}
      >
        .*
      </button>
      <button
        type="button"
        className="icon-btn"
        title={t('search.previous')}
        aria-label={t('search.previous')}
        onClick={() => run(true)}
      >
        <ChevronUp size={14} />
      </button>
      <button
        type="button"
        className="icon-btn"
        title={t('search.next')}
        aria-label={t('search.next')}
        onClick={() => run(false)}
      >
        <ChevronDown size={14} />
      </button>
      <span className="min-w-14 px-1 text-xs text-muted" aria-live="polite">
        {!validRegex ? t('search.badRegex') : missing ? t('search.notFound') : ''}
      </span>
      <button
        type="button"
        className="icon-btn"
        title={t('search.close')}
        aria-label={t('search.close')}
        onClick={close}
      >
        <X size={14} />
      </button>
    </div>
  );
}
