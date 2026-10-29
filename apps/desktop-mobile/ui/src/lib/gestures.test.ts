import { describe, expect, it } from 'vitest';
import { classifySwipe, edgeSpeed, stepDensity } from './gestures';

describe('gestures', () => {
  it('classifies swipes and ignores short or diagonal drags', () => {
    expect(classifySwipe(-120, 10)).toBe('left');
    expect(classifySwipe(120, -20)).toBe('right');
    expect(classifySwipe(5, 180)).toBe('down');
    expect(classifySwipe(0, -90)).toBe('up');
    expect(classifySwipe(30, 20)).toBeNull();
    expect(classifySwipe(100, 90)).toBeNull();
  });

  it('steps the grid density with the pinch scale', () => {
    const steps = [72, 96, 130, 180];
    expect(stepDensity(96, 1.6, steps)).toBe(130);
    expect(stepDensity(96, 0.5, steps)).toBe(72);
    expect(stepDensity(96, 1.1, steps)).toBe(96);
    expect(stepDensity(180, 2, steps)).toBe(180);
    expect(stepDensity(72, 0.4, steps)).toBe(72);
    expect(stepDensity(100, 1.5, steps)).toBe(130);
  });
});

describe('edgeSpeed', () => {
  it('scrolls only near the edges, faster closer to them', () => {
    expect(edgeSpeed(400, 0, 800)).toBe(0);
    expect(edgeSpeed(72, 0, 800)).toBe(0);
    expect(edgeSpeed(60, 0, 800)).toBeLessThan(0);
    expect(edgeSpeed(10, 0, 800)).toBeLessThan(edgeSpeed(60, 0, 800));
    expect(edgeSpeed(0, 0, 800)).toBe(-22);
    expect(edgeSpeed(-50, 0, 800)).toBe(-22);
    expect(edgeSpeed(790, 0, 800)).toBeGreaterThan(edgeSpeed(740, 0, 800));
    expect(edgeSpeed(900, 0, 800)).toBe(22);
  });
});
