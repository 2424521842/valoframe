import "@testing-library/jest-dom/vitest";
import { cleanup, configure } from "@testing-library/react";
import { afterEach } from "vitest";

// Every full-App test waits for a lazy-loaded workspace to resolve. Testing Library's default
// 1000ms async budget is enough on a fast dev box but not on a 2-core CI runner, where module
// resolution alone dominates the run — the failures showed up as `findBy*` giving up after ~1.3s
// with only the sidebar rendered. Raise the async budget; per-test timeout stays in the config.
configure({ asyncUtilTimeout: 10_000 });

afterEach(() => {
  cleanup();
  window.localStorage.clear();
  delete document.documentElement.dataset.motion;
});

Object.defineProperty(window, "matchMedia", {
  configurable: true,
  value: (query: string) => ({
    matches: false,
    media: query,
    onchange: null,
    addEventListener: () => undefined,
    removeEventListener: () => undefined,
    addListener: () => undefined,
    removeListener: () => undefined,
    dispatchEvent: () => false,
  }),
});

class TestResizeObserver {
  observe() {}
  unobserve() {}
  disconnect() {}
}

Object.defineProperty(globalThis, "ResizeObserver", {
  configurable: true,
  value: TestResizeObserver,
});

for (const method of ["hasPointerCapture", "setPointerCapture", "releasePointerCapture"] as const) {
  if (!(method in HTMLElement.prototype)) {
    Object.defineProperty(HTMLElement.prototype, method, {
      configurable: true,
      value: method === "hasPointerCapture" ? () => false : () => undefined,
    });
  }
}

if (!("scrollIntoView" in Element.prototype)) {
  Object.defineProperty(Element.prototype, "scrollIntoView", {
    configurable: true,
    value: () => undefined,
  });
}
