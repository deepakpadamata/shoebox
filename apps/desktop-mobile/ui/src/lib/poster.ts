// Poster frames for videos (M3-10). The library has no poster for a video, so each device makes
// its own from the video it can play: load the original (byte ranges, so only what the decoder
// needs), grab a frame, hand it to the app core as a small JPEG. One at a time, so a screenful of
// videos does not stall the grid.
import { api, videoUrl } from './api';

const MAX_SIDE = 480;
const inFlight = new Map<string, Promise<boolean>>();
let tail: Promise<unknown> = Promise.resolve();

function once<T>(target: EventTarget, ok: string, timeoutMs: number): Promise<T | void> {
  return new Promise((resolve, reject) => {
    const t = setTimeout(() => reject(new Error('timed out')), timeoutMs);
    target.addEventListener(ok, () => { clearTimeout(t); resolve(); }, { once: true });
    target.addEventListener('error', () => { clearTimeout(t); reject(new Error('cannot decode this video')); }, { once: true });
  });
}

async function grab(id: string): Promise<boolean> {
  const video = document.createElement('video');
  video.muted = true;
  video.preload = 'auto';
  video.crossOrigin = 'anonymous';
  // The same source the viewer plays from (on Android, `shoebox://` cannot serve a long video).
  video.src = videoUrl(id);
  try {
    await once(video, 'loadeddata', 10_000);
    const t = Math.min(1, (video.duration || 1) * 0.1);
    if (t > 0) {
      video.currentTime = t;
      await once(video, 'seeked', 10_000);
    }
    const scale = Math.min(1, MAX_SIDE / Math.max(video.videoWidth, video.videoHeight, 1));
    const canvas = document.createElement('canvas');
    canvas.width = Math.max(1, Math.round(video.videoWidth * scale));
    canvas.height = Math.max(1, Math.round(video.videoHeight * scale));
    canvas.getContext('2d')!.drawImage(video, 0, 0, canvas.width, canvas.height);
    const blob: Blob | null = await new Promise((r) => canvas.toBlob(r, 'image/jpeg', 0.8));
    if (!blob) return false;
    const bytes = new Uint8Array(await blob.arrayBuffer());
    let bin = '';
    for (let i = 0; i < bytes.length; i += 0x8000) bin += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
    await api.setPoster(id, btoa(bin));
    return true;
  } catch {
    return false;
  } finally {
    video.removeAttribute('src');
    video.load();
  }
}

/** Make the poster of video `id` (once; later calls share the result). Resolves to success. */
export function ensurePoster(id: string): Promise<boolean> {
  let p = inFlight.get(id);
  if (!p) {
    p = tail.then(() => grab(id));
    tail = p.catch(() => {});
    inFlight.set(id, p);
    // A failure may be retried later (the original may arrive), a success never needs repeating.
    p.then((ok) => { if (!ok) inFlight.delete(id); });
  }
  return p;
}
