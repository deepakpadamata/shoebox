<script lang="ts">
  // The date scroller on the right of the timeline: years (or months, when it all fits in one year)
  // placed where they fall, a handle for where you are, and a bubble with the month while you
  // drag. Click or drag anywhere on it to jump. The handle is a pill that shows while the timeline
  // scrolls or the mouse is over the rail; on touch screens the rail takes no touches while it is hidden.
  import { dateMarks, dayAt, type Layout } from '../lib/timeline';

  interface Props { layout: Layout; scrollTop: number; viewport: number; onseek: (top: number) => void; }
  let { layout, scrollTop, viewport, onseek }: Props = $props();

  const MONTHS = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];
  const PAD = 26;
  const MIN_GAP = 20;

  let height = $state(0);
  let dragging = $state(false);
  let hovering = $state(false);
  let recent = $state(false);
  let timer: ReturnType<typeof setTimeout> | undefined;

  const maxScroll = $derived(Math.max(1, layout.height - viewport));
  const track = $derived(Math.max(1, height - PAD * 2));
  const yOf = (top: number) => PAD + Math.min(1, top / maxScroll) * track;
  const marks = $derived(dateMarks(layout));
  const byMonth = $derived(new Set(marks.map((m) => m.year)).size <= 1);

  // Labels where they fall, skipping any that would crowd the one above.
  const labels = $derived.by(() => {
    const out: { y: number; text: string }[] = [];
    let lastYear = -1;
    let lastY = -Infinity;
    for (const m of marks) {
      if (!byMonth && m.year === lastYear) continue;
      lastYear = m.year;
      const y = yOf(m.top);
      if (y - lastY < MIN_GAP) continue;
      lastY = y;
      out.push({ y, text: byMonth ? MONTHS[m.month - 1] : String(m.year) });
    }
    return out;
  });

  const handleY = $derived(yOf(scrollTop));
  const current = $derived.by(() => {
    const day = dayAt(layout, scrollTop + 1);
    if (!day) return '';
    return `${MONTHS[Number(day.slice(5, 7)) - 1]} ${day.slice(0, 4)}`;
  });

  // The bubble also shows for a moment while the timeline scrolls by other means.
  $effect(() => {
    scrollTop;
    recent = true;
    clearTimeout(timer);
    timer = setTimeout(() => (recent = false), 1200);
    return () => clearTimeout(timer);
  });

  function seekTo(e: PointerEvent) {
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    const f = Math.min(1, Math.max(0, (e.clientY - r.top - PAD) / track));
    onseek(f * maxScroll);
  }
  function down(e: PointerEvent) {
    e.preventDefault();
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    dragging = true;
    seekTo(e);
  }
  function move(e: PointerEvent) {
    if (dragging) seekTo(e);
  }
  function up() {
    dragging = false;
  }
</script>

<div class="rail" class:active={dragging || hovering} class:awake={dragging || hovering || recent} bind:clientHeight={height} role="scrollbar" aria-label="Scroll by date"
  aria-controls="timeline" aria-orientation="vertical" aria-valuemin={0} aria-valuemax={100}
  aria-valuenow={Math.round((scrollTop / maxScroll) * 100)} aria-valuetext={current} tabindex="-1"
  onpointerdown={down} onpointermove={move} onpointerup={up} onpointercancel={up}
  onpointerenter={(e) => (hovering = e.pointerType === 'mouse')} onpointerleave={() => (hovering = false)}>
  {#each labels as l (l.y)}
    <span class="label" style="top:{l.y}px">{l.text}</span>
  {/each}
  <span class="handle" style="top:{handleY}px">
    <svg viewBox="0 0 12 20" aria-hidden="true"><path d="M2 7 L6 3 L10 7 M2 13 L6 17 L10 13" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" /></svg>
  </span>
  {#if current && (dragging || hovering || recent)}
    <span class="bubble" style="top:{handleY}px">{current}</span>
  {/if}
</div>

<style>
  /* Idle, only a strip at the edge takes the mouse; on touch, nothing does until the pill shows. */
  .rail { position: absolute; top: 0; right: 0; bottom: 0; width: 24px; z-index: 5; touch-action: none; user-select: none; cursor: pointer; }
  .rail.awake { width: 48px; }
  .label { position: absolute; right: 44px; pointer-events: none; transform: translateY(-50%); font-size: 11px; font-weight: 600; color: var(--ink-3);
    background: color-mix(in srgb, var(--bg) 80%, transparent); padding: 1px 4px; border-radius: 6px; opacity: 0; transition: opacity 0.15s; }
  .rail.active .label { opacity: 1; }
  .handle { position: absolute; right: 6px; width: 30px; height: 48px; border-radius: 15px; display: grid; place-items: center; opacity: 0;
    transform: translateY(-50%); background: var(--ink); color: var(--bg); box-shadow: var(--shadow); transition: opacity 0.25s; }
  .rail.awake .handle { opacity: 1; }
  .rail.active .handle { background: var(--accent); color: var(--accent-ink); }
  .handle svg { width: 12px; height: 20px; }
  .bubble { position: absolute; right: 44px; transform: translateY(-50%); white-space: nowrap; pointer-events: none;
    background: var(--ink); color: var(--bg); font-size: 13px; font-weight: 600; padding: 6px 11px; border-radius: 12px; box-shadow: var(--shadow); }
  @media (pointer: coarse) {
    .rail:not(.awake) { pointer-events: none; }
    .bubble { font-size: 14px; padding: 7px 12px; }
  }
</style>
