// What the Devices screen says is moving (mockup 07): the card at the top and each device's
// "right now" line, from this device's transfers, the devices' storage and how fast each keeper's
// count of originals has been growing.
import type { DeviceInfo, DeviceStorageInfo, Transfers } from './api';
import { bytes, plural } from './format';

export type Tone = 'busy' | 'paused' | 'idle';

export interface Summary {
  tone: Tone;
  head: string;
  sub: string;
  /** 0–1 for the bar, or null for none. */
  progress: number | null;
}

export interface RowActivity {
  /** `up` sending, `down` getting, `warn` waiting on something, `ok` done, `idle` nothing. */
  tone: 'up' | 'down' | 'warn' | 'ok' | 'idle';
  text: string;
  /** Under the text: how long is left, if known. */
  detail?: string;
}

export function speed(perSecond: number): string {
  return `${bytes(Math.round(perSecond))}/s`;
}

/** "about 1 h 20 min", "about 4 min", "under a minute". */
export function eta(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds <= 0) return '';
  const mins = Math.round(seconds / 60);
  if (mins < 1) return 'under a minute';
  if (mins < 60) return `about ${mins} min`;
  const h = Math.floor(mins / 60);
  const m = mins % 60;
  return m ? `about ${h} h ${m} min` : `about ${h} h`;
}

/** Keepers that do not hold every original yet, most behind first. */
export function keepersBehind(devices: DeviceInfo[], storage: DeviceStorageInfo[], total: number) {
  return devices
    .filter((d) => !d.revoked && d.role === 'keeper')
    .map((d) => {
      const held = storage.find((s) => s.id === d.id)?.originals ?? 0;
      return { d, held, missing: Math.max(0, total - held) };
    })
    .filter((k) => k.missing > 0)
    .sort((a, b) => b.missing - a.missing);
}

const nameOf = (devices: DeviceInfo[], id: string | null) =>
  devices.find((d) => d.id === id)?.name ?? 'another device';

function names(list: string[]): string {
  if (list.length <= 1) return list[0] ?? '';
  return `${list.slice(0, -1).join(', ')} and ${list[list.length - 1]}`;
}

function waiting(t: Transfers): number {
  return t.waiting_thumbs + t.waiting_previews + t.waiting_originals;
}

/**
 * The card at the top. `rates`: originals per second each device's count has been growing by
 * (device id → rate), measured by the screen while it is open.
 */
export function summarize(
  t: Transfers | null,
  devices: DeviceInfo[],
  storage: DeviceStorageInfo[],
  total: number,
  rates: Record<string, number>,
): Summary {
  const behind = keepersBehind(devices, storage, total);
  const rate = (dir: 'down' | 'up') => {
    if (!t) return '';
    const r = dir === 'down' ? t.download_rate : t.upload_rate;
    return r > 0 ? `${dir === 'down' ? '↓' : '↑'} ${speed(r)}` : '';
  };
  const speeds = [rate('down'), rate('up')].filter(Boolean).join(' · ');
  const keepers = devices.filter((d) => !d.revoked && d.role === 'keeper');

  if (t?.paused) {
    const what = behind[0] ? `${plural(behind[0].missing, 'original')} not on ${behind[0].d.name} yet` : 'nothing is fetched';
    return { tone: 'paused', head: `Paused · ${what}`, sub: `This device ${t.paused}. It carries on by itself.`, progress: null };
  }
  if (behind.length) {
    const k = behind[0];
    if (!k.d.online) {
      return { tone: 'paused', head: `Waiting for ${k.d.name} · ${plural(k.missing, 'original')} not on it yet`, sub: `${k.d.name} is offline. Backing up carries on when it is back.`, progress: total ? k.held / total : null };
    }
    const left = eta(k.missing / (rates[k.d.id] ?? 0));
    const sub = [`${k.held.toLocaleString('en')} of ${total.toLocaleString('en')}`, left && `${left} left`, speeds].filter(Boolean).join(' · ');
    return { tone: 'busy', head: `Backing up ${plural(k.missing, 'original')} to ${k.d.name}`, sub, progress: total ? k.held / total : null };
  }
  if (t && (t.downloading.length || waiting(t))) {
    const n = Math.max(waiting(t), t.downloading.length);
    const sub = [t.waiting_bytes ? `${bytes(t.waiting_bytes)} to go` : '', speeds].filter(Boolean).join(' · ');
    return { tone: 'busy', head: `Getting ${plural(n, 'item')} for this device`, sub, progress: null };
  }
  if (t && t.sending.length) {
    const to = [...new Set(t.sending.map((s) => nameOf(devices, s.peer)))];
    return { tone: 'busy', head: `Sending to ${names(to)}`, sub: speeds, progress: null };
  }
  if (keepers.length) {
    return { tone: 'idle', head: 'Everything is backed up', sub: `Every original is on ${names(keepers.map((k) => k.name))}. Nothing waiting.`, progress: null };
  }
  return { tone: 'idle', head: 'Up to date', sub: 'Nothing waiting. Add a keeper to have every original in one more place.', progress: null };
}

/** The "right now" line of one device, as this device sees it. */
export function rowActivity(
  d: DeviceInfo,
  t: Transfers | null,
  devices: DeviceInfo[],
  storage: DeviceStorageInfo[],
  total: number,
  rates: Record<string, number>,
): RowActivity {
  if (!t) return { tone: 'idle', text: '' };
  const me = devices.find((x) => x.is_this_device);
  if (d.is_this_device) {
    if (t.paused) return { tone: 'warn', text: `Paused: ${t.paused}` };
    const parts: RowActivity[] = [];
    if (t.downloading.length) parts.push({ tone: 'down', text: `↓ Getting ${t.downloading.length}${t.download_rate ? ` · ${speed(t.download_rate)}` : ''}` });
    if (t.sending.length) {
      const to = [...new Set(t.sending.map((s) => nameOf(devices, s.peer)))];
      parts.push({ tone: 'up', text: `↑ Sending ${t.sending.length} to ${names(to)}${t.upload_rate ? ` · ${speed(t.upload_rate)}` : ''}` });
    }
    if (parts.length) return { tone: parts[0].tone, text: parts.map((p) => p.text).join(' · ') };
    const w = waiting(t);
    return w ? { tone: 'warn', text: `${plural(w, 'item')} waiting` } : { tone: 'idle', text: 'Idle · has what it keeps' };
  }
  if (!d.online) return { tone: 'idle', text: 'Offline' };
  const toIt = t.sending.filter((s) => s.peer === d.id).length;
  const fromIt = t.downloading.filter((s) => s.peer === d.id).length;
  const k = d.role === 'keeper' ? keepersBehind(devices, storage, total).find((x) => x.d.id === d.id) : undefined;
  if (toIt) {
    const left = k ? eta(k.missing / (rates[d.id] ?? 0)) : '';
    return { tone: 'down', text: `↓ Getting ${toIt} from ${me?.name ?? 'this device'}${k ? ` · ${plural(k.missing, 'original')} to go` : ''}`, detail: left ? `${left} left` : undefined };
  }
  if (fromIt) return { tone: 'up', text: `↑ Sending ${fromIt} to ${me?.name ?? 'this device'}` };
  if (k) {
    const left = eta(k.missing / (rates[d.id] ?? 0));
    return { tone: 'down', text: `${plural(k.missing, 'original')} to go`, detail: left ? `${left} left` : undefined };
  }
  if (d.role === 'keeper') return { tone: 'ok', text: 'Has every original' };
  return { tone: 'idle', text: 'Idle' };
}

/** Growth of each device's originals count, from samples the screen takes while it is open. */
export class GrowthRates {
  private samples = new Map<string, Array<[number, number]>>();
  constructor(private windowMs = 3 * 60_000) {}

  add(storage: DeviceStorageInfo[], now = Date.now()) {
    for (const s of storage) {
      const list = this.samples.get(s.id) ?? [];
      list.push([now, s.originals]);
      while (list.length > 1 && now - list[0][0] > this.windowMs) list.shift();
      this.samples.set(s.id, list);
    }
  }

  /** Originals per second over the window; 0 until there are two samples 10 s apart. */
  rates(): Record<string, number> {
    const out: Record<string, number> = {};
    for (const [id, list] of this.samples) {
      const [t0, n0] = list[0];
      const [t1, n1] = list[list.length - 1];
      out[id] = t1 - t0 >= 10_000 && n1 > n0 ? ((n1 - n0) * 1000) / (t1 - t0) : 0;
    }
    return out;
  }
}
