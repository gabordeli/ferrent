// "Add to Home Screen" nudge: the iOS Safari install path is buried in the
// share sheet, so the app explains it once and remembers the dismissal.

const DISMISSED = "ferrent.install.dismissed";

export function isIos(): boolean {
  const agent = navigator.userAgent;
  const iPad =
    navigator.platform === "MacIntel" && navigator.maxTouchPoints > 1;
  return /iphone|ipad|ipod/i.test(agent) || iPad;
}

export function isStandalone(): boolean {
  const mode = window.matchMedia?.("(display-mode: standalone)").matches;
  const legacy = (navigator as { standalone?: boolean }).standalone === true;
  return Boolean(mode) || legacy;
}

/// True on iOS outside the installed app; `?install=1` forces it for a check.
export function shouldOfferInstall(): boolean {
  if (localStorage.getItem(DISMISSED) === "1") return false;
  if (isStandalone()) return false;
  const forced = new URLSearchParams(window.location.search).has("install");
  return isIos() || forced;
}

export function dismissInstall(): void {
  localStorage.setItem(DISMISSED, "1");
}
