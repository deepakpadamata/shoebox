// The words for the backup status (U1-07/08), in one place so the Photos chip, the sidebar row,
// the banners, the Backup page and the viewer say the same thing about the same library.
import type { SafetyStatus } from '../bindings/SafetyStatus';
import { plural } from './format';

export type Tone = 'ok' | 'neutral' | 'info' | 'warn' | 'risk';

export interface SafetyLine {
  tone: Tone;
  /** The chip and the sidebar row. */
  short: string;
  /** The hero sentence on the Backup page, and the banner when there is one. */
  long: string;
  /** A banner is shown for this state (the calm states have none). */
  banner: boolean;
}

/** `where`: what to call this device when it is the only one ("phone" or "computer"). */
export function safetyLine(s: SafetyStatus | null, where: 'phone' | 'computer' = 'phone'): SafetyLine {
  if (!s) return { tone: 'neutral', short: 'Checking backup', long: 'Checking your backup…', banner: false };
  const n = s.count;
  const gone = s.reason ? s.reason : 'another device';
  switch (s.state) {
    case 'safe':
      return { tone: 'ok', short: 'All backed up', long: `All ${plural(s.count, 'photo')} are safe on ${s.devices} devices.`, banner: false };
    case 'single_device':
      return {
        tone: 'neutral',
        short: `Only on this ${where}`,
        long: `Your photos are only on this ${where}. Add a device to keep a second copy.`,
        banner: false,
      };
    case 'busy':
      return { tone: 'info', short: n > 0 ? `Backing up ${n}` : 'Backing up', long: n > 0 ? `Backing up ${plural(n, 'photo')} to your other devices.` : 'Copying to your other devices.', banner: false };
    case 'waiting':
      return {
        tone: 'warn',
        short: `${n} waiting for ${gone}`,
        long: `${plural(n, 'photo')} will be backed up when ${gone} is reachable${since(s)}.`,
        banner: true,
      };
    default:
      return {
        tone: 'risk',
        short: `${n} at risk`,
        long: `${plural(n, 'photo')} ${n === 1 ? 'is' : 'are'} only on this device, and ${gone} has not been seen${since(s, true)}. Open Shoebox on it to back them up.`,
        banner: true,
      };
  }
}

function since(s: SafetyStatus, strong = false): string {
  if (s.since_secs == null) return '';
  const d = Math.floor(s.since_secs / 86400);
  const h = Math.floor(s.since_secs / 3600);
  const t = d >= 1 ? plural(d, 'day') : h >= 1 ? plural(h, 'hour') : '';
  if (!t) return '';
  return strong ? ` for ${t}` : ` (last seen ${t} ago)`;
}
