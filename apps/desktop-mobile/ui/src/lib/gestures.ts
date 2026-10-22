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
  /** The finger moved (after a long press) over this element. */
  onOver(el: Element | null): void;
  onEnd(): void;
  delayMs?: number;
}

/** Long-press to start, then drag across items (multi-select by drag, mockup 2b). */
export function longPressDrag(node: HTMLElement, opts: LongPressDragOptions) {
  let timer: ReturnType<typeof setTimeout> | undefined;
  let active = false;
  let origin: { x: number; y: number } | null = null;
  const at = (e: PointerEvent) => document.elementFromPoint(e.clientX, e.clientY);
  const down = (e: PointerEvent) => {
    if (e.pointerType === 'mouse') return;
    origin = { x: e.clientX, y: e.clientY };
    timer = setTimeout(() => {
      active = true;
      opts.onStart(at(e));
    }, opts.delayMs ?? 450);
  };
  const move = (e: PointerEvent) => {
    if (active) {
      e.preventDefault();
      opts.onOver(at(e));
    } else if (origin && Math.hypot(e.clientX - origin.x, e.clientY - origin.y) > 10) {
      clearTimeout(timer); // it is a scroll, not a long press
    }
  };
  const end = () => {
    clearTimeout(timer);
    if (active) opts.onEnd();
    active = false;
    origin = null;
  };
  node.addEventListener('pointerdown', down);
  node.addEventListener('pointermove', move, { passive: false });
  node.addEventListener('pointerup', end);
  node.addEventListener('pointercancel', end);
  return {
    destroy() {
      end();
      node.removeEventListener('pointerdown', down);
      node.removeEventListener('pointermove', move);
      node.removeEventListener('pointerup', end);
      node.removeEventListener('pointercancel', end);
    },
  };
}
