import { describe, expect, it } from 'vitest';
import type { DeviceInfo, DeviceStorageInfo, Transfers } from './api';
import { eta, GrowthRates, rowActivity, summarize } from './activity';

const dev = (id: string, name: string, role: string, extra: Partial<DeviceInfo> = {}): DeviceInfo => ({
  id, name, role, online: true, is_this_device: false, admin: false, revoked: false, path: 'lan', latency_ms: 3, ...extra,
});
const phone = dev('p', 'S25', 'companion', { is_this_device: true });
const nest = dev('n', 'nest', 'keeper', { admin: true });
const mba = dev('m', 'MBA', 'companion');
const devices = [phone, nest, mba];
const store = (id: string, originals: number): DeviceStorageInfo => ({ id, originals, original_bytes: originals * 1e7, other_bytes: 0 });
const idle: Transfers = {
  downloading: [], sending: [], download_rate: 0, upload_rate: 0,
  waiting_thumbs: 0, waiting_previews: 0, waiting_originals: 0, waiting_bytes: 0, paused: null,
};
const sendingToNest: Transfers = {
  ...idle,
  upload_rate: 24e6,
  sending: [1, 2, 3].map((i) => ({ photo: `x${i}`, kind: 'original', peer: 'n', size: 6e6, done: 1e6 })),
};

describe('the activity card', () => {
  it('says how many originals a keeper still needs, how far along and how long', () => {
    const s = summarize(sendingToNest, devices, [store('p', 6114), store('n', 2574)], 6114, { n: 0.75 });
    expect(s.tone).toBe('busy');
    expect(s.head).toBe('Backing up 3,540 originals to nest');
    expect(s.sub).toBe('2,574 of 6,114 · about 1 h 19 min left · ↑ 24.0 MB/s');
    expect(s.progress).toBeCloseTo(2574 / 6114);
  });

  it('waits for an offline keeper instead of promising progress', () => {
    const s = summarize(idle, [phone, { ...nest, online: false }], [store('n', 10)], 20, {});
    expect(s.tone).toBe('paused');
    expect(s.head).toBe('Waiting for nest · 10 originals not on it yet');
  });

  it('says why this device is paused', () => {
    const s = summarize({ ...idle, paused: 'is on mobile data' }, devices, [store('n', 10)], 20, {});
    expect(s.tone).toBe('paused');
    expect(s.sub).toBe('This device is on mobile data. It carries on by itself.');
  });

  it('is one green line when every keeper has everything', () => {
    const s = summarize(idle, devices, [store('n', 20)], 20, {});
    expect(s).toEqual({ tone: 'idle', head: 'Everything is backed up', sub: 'Every original is on nest. Nothing waiting.', progress: null });
  });
});

describe('a device row', () => {
  it('shows what this device sends and to whom', () => {
    expect(rowActivity(phone, sendingToNest, devices, [], 0, {}).text).toBe('↑ Sending 3 to nest · 24.0 MB/s');
  });

  it('shows the keeper getting them, with what is left', () => {
    const r = rowActivity(nest, sendingToNest, devices, [store('n', 2574)], 6114, { n: 0.75 });
    expect(r.text).toBe('↓ Getting 3 from S25 · 3,540 originals to go');
    expect(r.detail).toBe('about 1 h 19 min left');
  });

  it('says a keeper that has everything has every original, and an idle companion idle', () => {
    expect(rowActivity(nest, idle, devices, [store('n', 5)], 5, {}).text).toBe('Has every original');
    expect(rowActivity(mba, idle, devices, [], 5, {}).text).toBe('Idle');
    expect(rowActivity({ ...mba, online: false }, idle, devices, [], 5, {}).text).toBe('Offline');
  });
});

describe('time left and growth', () => {
  it('rounds to what a person reads', () => {
    expect(eta(20)).toBe('under a minute');
    expect(eta(4 * 60)).toBe('about 4 min');
    expect(eta(2 * 3600)).toBe('about 2 h');
    expect(eta(Infinity)).toBe('');
  });

  it('measures growth only once there is enough of it', () => {
    const g = new GrowthRates();
    g.add([store('n', 100)], 0);
    g.add([store('n', 101)], 5_000);
    expect(g.rates().n).toBe(0);
    g.add([store('n', 130)], 30_000);
    expect(g.rates().n).toBeCloseTo(1);
  });
});
