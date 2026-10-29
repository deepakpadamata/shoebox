// Touch gestures (M4-11) as Svelte actions on pointer events: swipe, pinch, long-press and
// drag-select. Pure helpers are exported and unit-tested; the actions only wire them to the DOM.

export type SwipeDir = 'left' | 'right' | 'down' | 'up';

/** Which swipe a drag of (dx, dy) is, or null if it is too short or too ambiguous. */
export function classifySwipe(dx: number, dy: number, minDistance = 56): SwipeDir | null {
  const ax = Math.abs(dx);
  const ay = Math.abs(dy);
  if (Math.max(ax, ay) < minDistance) return null;
  if (ax > ay * 1.5) return dx < 0 ? 'left' : 'right';
  if (ay > ax * 1.5) return dy > 0 ? 'down' : 'up';
  return null;
}

/** The grid density (minimum tile edge) after a pinch that scaled by `scale`. */
export function stepDensity(current: number, scale: number, steps: number[]): number {
  const i = steps.reduce((best, s, k) => (Math.abs(s - current) < Math.abs(steps[best] - current) ? k : best), 0);
  if (scale > 1.25) return steps[Math.min(steps.length - 1, i + 1)]; // spread: bigger tiles
  if (scale < 0.8) return steps[Math.max(0, i - 1)]; // pinch: smaller tiles
  return steps[i];
}

export interface SwipeOptions {
  onSwipe(dir: SwipeDir): void;
  /** Called while a vertical drag is in progress (for a "pull down to close" animation). */
  onDrag?(dx: number, dy: number): void;
  onEnd?(): void;
  /** Ignore gestures that start when a second finger is down (pinch takes over). */
  enabled?: () => boolean;
}

export function swipe(node: HTMLElement, opts: SwipeOptions) {
  let start: { x: number; y: number; id: number } | null = null;
  const down = (e: PointerEvent) => {
    if (e.pointerType === 'mouse' || (opts.enabled && !opts.enabled())) return;
    start = { x: e.clientX, y: e.clientY, id: e.pointerId };
  };
  const move = (e: PointerEvent) => {
    if (start && e.pointerId === start.id) opts.onDrag?.(e.clientX - start.x, e.clientY - start.y);
  };
  const up = (e: PointerEvent) => {
    if (!start || e.pointerId !== start.id) return;
    const dir = classifySwipe(e.clientX - start.x, e.clientY - start.y);
    start = null;
    opts.onEnd?.();
    if (dir) opts.onSwipe(dir);
  };
  const cancel = () => {
    start = null;
    opts.onEnd?.();
  };
  node.addEventListener('pointerdown', down);
  node.addEventListener('pointermove', move);
  node.addEventListener('pointerup', up);
  node.addEventListener('pointercancel', cancel);
  return {
    destroy() {
      node.removeEventListener('pointerdown', down);
      node.removeEventListener('pointermove', move);
      node.removeEventListener('pointerup', up);
      node.removeEventListener('pointercancel', cancel);
    },
  };
}

export interface PinchOptions {
  /** Live scale relative to the start of the pinch. */
  onScale?(scale: number): void;
  /** The pinch finished with this total scale. */
  onEnd(scale: number): void;
}

export function pinch(node: HTMLElement, opts: PinchOptions) {
  const pts = new Map<number, { x: number; y: number }>();
  let startDist = 0;
  const dist = () => {
    const [a, b] = [...pts.values()];
    return Math.hypot(a.x - b.x, a.y - b.y);
  };
  const down = (e: PointerEvent) => {
    pts.set(e.pointerId, { x: e.clientX, y: e.clientY });
    if (pts.size === 2) startDist = dist();
  };
  const move = (e: PointerEvent) => {
    if (!pts.has(e.pointerId)) return;
    pts.set(e.pointerId, { x: e.clientX, y: e.clientY });
    if (pts.size === 2 && startDist > 0) opts.onScale?.(dist() / startDist);
  };
  const up = (e: PointerEvent) => {
    if (pts.size === 2 && startDist > 0 && pts.has(e.pointerId)) {
      const scale = dist() / startDist;
      startDist = 0;
      opts.onEnd(scale);
    }
    pts.delete(e.pointerId);
  };
  node.addEventListener('pointerdown', down);
  node.addEventListener('pointermove', move);
  node.addEventListener('pointerup', up);
  node.addEventListener('pointercancel', up);
  return {
    destroy() {
      node.removeEventListener('pointerdown', down);
      node.removeEventListener('pointermove', move);
      node.removeEventListener('pointerup', up);
      node.removeEventListener('pointercancel', up);
    },
  };
}

export interface LongPressDragOptions {
  /** A long press started on the element under the finger. */
  onStart(el: Element | null): void;
  /** The finger moved (after a long press) over this element, or the view scrolled under it. */
  onOver(el: Element | null): void;
  onEnd(): void;
  delayMs?: number;
}

/**
 * Auto-scroll speed (px per frame) for a finger at `y` in a view from `top` to `bottom`: zero
 * outside the edge bands, rising to `max` at the edge and beyond; negative scrolls up.
 */
export function edgeSpeed(y: number, top: number, bottom: number, band = 72, max = 22): number {
  const ramp = (d: number) => Math.round(max * Math.min(1, Math.max(0, (band - d) / band)) ** 2);
  if (y - top < band) return -ramp(y - top);
  if (bottom - y < band) return ramp(bottom - y);
  return 0;
}

/**
 * Long-press to start, then drag across items (multi-select by drag, mockup 2b). Near the top or
 * bottom edge the view scrolls by itself, and the items passing under the finger keep counting.
 */
export function longPressDrag(node: HTMLElement, opts: LongPressDragOptions) {
  let timer: ReturnType<typeof setTimeout> | undefined;
  let active = false;
  let origin: { x: number; y: number } | null = null;
  let finger: { x: number; y: number } | null = null;
  let frame = 0;
  const at = (x: number, y: number) => {
    // Probe inside the view, even with the finger past its edge.
    const r = node.getBoundingClientRect();
    return document.elementFromPoint(x, Math.min(r.bottom - 2, Math.max(r.top + 2, y)));
  };
  const scroll = () => {
    frame = 0;
    if (!active || !finger) return;
    const r = node.getBoundingClientRect();
    const v = edgeSpeed(finger.y, r.top, r.bottom);
    if (v === 0) return;
    const before = node.scrollTop;
    node.scrollTop += v;
    if (node.scrollTop !== before) opts.onOver(at(finger.x, finger.y));
    frame = requestAnimationFrame(scroll);
  };
  const down = (e: PointerEvent) => {
    if (e.pointerType === 'mouse') return;
    origin = finger = { x: e.clientX, y: e.clientY };
    timer = setTimeout(() => {
      active = true;
      node.dataset.dragSelecting = '';
      opts.onStart(at(e.clientX, e.clientY));
    }, opts.delayMs ?? 450);
  };
  const move = (e: PointerEvent) => {
    finger = { x: e.clientX, y: e.clientY };
    if (active) {
      e.preventDefault();
      opts.onOver(at(e.clientX, e.clientY));
      if (!frame) frame = requestAnimationFrame(scroll);
    } else if (origin && Math.hypot(e.clientX - origin.x, e.clientY - origin.y) > 10) {
      clearTimeout(timer); // it is a scroll, not a long press
    }
  };
  // Once selecting, the finger must not also scroll the page: only the edge auto-scroll does.
  const touchmove = (e: TouchEvent) => {
    if (active && e.cancelable) e.preventDefault();
  };
  const end = () => {
    clearTimeout(timer);
    cancelAnimationFrame(frame);
    frame = 0;
    if (active) opts.onEnd();
    active = false;
    delete node.dataset.dragSelecting;
    origin = finger = null;
  };
  node.addEventListener('pointerdown', down);
  node.addEventListener('pointermove', move, { passive: false });
  node.addEventListener('touchmove', touchmove, { passive: false });
  node.addEventListener('pointerup', end);
  node.addEventListener('pointercancel', end);
  return {
    destroy() {
      end();
      node.removeEventListener('pointerdown', down);
      node.removeEventListener('pointermove', move);
      node.removeEventListener('touchmove', touchmove);
      node.removeEventListener('pointerup', end);
      node.removeEventListener('pointercancel', end);
    },
  };
}

export interface PullRefreshOptions {
  onRefresh: () => Promise<unknown> | unknown;
  /** Distance in pixels that counts as a pull. */
  threshold?: number;
}

/**
 * Pull down from the top of a scrolling list to refresh it. A circle follows the finger and fills
 * as the pull gets longer; past the threshold, letting go refreshes, and it spins until
 * `onRefresh` settles.
 */
export function pullRefresh(node: HTMLElement, opts: PullRefreshOptions) {
  const threshold = opts.threshold ?? 72;
  const SIZE = 36;
  const R = 13;
  const C = 2 * Math.PI * R;
  let startY = 0;
  let pulling = false;
  let busy = false;
  let spin: Animation | undefined;
  const wrap = document.createElement('div');
  wrap.setAttribute('role', 'progressbar');
  wrap.setAttribute('aria-label', 'Refreshing');
  wrap.style.cssText = `position:fixed;left:50%;z-index:20;width:${SIZE}px;height:${SIZE}px;margin-left:-${SIZE / 2}px;border-radius:50%;background:var(--surface);border:1px solid var(--line);box-shadow:var(--shadow);pointer-events:none;opacity:0;display:grid;place-items:center`;
  wrap.innerHTML = `<svg width="${SIZE - 8}" height="${SIZE - 8}" viewBox="0 0 ${SIZE} ${SIZE}" style="transform:rotate(-90deg)"><circle cx="${SIZE / 2}" cy="${SIZE / 2}" r="${R}" fill="none" stroke="var(--accent)" stroke-width="3" stroke-linecap="round" stroke-dasharray="${C}" stroke-dashoffset="${C}"/></svg>`;
  document.body.appendChild(wrap);
  const arc = wrap.querySelector('circle') as SVGCircleElement;
  const svg = wrap.querySelector('svg') as SVGElement;

  const place = (dy: number) => {
    const top = node.getBoundingClientRect().top;
    const progress = Math.min(1, dy / threshold);
    wrap.style.top = `${top}px`;
    wrap.style.transform = `translateY(${Math.min(dy, threshold * 1.3) - SIZE - 4}px)`;
    wrap.style.opacity = dy > 6 || busy ? '1' : '0';
    arc.style.strokeDashoffset = String(C * (1 - progress * 0.85));
    svg.style.transform = `rotate(${-90 + progress * 270}deg)`;
  };
  const start = (e: TouchEvent) => {
    if (busy || node.scrollTop > 0 || e.touches.length !== 1) return;
    startY = e.touches[0].clientY;
    pulling = true;
  };
  const move = (e: TouchEvent) => {
    if (!pulling) return;
    // A long-press drag-select (`longPressDrag`) owns this touch.
    if ('dragSelecting' in node.dataset) {
      pulling = false;
      wrap.style.opacity = '0';
      return;
    }
    const dy = e.touches[0].clientY - startY;
    if (dy <= 0 || node.scrollTop > 0) {
      wrap.style.opacity = '0';
      return;
    }
    place(dy * 0.6);
  };
  const end = async (e: TouchEvent) => {
    if (!pulling) return;
    pulling = false;
    const dy = (e.changedTouches[0]?.clientY ?? startY) - startY;
    if (dy * 0.6 < threshold) {
      wrap.style.opacity = '0';
      return;
    }
    busy = true;
    place(threshold);
    arc.style.strokeDashoffset = String(C * 0.3);
    spin = svg.animate([{ transform: 'rotate(-90deg)' }, { transform: 'rotate(270deg)' }], { duration: 800, iterations: Infinity });
    try {
      await opts.onRefresh();
    } finally {
      busy = false;
      spin?.cancel();
      wrap.style.opacity = '0';
    }
  };
  node.addEventListener('touchstart', start, { passive: true });
  node.addEventListener('touchmove', move, { passive: true });
  node.addEventListener('touchend', end);
  node.addEventListener('touchcancel', end);
  return {
    destroy() {
      node.removeEventListener('touchstart', start);
      node.removeEventListener('touchmove', move);
      node.removeEventListener('touchend', end);
      node.removeEventListener('touchcancel', end);
      spin?.cancel();
      wrap.remove();
    },
  };
}
