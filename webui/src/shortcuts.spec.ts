import { afterEach, describe, expect, it, vi } from "vitest";

import { installShortcuts } from "./shortcuts";

function setup() {
  const handlers = {
    pauseResume: vi.fn(),
    remove: vi.fn(),
    focusSearch: vi.fn(),
    refresh: vi.fn(),
    selectAll: vi.fn(),
    addTorrent: vi.fn(),
    escape: vi.fn(),
    help: vi.fn(),
    move: vi.fn(),
    home: vi.fn(),
    end: vi.fn(),
  };
  const dispose = installShortcuts(handlers);
  return { handlers, dispose };
}

function press(key: string, init: KeyboardEventInit = {}): KeyboardEvent {
  const event = new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true, ...init });
  document.body.dispatchEvent(event);
  return event;
}

afterEach(() => {
  document.body.innerHTML = "";
});

describe("installShortcuts", () => {
  it("maps the selection and navigation keys", () => {
    const { handlers, dispose } = setup();

    press(" ");
    press("Delete");
    press("ArrowDown");
    press("ArrowUp");
    press("Home");
    press("End");
    press("?");
    press("Escape");

    expect(handlers.pauseResume).toHaveBeenCalledOnce();
    expect(handlers.remove).toHaveBeenCalledOnce();
    expect(handlers.move).toHaveBeenNthCalledWith(1, 1);
    expect(handlers.move).toHaveBeenNthCalledWith(2, -1);
    expect(handlers.home).toHaveBeenCalledOnce();
    expect(handlers.end).toHaveBeenCalledOnce();
    expect(handlers.help).toHaveBeenCalledOnce();
    expect(handlers.escape).toHaveBeenCalledOnce();
    dispose();
  });

  it("handles the Ctrl combinations and prevents the browser default", () => {
    const { handlers, dispose } = setup();

    expect(press("f", { ctrlKey: true }).defaultPrevented).toBe(true);
    expect(press("r", { ctrlKey: true }).defaultPrevented).toBe(true);
    expect(press("a", { ctrlKey: true }).defaultPrevented).toBe(true);
    expect(press("o", { ctrlKey: true }).defaultPrevented).toBe(true);

    expect(handlers.focusSearch).toHaveBeenCalledOnce();
    expect(handlers.refresh).toHaveBeenCalledOnce();
    expect(handlers.selectAll).toHaveBeenCalledOnce();
    expect(handlers.addTorrent).toHaveBeenCalledOnce();
    dispose();
  });

  it("leaves typing alone except for Escape", () => {
    const { handlers, dispose } = setup();
    const input = document.createElement("input");
    document.body.append(input);
    input.focus();

    const space = new KeyboardEvent("keydown", {
      key: " ",
      bubbles: true,
      cancelable: true,
    });
    input.dispatchEvent(space);
    expect(handlers.pauseResume).not.toHaveBeenCalled();
    expect(space.defaultPrevented).toBe(false);

    input.dispatchEvent(
      new KeyboardEvent("keydown", { key: "Escape", bubbles: true }),
    );
    expect(handlers.escape).toHaveBeenCalledOnce();

    input.dispatchEvent(
      new KeyboardEvent("keydown", { key: "a", ctrlKey: true, bubbles: true }),
    );
    expect(handlers.selectAll).not.toHaveBeenCalled();
    dispose();
  });

  it("stops listening after dispose", () => {
    const { handlers, dispose } = setup();
    dispose();
    press(" ");
    expect(handlers.pauseResume).not.toHaveBeenCalled();
  });
});
