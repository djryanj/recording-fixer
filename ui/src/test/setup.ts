// Global test setup. Everything here is a no-op in the node environment.
import { afterEach, vi } from 'vitest';

if (typeof document !== 'undefined') {
  await import('@testing-library/jest-dom/vitest');
  const { cleanup } = await import('@testing-library/svelte');
  afterEach(() => cleanup());

  // jsdom has no canvas: hand out a recording 2D context so drawing code runs and can be inspected.
  const calls: { fn: string; args: unknown[] }[] = [];
  const ctx = new Proxy(
    {},
    {
      get(_t, prop: string) {
        if (prop === 'canvas') return undefined;
        if (prop === 'measureText') return () => ({ width: 40 });
        return (...args: unknown[]) => void calls.push({ fn: prop, args });
      },
      set: () => true,
    },
  );
  HTMLCanvasElement.prototype.getContext = vi.fn(() => ctx) as never;
  (globalThis as { __canvasCalls?: typeof calls }).__canvasCalls = calls;
  afterEach(() => (calls.length = 0));

  class RO {
    observe() {}
    unobserve() {}
    disconnect() {}
  }
  globalThis.ResizeObserver ??= RO as never;

  // jsdom lacks PointerEvent; a MouseEvent with a pointerId is enough for the components.
  if (!('PointerEvent' in globalThis)) {
    class PointerEventShim extends MouseEvent {
      pointerId: number;
      constructor(type: string, init: MouseEventInit & { pointerId?: number } = {}) {
        super(type, init);
        this.pointerId = init.pointerId ?? 1;
      }
    }
    globalThis.PointerEvent = PointerEventShim as never;
  }
  HTMLElement.prototype.setPointerCapture ??= () => {};
  HTMLElement.prototype.releasePointerCapture ??= () => {};
  HTMLElement.prototype.hasPointerCapture ??= () => false;
  Element.prototype.scrollTo ??= () => {};
  // Layout does not exist in jsdom; give elements a stable size so width-dependent code runs.
  Object.defineProperty(HTMLElement.prototype, 'clientWidth', { configurable: true, get: () => 1000 });
  Object.defineProperty(HTMLElement.prototype, 'clientHeight', { configurable: true, get: () => 600 });
  Element.prototype.getBoundingClientRect = function () {
    return { x: 0, y: 0, left: 0, top: 0, right: 1000, bottom: 600, width: 1000, height: 600, toJSON() {} };
  };
}
