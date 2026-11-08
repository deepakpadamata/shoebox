import { describe, expect, it } from 'vitest';
import { safetyLine } from './safety';

const st = (o: Partial<Parameters<typeof safetyLine>[0] & object>) => ({ state: 'safe', count: 0, devices: 2, since_secs: null, reason: null, ...o }) as never;

describe('safetyLine', () => {
  it('says the same fixed words for the five states', () => {
    expect(safetyLine(st({ state: 'safe', count: 100 })).short).toBe('All backed up');
    expect(safetyLine(st({ state: 'busy', count: 23 })).short).toBe('Backing up 23');
    expect(safetyLine(st({ state: 'waiting', count: 214, reason: 'Mac Studio' })).short).toBe('214 waiting for Mac Studio');
    expect(safetyLine(st({ state: 'at_risk', count: 9, reason: 'Mac Studio' })).short).toBe('9 at risk');
    expect(safetyLine(st({ state: 'single_device', devices: 1 })).short).toBe('Only on this phone');
    expect(safetyLine(st({ state: 'single_device', devices: 1 }), 'computer').short).toBe('Only on this computer');
  });
  it('is never "backed up" for a single device', () => {
    const l = safetyLine(st({ state: 'single_device', devices: 1, count: 4 }));
    expect(l.short + l.long).not.toMatch(/backed up|copies/i);
  });
  it('names the device and how long, and raises a banner only when something waits', () => {
    const w = safetyLine(st({ state: 'waiting', count: 3, reason: 'Mac', since_secs: 90000 }));
    expect(w.long).toContain('Mac');
    expect(w.long).toContain('1 day');
    expect(w.banner).toBe(true);
    expect(safetyLine(st({ state: 'at_risk', count: 1, reason: 'Mac', since_secs: 8 * 86400 })).tone).toBe('risk');
    expect(safetyLine(st({ state: 'safe' })).banner).toBe(false);
  });
});
