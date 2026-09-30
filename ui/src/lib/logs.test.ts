import { describe, expect, it } from 'vitest';
import { LogBuffer, MAX_LINES, filterLines } from './logs';
import type { LogLine } from './types';

const line = (text: string, level: LogLine['level'] = 'info'): LogLine => ({ text, level });

describe('LogBuffer', () => {
  it('appends chunks that connect and asks for a refetch otherwise', () => {
    const b = new LogBuffer();
    b.reset({ id: 'a', key: 1, start: 0, lines: [line('one'), line('two')] });
    expect(b.append({ id: 'a', key: 1, start: 2, lines: [line('three')] })).toBe(true);
    // Overlapping chunks add only what is new.
    expect(b.append({ id: 'a', key: 1, start: 1, lines: [line('two'), line('three'), line('four')] })).toBe(true);
    expect(b.lines.map((l) => l.text)).toEqual(['one', 'two', 'three', 'four']);
    // Already seen.
    expect(b.append({ id: 'a', key: 1, start: 0, lines: [line('one')] })).toBe(true);
    // A gap, or another launch.
    expect(b.append({ id: 'a', key: 1, start: 9, lines: [line('x')] })).toBe(false);
    expect(b.append({ id: 'a', key: 2, start: 4, lines: [line('x')] })).toBe(false);
    expect(b.end).toBe(4);
  });

  it('keeps at most MAX_LINES and counts dropped ones', () => {
    const b = new LogBuffer();
    b.reset({ id: 'a', key: 1, start: 0, lines: [] });
    const many = Array.from({ length: MAX_LINES + 5 }, (_, i) => line(String(i)));
    expect(b.append({ id: 'a', key: 1, start: 0, lines: many })).toBe(true);
    expect(b.lines.length).toBe(MAX_LINES);
    expect(b.start).toBe(5);
    expect(b.lines[0].text).toBe('5');
    expect(b.append({ id: 'a', key: 1, start: MAX_LINES + 5, lines: [line('last')] })).toBe(true);
    expect(b.start).toBe(6);
  });

  it('fixed lines have no launch', () => {
    const b = new LogBuffer();
    b.setLines([line('a')]);
    expect(b.key).toBeNull();
    expect(b.append({ id: 'a', key: 1, start: 0, lines: [line('b')] })).toBe(false);
  });
});

describe('filterLines', () => {
  const lines = [line('Loading', 'launcher'), line('hello'), line('careful', 'warn'), line('Boom', 'error'), line('trace', 'debug')];
  it('shows everything without a filter', () => {
    expect(filterLines(lines, 'all', '  ')).toBeNull();
  });
  it('filters by severity and text', () => {
    expect(filterLines(lines, 'warnings', '')).toEqual([2, 3]);
    expect(filterLines(lines, 'errors', '')).toEqual([3]);
    expect(filterLines(lines, 'all', 'BOO')).toEqual([3]);
    expect(filterLines(lines, 'warnings', 'care')).toEqual([2]);
  });
});
