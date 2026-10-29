// Poster frames for videos (M3-10). A video without one in the library gets it from the first
// device that shows it and can play it: load the original (byte ranges, so only what the decoder
// needs), grab a frame, hand it to the app core as a small JPEG, which keeps it in the library for
// every device (phones usually make theirs natively during backup). A couple at a time, newest
// request first, and only for tiles still on screen, so scrolling never leaves a backlog.
import { api, videoUrl } from './api';

const MAX_SIDE = 480;
const WORKERS = 2;
// Darker than this on average is a fade-in, not the picture: try further in.
const DARK = 16;

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
    const scale = Math.min(1, MAX_SIDE / Math.max(video.videoWidth, video.videoHeight, 1));
    const canvas = document.createElement('canvas');
    canvas.width = Math.max(1, Math.round(video.videoWidth * scale));
    canvas.height = Math.max(1, Math.round(video.videoHeight * scale));
    const ctx = canvas.getContext('2d', { willReadFrequently: true })!;
    const duration = video.duration || 1;
    for (const at of [Math.min(1, duration * 0.1), duration * 0.33, duration * 0.5]) {
      if (at > 0) {
        video.currentTime = at;
        await once(video, 'seeked', 10_000);
      }
      ctx.drawImage(video, 0, 0, canvas.width, canvas.height);
      if (brightness(ctx, canvas.width, canvas.height) >= DARK) break;
    }
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

/** Mean brightness (0–255) of the frame, from a few sampled pixels. */
function brightness(ctx: CanvasRenderingContext2D, w: number, h: number): number {
  const d = ctx.getImageData(0, 0, w, h).data;
  const step = Math.max(4, Math.floor(d.length / 4 / 400) * 4);
  let sum = 0;
  let n = 0;
  for (let i = 0; i < d.length; i += step) {
    sum += d[i] + d[i + 1] + d[i + 2];
    n += 3;
  }
  return n ? sum / n : 0;
}

interface Request { id: string; done: (ok: boolean) => void; wanted: number }
const waiting = new Map<string, Request>();
const results = new Map<string, Promise<boolean>>();
let running = 0;

function pump() {
  while (running < WORKERS) {
    // The newest request still on screen goes first.
    const next = [...waiting.values()].reverse().find((r) => r.wanted > 0);
    if (!next) return;
    waiting.delete(next.id);
    running++;
    grab(next.id).then((ok) => {
      running--;
      // A failure may be retried later (the original may arrive), a success never needs repeating.
      if (!ok) results.delete(next.id);
      next.done(ok);
      pump();
    });
  }
}

/**
 * Make the poster of video `id` while its tile is on screen. Resolves to success (or `false` if
 * the tile left before its turn); call the returned function when the tile goes away.
 */
export function ensurePoster(id: string): { result: Promise<boolean>; release: () => void } {
  const queued = waiting.get(id);
  if (queued) {
    queued.wanted++;
    waiting.delete(id);
    waiting.set(id, queued); // now the newest
  } else if (!results.has(id)) {
    results.set(id, new Promise<boolean>((done) => waiting.set(id, { id, done, wanted: 1 })));
  }
  const result = results.get(id)!;
  let released = false;
  const release = () => {
    if (released) return;
    released = true;
    const r = waiting.get(id);
    if (r && --r.wanted === 0) {
      // Scrolled away before its turn: forget it, the next tile showing it asks again.
      waiting.delete(id);
      results.delete(id);
      r.done(false);
    }
  };
  queueMicrotask(pump);
  return { result, release };
}
