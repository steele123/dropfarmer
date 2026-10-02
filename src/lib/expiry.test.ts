import { describe, expect, it } from 'vitest';
import { expiryWarnings } from './expiry';
import { previewState, type Campaign, type Drop } from './model';

const now = Date.parse('2026-10-02T12:00:00Z');
const at = (minutes: number) => new Date(now + minutes * 60000).toISOString();
function campaign(): Campaign {
  const c = previewState().campaigns[0];
  c.startsAt = at(-1440);
  c.endsAt = at(3000);
  c.drops = [
    {
      ...c.drops[0],
      id: 'a',
      required: 120,
      minutes: 0,
      startsAt: at(-1440),
      endsAt: at(3000),
    },
  ];
  return c;
}
const reward = (c: Campaign, fields: Partial<Drop>): Drop => ({
  ...c.drops[0],
  ...fields,
});

describe('reward expiry warnings', () => {
  it('uses the earlier reward or campaign deadline and ignores claimed rewards', () => {
    const c = campaign();
    expect(expiryWarnings(c, now)).toEqual([]);
    c.drops[0].endsAt = at(300);
    expect(expiryWarnings(c, now)[0].level).toBe('soon');
    expect(expiryWarnings(c, now)[0].deadline).toBe(Date.parse(at(300)));
    c.endsAt = at(60);
    expect(expiryWarnings(c, now)[0].level).toBe('insufficient');
    c.drops[0].claimed = true;
    expect(expiryWarnings(c, now)).toEqual([]);
  });
  it('uses remaining progress and does not add independent reward times together', () => {
    const c = campaign();
    c.endsAt = at(150);
    c.drops.push(reward(c, { id: 'b' }));
    expect(expiryWarnings(c, now).every((w) => w.level === 'soon')).toBe(true);
    c.endsAt = at(60);
    c.drops[0].minutes = 70;
    expect(expiryWarnings(c, now).find((w) => w.dropId === 'a')?.level).toBe(
      'soon',
    );
    expect(expiryWarnings(c, now).find((w) => w.dropId === 'b')?.level).toBe(
      'insufficient',
    );
  });
  it('includes prerequisite watch time and future start times', () => {
    const c = campaign();
    c.endsAt = at(180);
    c.drops.push(reward(c, { id: 'b', prerequisites: ['a'] }));
    expect(expiryWarnings(c, now).find((w) => w.dropId === 'b')?.level).toBe(
      'insufficient',
    );
    c.drops[0].claimed = true;
    expect(expiryWarnings(c, now)[0].level).toBe('soon');
    c.drops[1].startsAt = at(100);
    expect(expiryWarnings(c, now)[0].level).toBe('insufficient');
  });
  it('keeps earned rewards claimable and labels expired windows without inventing claim deadlines', () => {
    const c = campaign();
    c.endsAt = at(20);
    c.drops[0].minutes = 120;
    expect(expiryWarnings(c, now)[0].detail).toContain('claim');
    expect(expiryWarnings(c, now)[0].level).toBe('soon');
    c.endsAt = at(-1);
    expect(expiryWarnings(c, now)[0].level).toBe('ended');
    expect(expiryWarnings(c, now)[0].detail).toContain('claim availability');
  });
  it('handles unavailable dates, missing prerequisites and cycles without inventing estimates', () => {
    const c = campaign();
    c.endsAt = at(60);
    c.drops[0].prerequisites = ['missing'];
    expect(expiryWarnings(c, now)[0].level).toBe('soon');
    c.drops[0].prerequisites = ['b'];
    c.drops.push(reward(c, { id: 'b', prerequisites: ['a'] }));
    expect(expiryWarnings(c, now).every((w) => w.level === 'soon')).toBe(true);
    c.endsAt = 'invalid';
    expect(expiryWarnings(c, now)).toEqual([]);
  });
});
