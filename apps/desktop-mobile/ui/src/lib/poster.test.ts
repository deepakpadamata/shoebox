import { beforeEach, describe, expect, it, vi } from 'vitest';

const makePoster = vi.fn(async (_id: string) => true);
vi.mock('./api', () => ({ api: { makePoster: (id: string) => makePoster(id) }, playsNatively: () => true, videoUrl: () => '' }));

describe('ensurePoster', () => {
  beforeEach(() => {
    vi.resetModules();
    makePoster.mockClear();
  });

  it('does not make a poster again soon after making it', async () => {
    const { ensurePoster } = await import('./poster');
    expect(await ensurePoster('a').result).toBe(true);
    expect(await ensurePoster('a').result).toBe(true);
    expect(makePoster).toHaveBeenCalledTimes(1);
  });

  it('remembers only the recent ones', async () => {
    const { ensurePoster } = await import('./poster');
    for (let i = 0; i < 501; i++) await ensurePoster(`v${i}`).result;
    expect(makePoster).toHaveBeenCalledTimes(501);
    await ensurePoster('v500').result; // recent: remembered
    expect(makePoster).toHaveBeenCalledTimes(501);
    await ensurePoster('v0').result; // the oldest was forgotten
    expect(makePoster).toHaveBeenCalledTimes(502);
  });

  it('tries a failed one again', async () => {
    makePoster.mockResolvedValueOnce(false);
    const { ensurePoster } = await import('./poster');
    expect(await ensurePoster('b').result).toBe(false);
    expect(await ensurePoster('b').result).toBe(true);
    expect(makePoster).toHaveBeenCalledTimes(2);
  });
});
