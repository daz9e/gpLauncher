import '@testing-library/jest-dom/vitest';
import { afterEach } from 'vitest';
import { cleanup } from '@testing-library/svelte';

// jsdom lacks these browser APIs.
class NoObserver {
  observe() {}
  unobserve() {}
  disconnect() {}
  takeRecords() {
    return [];
  }
}
globalThis.IntersectionObserver ??= NoObserver as unknown as typeof IntersectionObserver;
globalThis.ResizeObserver ??= NoObserver as unknown as typeof ResizeObserver;
Element.prototype.scrollIntoView ??= function () {};
Element.prototype.scrollTo ??= function () {};

afterEach(() => cleanup());
