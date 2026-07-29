import { describe, it, expect, beforeEach } from 'vitest';
import { act, renderHook } from '@testing-library/react';
import {
  clampWidth,
  DEFAULT_WIDTH,
  MAX_WIDTH,
  MIN_WIDTH,
  useWindowState,
} from './use-window-state';

function setViewport(width: number) {
  Object.defineProperty(window, 'innerWidth', { configurable: true, value: width });
}

describe('clampWidth', () => {
  it('holds the bounds', () => {
    expect(clampWidth(10, 1600)).toBe(MIN_WIDTH);
    expect(clampWidth(99999, 1600)).toBe(MAX_WIDTH);
  });

  it('never exceeds the viewport, so a width from a wider monitor still fits', () => {
    // The panel must not render wider than the window it is docked in.
    expect(clampWidth(1000, 600)).toBe(576);
    expect(clampWidth(1000, 600)).toBeLessThan(600);
  });

  it('keeps the minimum usable even on a viewport narrower than it', () => {
    // Below MIN_WIDTH the composer stops working; a tiny window must not produce a
    // panel that cannot be typed into.
    expect(clampWidth(500, 100)).toBe(MIN_WIDTH);
  });

  it('rounds to whole pixels', () => {
    expect(clampWidth(480.7, 1600)).toBe(481);
  });
});

describe('useWindowState', () => {
  beforeEach(() => {
    window.localStorage.clear();
    setViewport(1600);
  });

  it('defaults, then persists a chosen width across mounts', () => {
    const first = renderHook(() => useWindowState());
    expect(first.result.current.width).toBe(DEFAULT_WIDTH);

    act(() => first.result.current.setWidth(700));
    expect(first.result.current.width).toBe(700);

    const second = renderHook(() => useWindowState());
    expect(second.result.current.width).toBe(700);
  });

  it('clamps a stored width that no longer fits the viewport', () => {
    window.localStorage.setItem('fkst-chat-width', '1000');
    setViewport(600);
    const { result } = renderHook(() => useWindowState());
    expect(result.current.width).toBe(576);
  });

  it('ignores a corrupt stored width rather than rendering NaN', () => {
    window.localStorage.setItem('fkst-chat-width', 'not-a-number');
    const { result } = renderHook(() => useWindowState());
    expect(result.current.width).toBe(DEFAULT_WIDTH);
  });

  it('re-clamps when the window is narrowed after mount', () => {
    const { result } = renderHook(() => useWindowState());
    act(() => result.current.setWidth(900));

    act(() => {
      setViewport(500);
      window.dispatchEvent(new Event('resize'));
    });
    expect(result.current.width).toBeLessThanOrEqual(500);
  });

  it('persists pin but NOT full screen', () => {
    const first = renderHook(() => useWindowState());
    act(() => first.result.current.togglePinned());
    act(() => first.result.current.toggleFullScreen());
    expect(first.result.current.pinned).toBe(true);
    expect(first.result.current.fullScreen).toBe(true);

    // Full screen is a momentary reading mode; reopening already maximised would
    // be a surprise. Pin is a durable preference.
    const second = renderHook(() => useWindowState());
    expect(second.result.current.pinned).toBe(true);
    expect(second.result.current.fullScreen).toBe(false);
  });

  it('exits full screen without touching pin or width', () => {
    const { result } = renderHook(() => useWindowState());
    act(() => result.current.setWidth(640));
    act(() => result.current.togglePinned());
    act(() => result.current.toggleFullScreen());
    act(() => result.current.exitFullScreen());

    expect(result.current.fullScreen).toBe(false);
    expect(result.current.pinned).toBe(true);
    expect(result.current.width).toBe(640);
  });

  it('survives storage being unavailable', () => {
    const getItem = window.localStorage.getItem;
    window.localStorage.getItem = () => {
      throw new Error('blocked');
    };
    try {
      const { result } = renderHook(() => useWindowState());
      expect(result.current.width).toBe(DEFAULT_WIDTH);
      expect(result.current.pinned).toBe(false);
    } finally {
      window.localStorage.getItem = getItem;
    }
  });
});
