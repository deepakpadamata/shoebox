import { describe, expect, it } from 'vitest';
import { classifySwipe, stepDensity } from './gestures';

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
