// Global keyboard shortcuts, matching the desktop client's list. The module
// is pure DOM: the app passes the actions, so the bindings stay testable
// without mounting the whole shell.

export interface ShortcutHandlers {
  pauseResume: () => void;
  remove: () => void;
  focusSearch: () => void;
  refresh: () => void;
  selectAll: () => void;
  addTorrent: () => void;
  escape: () => void;
  help: () => void;
  move: (delta: number) => void;
  home: () => void;
  end: () => void;
}

function isTyping(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  const tag = target.tagName;
  return (
    tag === "INPUT" ||
    tag === "TEXTAREA" ||
    tag === "SELECT" ||
    target.isContentEditable
  );
}

/// Key combinations that work even while a field has focus.
function isGlobalCombo(event: KeyboardEvent): boolean {
  return (event.ctrlKey || event.metaKey) && !event.altKey;
}

export function installShortcuts(handlers: ShortcutHandlers): () => void {
  function onKey(event: KeyboardEvent): void {
    const typing = isTyping(event.target);
    const combo = isGlobalCombo(event);

    if (combo) {
      switch (event.key.toLowerCase()) {
        case "f":
          event.preventDefault();
          handlers.focusSearch();
          return;
        case "r":
          event.preventDefault();
          handlers.refresh();
          return;
        case "a":
          if (typing) return;
          event.preventDefault();
          handlers.selectAll();
          return;
        case "o":
          event.preventDefault();
          handlers.addTorrent();
          return;
        default:
          return;
      }
    }

    if (typing) {
      if (event.key === "Escape") handlers.escape();
      return;
    }

    switch (event.key) {
      case " ":
        event.preventDefault();
        handlers.pauseResume();
        break;
      case "Delete":
      case "Backspace":
        event.preventDefault();
        handlers.remove();
        break;
      case "ArrowDown":
        event.preventDefault();
        handlers.move(1);
        break;
      case "ArrowUp":
        event.preventDefault();
        handlers.move(-1);
        break;
      case "Home":
        event.preventDefault();
        handlers.home();
        break;
      case "End":
        event.preventDefault();
        handlers.end();
        break;
      case "Escape":
        handlers.escape();
        break;
      case "?":
        event.preventDefault();
        handlers.help();
        break;
      default:
        break;
    }
  }

  window.addEventListener("keydown", onKey);
  return () => window.removeEventListener("keydown", onKey);
}
