import "@testing-library/jest-dom/vitest";
import { beforeEach } from "vitest";

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
    writable: true,
    value: new MemoryStorage(),
  });
}

installTestStorage("localStorage");
installTestStorage("sessionStorage");

// cmdk and Recharts use ResizeObserver internally. Tests that need a resize
// can call `trigger`; observing alone stays side-effect free like the browser.
export class MockResizeObserver implements ResizeObserver {
  private readonly callback: ResizeObserverCallback;
  private readonly targets = new Set<Element>();

  constructor(callback: ResizeObserverCallback) {
    this.callback = callback;
  }

  observe(target: Element) {
    this.targets.add(target);
  }

  unobserve(target: Element) {
    this.targets.delete(target);
  }

  disconnect() {
    this.targets.clear();
  }

  trigger(entries: ResizeObserverEntry[]) {
    const observedEntries = entries.filter((entry) =>
      this.targets.has(entry.target),
    );
    if (observedEntries.length > 0) {
      this.callback(observedEntries, this);
    }
  }
}

global.ResizeObserver = MockResizeObserver;

class MockMediaQueryList extends EventTarget {
  readonly media: string;
  matches = false;
  onchange: ((this: MediaQueryList, ev: MediaQueryListEvent) => unknown) | null =
    null;

  constructor(query: string) {
    super();
    this.media = query;
  }

  addListener(callback: ((event: MediaQueryListEvent) => void) | null) {
    if (callback) this.addEventListener("change", callback as EventListener);
  }

  removeListener(callback: ((event: MediaQueryListEvent) => void) | null) {
    if (callback) this.removeEventListener("change", callback as EventListener);
  }

  override dispatchEvent(event: Event): boolean {
    if (event.type === "change" && this.onchange) {
      this.onchange.call(this, event as MediaQueryListEvent);
    }
    return super.dispatchEvent(event);
  }
}

const mediaQueries = new Map<string, MockMediaQueryList>();

// jsdom does not implement matchMedia; PushDrawer.useIsMobile and a few
// pages call it during mount and crash the test render without a stub.
if (typeof window !== "undefined" && typeof window.matchMedia !== "function") {
  Object.defineProperty(window, "matchMedia", {
    writable: true,
    value: (query: string) => {
      const existing = mediaQueries.get(query);
      if (existing) return existing;
      const created = new MockMediaQueryList(query);
      mediaQueries.set(query, created);
      return created;
    },
  });
}

beforeEach(() => {
  globalThis.localStorage?.clear?.();
  globalThis.sessionStorage?.clear?.();
  mediaQueries.clear();
});
