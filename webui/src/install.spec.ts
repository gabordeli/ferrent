import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { dismissInstall, isIos, isStandalone, shouldOfferInstall } from "./install";

function setAgent(agent: string) {
  vi.stubGlobal("navigator", {
    ...navigator,
    userAgent: agent,
    platform: "iPhone",
    maxTouchPoints: 5,
  });
}

beforeEach(() => {
  localStorage.clear();
});

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("install hint", () => {
  it("detects iOS", () => {
    setAgent("Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X)");
    expect(isIos()).toBe(true);
    setAgent("Mozilla/5.0 (X11; Linux x86_64)");
    expect(isIos()).toBe(false);
  });

  it("offers the hint to iOS Safari outside the installed app", () => {
    setAgent("Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X)");
    expect(shouldOfferInstall()).toBe(true);
    dismissInstall();
    expect(shouldOfferInstall()).toBe(false);
  });

  it("stays quiet for a desktop browser", () => {
    setAgent("Mozilla/5.0 (X11; Linux x86_64)");
    expect(shouldOfferInstall()).toBe(false);
  });

  it("returns true when the display mode is standalone", () => {
    vi.stubGlobal("matchMedia", (query: string) => ({
      matches: query === "(display-mode: standalone)",
      media: query,
      addEventListener: () => {},
      removeEventListener: () => {},
    }));
    expect(isStandalone()).toBe(true);
  });
});
