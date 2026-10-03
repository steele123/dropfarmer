import { describe, expect, it } from 'vitest';
import { historyCsv } from './historyExport';
import { previewState } from './model';
import { rewardHistory } from './analytics';

describe('history CSV export', () => {
  it('preserves Unicode, commas, quotes, and multiline names with a UTF-8 BOM', () => {
    const r = previewState().analytics.rewards[0];
    r.name = 'Épée, "rare"\nEdition';
    const csv = historyCsv([r]);
    expect(csv.startsWith('\uFEFF"Reward","Game"')).toBe(true);
    expect(csv).toContain('"Épée, ""rare""\nEdition"');
    expect(csv).toContain('"Claimed"');
    expect(csv).toContain(`"${r.recordedAt}"`);
    expect(csv.endsWith('\r\n')).toBe(true);
  });
  it('prevents spreadsheet formulas in imported names', () => {
    const r = previewState().analytics.rewards[0];
    for (const name of [
      '=1+1',
      '+SUM(A1)',
      '-2+3',
      '@SUM(A1)',
      '  =1+1',
      '\tvalue',
    ]) {
      expect(historyCsv([{ ...r, name }])).toContain(`"'${name}"`);
    }
  });
  it('keeps the original recorded date alongside the Twitch award date', () => {
    const reward = {
      ...previewState().analytics.rewards[0],
      awardedAt: '2026-10-02T12:00:00Z',
    };
    const csv = historyCsv([reward]);
    expect(csv).toContain('"Awarded by Twitch (UTC)"');
    expect(csv).toContain(`"${reward.recordedAt}"`);
    expect(csv).toContain(`"${reward.awardedAt}"`);
  });
  it('exports every matching reward rather than a page of history', () => {
    const rewards = previewState().analytics.rewards;
    const matching = rewardHistory(
      Array.from({ length: 80 }, (_, i) => ({ ...rewards[0], id: String(i) })),
      '',
      'Rust',
    );
    expect(historyCsv(matching).split('\r\n')).toHaveLength(82);
    expect(historyCsv([]).split('\r\n')).toHaveLength(2);
  });
});
