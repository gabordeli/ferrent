import { mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it } from "vitest";

import { setLocale, t } from "../i18n";
import ShortcutHelp from "./ShortcutHelp.vue";

function factory() {
  return mount(ShortcutHelp, { global: { stubs: { teleport: true } } });
}

beforeEach(() => {
  setLocale("en");
});

describe("ShortcutHelp", () => {
  it("lists every shortcut with its translated label", () => {
    const wrapper = factory();
    const keys = wrapper.findAll("kbd").map((item) => item.text());

    expect(keys).toEqual([
      "Space",
      "Delete",
      "Ctrl+F",
      "Ctrl+R",
      "Ctrl+A",
      "Ctrl+O",
      "↑ / ↓",
      "Home / End",
      "Esc",
      "?",
    ]);
    expect(wrapper.text()).toContain(t("shortcuts.pauseResume"));
    expect(wrapper.text()).toContain(t("shortcuts.help"));
    expect(wrapper.get("dialog").attributes("aria-modal")).toBe("true");
  });

  it("closes from the close button and from the backdrop", async () => {
    const wrapper = factory();

    await wrapper
      .get(`button[aria-label="${t("sheet.close")}"]`)
      .trigger("click");
    expect(wrapper.emitted("close")).toHaveLength(1);

    await wrapper.get(".sheet-backdrop").trigger("click");
    expect(wrapper.emitted("close")).toHaveLength(2);
  });

  it("closes on Escape and releases the focus trap", async () => {
    const wrapper = factory();

    await wrapper.get("dialog").trigger("keydown", { key: "Escape" });
    expect(wrapper.emitted("close")).toHaveLength(1);

    wrapper.unmount();
  });
});
