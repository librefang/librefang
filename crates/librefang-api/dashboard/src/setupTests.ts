import "@testing-library/jest-dom/vitest";

class MemoryStorage implements Storage {
  private store = new Map<string, string>();

  get length() {
    return this.store.size;
  }

  clear() {
    this.store.clear();
  }

  getItem(key: string) {
    return this.store.get(key) ?? null;
  }

  key(index: number) {
    return [...this.store.keys()][index] ?? null;
  }

  removeItem(key: string) {
    this.store.delete(key);
  }

  setItem(key: string, value: string) {
    this.store.set(key, value);
  }
}

function installTestStorage(name: "localStorage" | "sessionStorage") {
  // Install deterministic storage without reading Node's experimental global
  // getter. Node 25 warns on that read unless --localstorage-file is set,
  // while jsdom may expose a throwing Storage for opaque origins.
  Object.defineProperty(globalThis, name, {
    configurable: true,
    value: new MemoryStorage(),
  });
}

installTestStorage("localStorage");
installTestStorage("sessionStorage");

// cmdk uses ResizeObserver internally; jsdom doesn't provide it
global.ResizeObserver = class ResizeObserver {
  observe() {}
  unobserve() {}
  disconnect() {}
};

// jsdom implements no layout, so Element.prototype.scrollIntoView is absent.
// Handlers that scroll a section into view after acting on it (the workflow
// re-run button, for one) then throw an uncaught TypeError, which vitest
// reports as an unhandled error and exits non-zero even when every assertion
// passed. Stub it as the no-op it effectively is without a layout engine.
if (typeof Element !== "undefined" && !Element.prototype.scrollIntoView) {
  Element.prototype.scrollIntoView = function scrollIntoView() {};
}

// jsdom does not implement matchMedia; PushDrawer.useIsMobile and a few
// pages call it during mount and crash the test render without a stub.
if (typeof window !== "undefined" && typeof window.matchMedia !== "function") {
  Object.defineProperty(window, "matchMedia", {
    writable: true,
    value: (query: string) => ({
      matches: false,
      media: query,
      onchange: null,
      addListener: () => {},
      removeListener: () => {},
      addEventListener: () => {},
      removeEventListener: () => {},
      dispatchEvent: () => false,
    }),
  });
}
