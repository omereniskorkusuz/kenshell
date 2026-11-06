import { describe, expect, it } from 'vitest';
import rules from '../data/config-rules.json';
import en from '../i18n/en.json';
import tr from '../i18n/tr.json';

describe('config rules data (P4-T07)', () => {
  it('every rule id has TR and EN text', () => {
    const ids = [...new Set(rules.rules.map((r) => r.id))];
    const enRules = en.cfgRule as Record<string, string>;
    const trRules = tr.cfgRule as Record<string, string>;
    for (const id of ids) {
      expect(enRules[id], id).toBeTruthy();
      expect(trRules[id], id).toBeTruthy();
    }
    expect(Object.keys(enRules).sort()).toEqual(ids.sort());
  });
});
