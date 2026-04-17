import "@testing-library/jest-dom/vitest";

// cmdk uses ResizeObserver internally; jsdom doesn't provide it
global.ResizeObserver = class ResizeObserver {
  observe() {}
  unobserve() {}
  disconnect() {}
};
