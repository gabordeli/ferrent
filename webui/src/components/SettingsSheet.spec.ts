import { flushPromises, mount } from "@vue/test-utils";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import * as api from "../api";
import { setLocale, t } from "../i18n";
import SettingsSheet from "./SettingsSheet.vue";

vi.mock("../api", () => ({
  getDaemonConfig: vi.fn(),
  setDaemonConfig: vi.fn(),
}));

const DOC = '[general]\ndata_dir = "/var/lib/ferrent"\n';

function factory(canWrite = true) {
  return mount(SettingsSheet, {
    props: { canWrite },
    global: { stubs: { teleport: true } },
  });
}

beforeEach(() => {
  setLocale("hu");
  vi.mocked(api.getDaemonConfig).mockResolvedValue({
    revision: 3,
    env_locked: ["bind"],
    toml: DOC,
  });
  vi.mocked(api.setDaemonConfig).mockResolvedValue({ revision: 4 });
});

afterEach(() => {
  vi.clearAllMocks();
});

describe("SettingsSheet", () => {
  it("loads the document, the revision and the locked keys", async () => {
    const wrapper = factory();
    await flushPromises();

    const textarea = wrapper.get("textarea").element as HTMLTextAreaElement;
    expect(textarea.value).toBe(DOC);
    expect(textarea.readOnly).toBe(false);
    expect(wrapper.text()).toContain("3");
    expect(wrapper.text()).toContain("bind");
  });

  it("saves the draft against the revision it started from", async () => {
    const wrapper = factory();
    await flushPromises();

    const draft = '[general]\ndata_dir = "/data"\n';
    await wrapper.get("textarea").setValue(draft);
    const save = wrapper.get(".sheet-actions .primary");
    expect(save.attributes("disabled")).toBeUndefined();
    await save.trigger("click");
    await flushPromises();

    expect(api.setDaemonConfig).toHaveBeenCalledWith(draft, 3);
    expect(wrapper.emitted("toast")?.[0]).toEqual([t("settings.saved")]);
  });

  it("keeps the draft and shows a stale revision error", async () => {
    vi.mocked(api.setDaemonConfig).mockRejectedValueOnce(
      new Error("config revision 2 is stale (current 3)"),
    );
    const wrapper = factory();
    await flushPromises();

    const draft = '[general]\ndata_dir = "/data"\n';
    await wrapper.get("textarea").setValue(draft);
    await wrapper.get(".sheet-actions .primary").trigger("click");
    await flushPromises();

    expect(wrapper.get(".detail-error").text()).toContain("stale");
    expect((wrapper.get("textarea").element as HTMLTextAreaElement).value).toBe(draft);
  });

  it("is read-only without a write account", async () => {
    const wrapper = factory(false);
    await flushPromises();

    expect((wrapper.get("textarea").element as HTMLTextAreaElement).readOnly).toBe(
      true,
    );
    expect(wrapper.find(".sheet-actions").exists()).toBe(false);
    expect(wrapper.text()).toContain(t("auth.readonly"));
  });
});
