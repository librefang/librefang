import { describe, expect, it } from "vitest";
import { fadeInScale, fadeInUp, slideInRight, staggerItem } from "./motion";

describe("dashboard motion variants", () => {
  it.each([fadeInScale, fadeInUp, slideInRight, staggerItem])(
    "avoids paint-heavy filter animation",
    (variant) => {
      for (const state of Object.values(variant)) {
        if (typeof state === "object" && state !== null) {
          expect(state).not.toHaveProperty("filter");
        }
      }
    },
  );

  it("shares the fade-up geometry with staggered items", () => {
    expect(staggerItem.initial).toBe(fadeInUp.initial);
    expect(staggerItem.animate).toMatchObject({ opacity: 1, y: 0 });
  });
});
