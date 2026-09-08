import "@testing-library/jest-dom/vitest";

import { cleanup } from "@testing-library/react";
import { afterEach } from "vitest";

afterEach(cleanup);

// jsdom has no layout engine, so the resizable shell panels get a no-op
// observer. Sizes are meaningless in tests; presence and behaviour are not.
// Base UI's scroll area asks the viewport for its running animations on a
// timer. jsdom has no Web Animations API, so without this the timer throws
// after the test that scheduled it has already finished.
if (typeof Element !== "undefined" && !("getAnimations" in Element.prototype)) {
  (Element.prototype as { getAnimations?: () => Animation[] }).getAnimations = () => [];
}

if (!("ResizeObserver" in globalThis)) {
  globalThis.ResizeObserver = class {
    observe() {}
    unobserve() {}
    disconnect() {}
  } as unknown as typeof ResizeObserver;
}
