// The viewer for a Shoebox web link. Everything it shows is decrypted here: the page fetches ciphertext from the
// service named by the address, and the key comes from the part of the address after "#", which no browser sends.

import init, * as core from './pkg/shoebox_web.js';
import { zipWriter, ZIP_FILE_LIMIT } from './zip.js';

const $ = (id) => document.getElementById(id);
const statusEl = $('status');
const THUMB_CONCURRENCY = 6;
const MAX_PLAY_BYTES = 100 * 1024 * 1024;
const ZIP_IN_MEMORY_LIMIT = 1.5 * 1024 * 1024 * 1024;

let share = null; // { id, key, manifest, blob(cid) }
let items = [];
const urls = new Map(); // cid -> object URL of the decrypted blob
let current = -1;
let lastFocus = null;

const fromHex = (h) => Uint8Array.from(h.match(/../g) ?? [], (b) => parseInt(b, 16));

function say(headline, text, isError = true) {
  statusEl.className = isError ? 'status error' : 'status';
  statusEl.textContent = '';
  if (headline) {
    const s = document.createElement('strong');
    s.textContent = headline;
    statusEl.append(s);
  }
  if (text) statusEl.append(document.createTextNode(text));
}

function sniff(bytes) {
  if (bytes[0] === 0xff && bytes[1] === 0xd8) return ['image/jpeg', 'jpg'];
  if (bytes[0] === 0x89 && bytes[1] === 0x50 && bytes[2] === 0x4e) return ['image/png', 'png'];
  if (bytes[0] === 0x47 && bytes[1] === 0x49 && bytes[2] === 0x46) return ['image/gif', 'gif'];
  if (bytes[0] === 0x52 && bytes[1] === 0x49 && bytes[8] === 0x57 && bytes[9] === 0x45) return ['image/webp', 'webp'];
  if (bytes[4] === 0x66 && bytes[5] === 0x74 && bytes[6] === 0x79 && bytes[7] === 0x70) {
    const brand = String.fromCharCode(...bytes.slice(8, 12));
    return brand === 'qt  ' ? ['video/quicktime', 'mov'] : ['video/mp4', 'mp4'];
  }
  if (bytes[0] === 0x1a && bytes[1] === 0x45 && bytes[2] === 0xdf && bytes[3] === 0xa3) return ['video/webm', 'mkv'];
  return ['application/octet-stream', 'bin'];
}

class ServiceError extends Error {
  constructor(status) {
    super(`the service answered ${status}`);
    this.status = status;
  }
}

/** Thrown when none of a Live link's devices answers: it is offline, which is not an error to be alarmed by. */
class Offline extends Error {}

async function get(path, init) {
  let res;
  try {
    res = await fetch(path, { credentials: 'omit', referrerPolicy: 'no-referrer', ...init });
  } catch {
    throw new ServiceError(0);
  }
  if (!res.ok) throw new ServiceError(res.status);
  return new Uint8Array(await res.arrayBuffer());
}

// ---- Live links: the same sealed bytes, asked of a device over iroh instead of the service ----

const LIVE_PIECE = 4 * 1024 * 1024 + 16;

function liveError(e) {
  const m = String(e?.message ?? e);
  if (m === 'gone') return new ServiceError(404);
  if (m === 'expired') return new ServiceError(410);
  if (m === 'busy') return new ServiceError(429);
  return new Offline(m);
}

async function connectLive(serversHex) {
  // Loaded only for Live links: the Cloud path never fetches it.
  const mod = await import('./pkg-live/shoebox_web_live.js');
  await mod.default();
  try {
    return await mod.Live.connect(fromHex(serversHex));
  } catch (e) {
    throw liveError(e);
  }
}

function liveTransport(conn, idHex) {
  const id = fromHex(idHex);
  return {
    async manifest() {
      try {
        return await conn.manifest(id);
      } catch (e) {
        throw liveError(e);
      }
    },
    async blob(cid) {
      const c = fromHex(cid);
      let out = null;
      let have = 0;
      try {
        for (;;) {
          const r = await conn.blob_range(id, c, have, LIVE_PIECE);
          const total = Number(new DataView(r.buffer, r.byteOffset, 8).getBigUint64(0, true));
          const part = r.subarray(8);
          out ??= new Uint8Array(total);
          out.set(part, have);
          have += part.length;
          if (have >= total) return out;
          if (part.length === 0) throw new Error('unreachable');
        }
      } catch (e) {
        throw liveError(e);
      }
    },
  };
}

/** Fetch, check against its id, and open one blob. */
async function openBlob(ref) {
  const sealed = await share.blob(ref.cid);
  if (core.cid_of(sealed) !== ref.cid) throw new Error('a blob did not match its id');
  return core.decrypt_blob(fromHex(ref.key), sealed);
}

async function blobUrl(ref) {
  const known = urls.get(ref.cid);
  if (known) return known;
  const bytes = await openBlob(ref);
  const [type] = sniff(bytes);
  const url = URL.createObjectURL(new Blob([bytes], { type }));
  urls.set(ref.cid, url);
  return url;
}

function problem(e) {
  if (e instanceof Offline) return ['This album is not available right now', 'It is shared from a device that is switched off or offline. Try again later, or ask the person who sent it.'];
  if (e instanceof ServiceError) {
    if (e.status === 404) return ['This link has stopped working', 'The person who sent it may have stopped sharing, or the link was copied incompletely.'];
    if (e.status === 410) return ['This link has expired', 'Ask the person who sent it for a new one.'];
    if (e.status === 429) return ['This link is busy', 'It has been opened a lot today. Try again later.'];
    if (e.status === 0) return ['Could not reach the server', 'Check your connection and try again.'];
    return ['Something went wrong', `The server answered ${e.status}.`];
  }
  const m = String(e?.message ?? e);
  if (m.startsWith('newer-format')) return ['This page is out of date', 'This album was shared with a newer version of Shoebox. Reload the page, or try again later.'];
  if (m.startsWith('wrong-key') || m.startsWith('bad-signature') || m.startsWith('bad-link')) return ['This link looks incomplete', 'Make sure you copied all of it, including everything after the “#”.'];
  return ['Something went wrong', m];
}

function dayKey(ms) {
  const d = new Date(ms);
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`;
}

function fileName(item, ext) {
  const d = new Date(item.taken);
  const p = (n) => String(n).padStart(2, '0');
  return `${d.getFullYear()}${p(d.getMonth() + 1)}${p(d.getDate())}_${p(d.getHours())}${p(d.getMinutes())}${p(d.getSeconds())}.${ext}`;
}

// ---- the grid ----

const queue = [];
let running = 0;
function enqueue(job) {
  queue.push(job);
  pump();
}
function pump() {
  while (running < THUMB_CONCURRENCY && queue.length) {
    const job = queue.shift();
    running++;
    job().finally(() => {
      running--;
      pump();
    });
  }
}

function renderGrid() {
  const grid = $('grid');
  grid.textContent = '';
  const observer = new IntersectionObserver(
    (entries) => {
      for (const entry of entries) {
        if (!entry.isIntersecting) continue;
        observer.unobserve(entry.target);
        const tile = entry.target;
        const item = items[Number(tile.dataset.index)];
        enqueue(async () => {
          try {
            const img = document.createElement('img');
            img.alt = '';
            img.decoding = 'async';
            img.src = await blobUrl(item.thumb);
            tile.prepend(img);
          } catch {
            /* leave the tile empty; the viewer shows the real error */
          }
        });
      }
    },
    { rootMargin: '600px' },
  );
  let day = null;
  let row = null;
  items.forEach((item, index) => {
    const key = dayKey(item.taken);
    if (key !== day) {
      day = key;
      const h = document.createElement('h2');
      h.className = 'day';
      h.textContent = new Date(item.taken).toLocaleDateString(undefined, { weekday: 'long', year: 'numeric', month: 'long', day: 'numeric' });
      grid.append(h);
      row = document.createElement('div');
      row.className = 'grid';
      row.setAttribute('role', 'list');
      grid.append(row);
    }
    const tile = document.createElement('button');
    tile.type = 'button';
    tile.className = 'tile';
    tile.dataset.index = String(index);
    tile.setAttribute('role', 'listitem');
    tile.setAttribute(
      'aria-label',
      `${item.kind === 'video' ? 'Video' : 'Photo'} taken ${new Date(item.taken).toLocaleString()}${item.motion ? ', with motion' : ''}`,
    );
    if (item.kind === 'video' || item.motion) {
      const b = document.createElement('span');
      b.className = 'badge';
      b.textContent = item.kind === 'video' ? '▶' : 'LIVE';
      tile.append(b);
    }
    tile.addEventListener('click', () => openLightbox(index));
    row.append(tile);
    observer.observe(tile);
  });
}

// ---- the viewer ----

const box = $('lightbox');
let pointerStart = null;

function openLightbox(index) {
  lastFocus = document.activeElement;
  current = index;
  if (!box.open) box.showModal();
  showCurrent();
  $('close').focus();
}

async function showCurrent() {
  const index = current;
  const item = items[index];
  const stage = $('stage');
  stage.textContent = '';
  const wait = document.createElement('p');
  wait.className = 'wait';
  wait.textContent = 'Opening…';
  stage.append(wait);
  $('caption').textContent = `${new Date(item.taken).toLocaleString()} · ${index + 1} of ${items.length}`;
  $('prev').disabled = index === 0;
  $('next').disabled = index === items.length - 1;
  $('live').hidden = !(item.motion || item.kind === 'video');
  $('live').textContent = item.kind === 'video' ? 'Play video' : 'Play motion';
  try {
    const url = await blobUrl(item.preview);
    if (current !== index) return;
    const img = document.createElement('img');
    img.alt = '';
    img.src = url;
    stage.textContent = '';
    stage.append(img);
  } catch (e) {
    if (current === index) stage.textContent = problem(e)[0];
  }
}

async function playMotion() {
  const index = current;
  const item = items[index];
  const ref = item.kind === 'video' ? item.original : item.motion;
  if (!ref) {
    $('caption').textContent = 'This video was not included in the share.';
    return;
  }
  if (ref.size > MAX_PLAY_BYTES) {
    $('caption').textContent = 'This video is large: download it to watch it.';
    return;
  }
  $('live').disabled = true;
  try {
    const url = await blobUrl(ref);
    if (current !== index) return;
    const v = document.createElement('video');
    v.src = url;
    v.controls = true;
    v.autoplay = true;
    v.playsInline = true;
    v.loop = item.kind !== 'video';
    $('stage').textContent = '';
    $('stage').append(v);
  } catch (e) {
    $('caption').textContent = problem(e)[0];
  } finally {
    $('live').disabled = false;
  }
}

function step(delta) {
  const next = current + delta;
  if (next < 0 || next >= items.length) return;
  current = next;
  showCurrent();
}

function saveBytes(bytes, name) {
  const [type] = sniff(bytes);
  const a = document.createElement('a');
  a.href = URL.createObjectURL(new Blob([bytes], { type }));
  a.download = name;
  document.body.append(a);
  a.click();
  a.remove();
  setTimeout(() => URL.revokeObjectURL(a.href), 30000);
}

async function downloadCurrent() {
  const item = items[current];
  const ref = item.original ?? item.preview;
  const button = $('download');
  button.disabled = true;
  try {
    const bytes = await openBlob(ref);
    saveBytes(bytes, fileName(item, sniff(bytes)[1]));
  } catch (e) {
    $('caption').textContent = problem(e)[0];
  } finally {
    button.disabled = false;
  }
}

async function downloadAll() {
  const button = $('download-all');
  const refs = items.map((it) => it.original ?? it.preview);
  const total = refs.reduce((n, r) => n + r.size, 0);
  const canStream = typeof window.showSaveFilePicker === 'function';
  if (!canStream && total > ZIP_IN_MEMORY_LIMIT && !confirm('This is a very large download and your browser has to hold all of it in memory. Continue?')) return;
  let writable = null;
  const parts = [];
  try {
    if (canStream) {
      const handle = await window.showSaveFilePicker({ suggestedName: `${$('album').textContent}.zip`, types: [{ description: 'Zip', accept: { 'application/zip': ['.zip'] } }] });
      writable = await handle.createWritable();
    }
  } catch {
    return; // the person cancelled
  }
  button.disabled = true;
  const zip = zipWriter(async (bytes) => {
    if (writable) await writable.write(bytes);
    else parts.push(bytes);
  });
  const used = new Set();
  try {
    for (let i = 0; i < items.length; i++) {
      say('', `Preparing the download: ${i + 1} of ${items.length}…`, false);
      const bytes = await openBlob(refs[i]);
      if (bytes.length > ZIP_FILE_LIMIT) continue;
      let name = fileName(items[i], sniff(bytes)[1]);
      while (used.has(name)) name = `${i}_${name}`;
      used.add(name);
      await zip.add(name, bytes, new Date(items[i].taken));
    }
    await zip.finish();
    if (writable) await writable.close();
    else {
      const a = document.createElement('a');
      a.href = URL.createObjectURL(new Blob(parts, { type: 'application/zip' }));
      a.download = `${$('album').textContent}.zip`;
      document.body.append(a);
      a.click();
      a.remove();
      setTimeout(() => URL.revokeObjectURL(a.href), 60000);
    }
    say('', '', false);
  } catch (e) {
    if (writable) await writable.abort().catch(() => {});
    const [h, t] = problem(e);
    say(h, t);
  } finally {
    button.disabled = false;
  }
}

box.addEventListener('close', () => {
  current = -1;
  $('stage').textContent = '';
  lastFocus?.focus?.();
});
$('close').addEventListener('click', () => box.close());
$('prev').addEventListener('click', () => step(-1));
$('next').addEventListener('click', () => step(1));
$('download').addEventListener('click', downloadCurrent);
$('live').addEventListener('click', playMotion);
$('download-all').addEventListener('click', downloadAll);
box.addEventListener('keydown', (e) => {
  if (e.key === 'ArrowLeft') step(-1);
  else if (e.key === 'ArrowRight') step(1);
});
$('stage').addEventListener('pointerdown', (e) => {
  pointerStart = e.clientX;
});
$('stage').addEventListener('pointerup', (e) => {
  if (pointerStart === null) return;
  const dx = e.clientX - pointerStart;
  pointerStart = null;
  if (Math.abs(dx) > 50) step(dx < 0 ? 1 : -1);
});

// ---- start ----

async function main() {
  try {
    await init();
  } catch {
    say('This browser cannot open the album', 'It needs WebAssembly, which every current browser has.');
    return;
  }
  let link;
  try {
    link = JSON.parse(core.parse_link(location.hash));
  } catch {
    say('This link looks incomplete', 'Make sure you copied all of it, including everything after the “#”.');
    return;
  }
  try {
    let id = link.locator;
    let transport;
    if (link.mode === 'live') {
      id = link.locator.slice(0, 32);
      say('Looking for the album…', 'It is shared from a device, which can take a few seconds.', false);
      const conn = await connectLive(link.locator.slice(32));
      transport = liveTransport(conn, id);
    } else {
      transport = {
        manifest: () => get(`/s/${id}/m`),
        blob: (cid) => get(`/s/${id}/b/${cid}`),
      };
    }
    const sealed = await transport.manifest();
    share = {
      id,
      key: fromHex(link.key),
      blob: transport.blob,
      manifest: JSON.parse(core.open_share(sealed, fromHex(link.key))),
    };
  } catch (e) {
    const [h, t] = problem(e);
    say(h, t);
    if (e instanceof Offline) {
      const retry = document.createElement('button');
      retry.textContent = 'Try again';
      retry.addEventListener('click', () => location.reload());
      statusEl.append(' ', retry);
    }
    return;
  }
  const m = share.manifest;
  items = [...m.items].sort((a, b) => a.taken - b.taken);
  document.title = `${m.album} · Shared album`;
  $('album').textContent = m.album;
  const bits = [`${items.length} ${items.length === 1 ? 'item' : 'items'}`];
  if (m.expires) bits.push(`link expires ${new Date(m.expires).toLocaleDateString()}`);
  $('meta').textContent = bits.join(' · ');
  if (items.length === 0) {
    say('This album is empty', 'Nothing has been added to it yet.', false);
    return;
  }
  say('', '', false);
  statusEl.hidden = true;
  $('download-all').hidden = false;
  $('download-all').textContent = items.some((i) => i.original) ? 'Download all (full size)' : 'Download all';
  renderGrid();
}

main();
