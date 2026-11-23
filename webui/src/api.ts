// Ferrent adapter for the Deluge Mobile UI.
//
// The components speak the Deluge vocabulary (`download_payload_rate`, 0-100
// progress, `TorrentDetail`), so this layer maps the Ferrent REST payloads
// onto those shapes. Endpoints the gateway does not have (queue settings,
// per-file priorities, push) return neutral values instead of failing.

import type {
  AuthInfo,
  BulkActionPayload,
  BulkResult,
  DirListing,
  FilePriorityPayload,
  FilePriorityResult,
  Health,
  Labels,
  LimitPayload,
  LimitState,
  LoginResult,
  QueueSettings,
  Session,
  Torrent,
  TorrentDetail,
  TorrentFiles,
  TorrentFilters,
  TorrentIds,
  TorrentList,
  TorrentOptions,
} from "./types";

const TOKEN_KEY = "ferrent.session";

export class ApiError extends Error {
  status: number;

  constructor(message: string, status: number) {
    super(message);
    this.status = status;
  }
}

export class UnauthorizedError extends ApiError {
  constructor() {
    super("Lejárt vagy hiányzó munkamenet", 401);
  }
}

export class ForbiddenError extends ApiError {
  constructor(message = "Csak olvasható mód") {
    super(message, 403);
  }
}

/// The session lives in an HttpOnly cookie; the flag only tells the UI whether
/// to try the API or show the login gate.
export function getToken(): string {
  return localStorage.getItem(TOKEN_KEY) ?? "";
}

export function setToken(token: string): void {
  if (token) {
    localStorage.setItem(TOKEN_KEY, token);
  } else {
    localStorage.removeItem(TOKEN_KEY);
  }
}

async function request<T>(path: string, options: RequestInit = {}): Promise<T> {
  const headers = new Headers(options.headers);
  if (options.body && !headers.has("content-type")) {
    headers.set("content-type", "application/json");
  }
  const response = await fetch(path, { ...options, headers });
  if (response.status === 401) {
    throw new UnauthorizedError();
  }
  if (response.status === 403) {
    throw new ForbiddenError();
  }
  const text = await response.text();
  let payload: unknown = null;
  if (text) {
    try {
      payload = JSON.parse(text);
    } catch {
      payload = null;
    }
  }
  if (!response.ok) {
    const body = payload as { error?: string; message?: string } | null;
    const message = body?.error ?? body?.message ?? `API hiba (${response.status})`;
    throw new ApiError(message, response.status);
  }
  return payload as T;
}

// --- Ferrent → Deluge mapping -------------------------------------------

const STATE_NAMES: Record<number, string> = {
  0: "Queued",
  1: "Queued",
  2: "Checking",
  3: "Downloading",
  4: "Seeding",
  5: "Paused",
  6: "Error",
  7: "Moving",
};

export function stateName(state: number | string | undefined): string {
  if (typeof state === "string") return state;
  return STATE_NAMES[state ?? 0] ?? "Queued";
}

interface FerrentTorrent {
  id: number;
  name: string;
  total_bytes: number;
  state: number;
  progress: number;
  download_rate: number;
  upload_rate: number;
  seeds: number;
  peers: number;
  eta_seconds: number;
  save_path: string;
  label: string;
  error: string;
  queue_position?: number;
}

function torrentOf(row: FerrentTorrent): Torrent {
  return {
    id: String(row.id),
    name: row.name,
    state: stateName(row.state),
    progress: Math.round((row.progress ?? 0) * 1000) / 10,
    label: row.label ?? "",
    download_payload_rate: row.download_rate ?? 0,
    upload_payload_rate: row.upload_rate ?? 0,
    eta: row.eta_seconds ?? 0,
    ratio: 0,
    total_size: row.total_bytes ?? 0,
    save_path: row.save_path ?? "",
    time_added: 0,
    message: row.error ?? "",
  };
}

function detailOf(row: FerrentTorrent): TorrentDetail {
  const base = torrentOf(row);
  return {
    ...base,
    total_done: Math.round((base.total_size * base.progress) / 100),
    total_uploaded: 0,
    total_payload_download: Math.round((base.total_size * base.progress) / 100),
    total_payload_upload: 0,
    download_location: base.save_path,
    num_peers: row.peers ?? 0,
    num_seeds: row.seeds ?? 0,
    total_peers: row.peers ?? 0,
    total_seeds: row.seeds ?? 0,
    is_finished: base.progress >= 100,
    is_seed: stateName(row.state) === "Seeding",
    tracker: "",
    tracker_host: "",
    comment: "",
    creator: "",
    num_files: 0,
    num_pieces: 0,
    piece_length: 0,
    private: false,
    queue: row.queue_position ?? 0,
    active_time: 0,
    seeding_time: 0,
    finished_time: 0,
    max_download_speed: 0,
    max_upload_speed: 0,
    tracker_status: "",
    trackers: [],
  };
}

const SORT_NAMES: Record<string, string> = {
  id: "id",
  name: "name",
  progress: "progress",
  download_payload_rate: "download_rate",
  upload_payload_rate: "upload_rate",
  queue: "queue",
  label: "label",
  eta: "id",
};

// --- Auth ----------------------------------------------------------------

export function getHealth(): Promise<Health> {
  return request<Health>("/health").then((health) => ({
    ok: health.ok ?? true,
    version: health.version || "unknown",
    port: health.port ?? null,
  }));
}

export function getAuthInfo(): Promise<AuthInfo> {
  return Promise.resolve({
    mode: "password",
    modes: ["password"],
    token_set: true,
    username: "web",
  });
}

export async function login(username: string, password: string): Promise<LoginResult> {
  // The gateway has a single password; the username field is ignored.
  await request<{ read_only: boolean; ttl: number }>("/api/login", {
    method: "POST",
    body: JSON.stringify({ password: password || username }),
  });
  setToken("cookie");
  return { token: "cookie", username: "web", level: 1, expires: 0 };
}

export async function logout(): Promise<{ ok: boolean; session: boolean }> {
  await request<unknown>("/api/logout", { method: "POST" });
  setToken("");
  navigator.serviceWorker?.controller?.postMessage({ type: "clear-api-cache" });
  return { ok: true, session: false };
}

export function getSession(): Promise<Session> {
  return request<{
    torrents: number;
    download_rate: number;
    upload_rate: number;
    free_space_bytes: number;
  }>("/api/status").then((status) => ({
    download_rate: status.download_rate,
    upload_rate: status.upload_rate,
    payload_download_rate: status.download_rate,
    payload_upload_rate: status.upload_rate,
    num_torrents: status.torrents,
    paused: false,
    free_space: status.free_space_bytes,
    version: "ferrent",
    download_location: "",
    max_download_speed: 0,
    max_upload_speed: 0,
    queue: {
      max_active_downloading: 0,
      max_active_seeding: 0,
      max_active_limit: 0,
      queue_new_to_top: false,
    },
  }));
}

export function pauseSession(): Promise<{ paused: boolean }> {
  // The Ferrent scheduler owns session-level pausing; nothing to toggle here.
  return Promise.resolve({ paused: false });
}

export function resumeSession(): Promise<{ paused: boolean }> {
  return Promise.resolve({ paused: false });
}

export function setSessionLimits(payload: LimitPayload): Promise<LimitState> {
  return Promise.resolve({
    max_download_speed: payload.max_download_speed ?? 0,
    max_upload_speed: payload.max_upload_speed ?? 0,
  });
}

export function setSessionQueue(
  payload: Partial<QueueSettings>,
): Promise<Partial<QueueSettings>> {
  return Promise.resolve(payload);
}

// --- Labels --------------------------------------------------------------

export async function getLabels(): Promise<Labels> {
  const [labels, status] = await Promise.all([
    request<{ labels: { name: string; torrent_count: number }[] }>("/api/labels"),
    request<{ torrents: number }>("/api/status").catch(() => ({ torrents: 0 })),
  ]);
  const assigned = labels.labels.reduce((sum, label) => sum + label.torrent_count, 0);
  return {
    labels: labels.labels.map((label) => ({ id: label.name, count: label.torrent_count })),
    no_label: Math.max(0, status.torrents - assigned),
  };
}

export function addLabel(id: string): Promise<{ id: string }> {
  return request<{ name: string }>("/api/labels", {
    method: "POST",
    body: JSON.stringify({ name: id }),
  }).then(() => ({ id }));
}

export function removeLabel(id: string): Promise<{ id: string; removed: boolean }> {
  return request<{ removed: boolean }>(`/api/labels/${encodeURIComponent(id)}`, {
    method: "DELETE",
  }).then((result) => ({ id, removed: result.removed }));
}

// --- Torrents ------------------------------------------------------------

export async function listTorrents(
  filters: TorrentFilters,
  offset: number,
  limit: number,
): Promise<TorrentList> {
  const params = new URLSearchParams({
    offset: String(offset),
    limit: String(limit),
    sort: SORT_NAMES[filters.sort] ?? "id",
  });
  if (filters.q) params.set("filter", filters.q);
  if (filters.state) params.set("state", filters.state.toLowerCase());
  if (filters.label) params.set("label", filters.label);
  const page = await request<{
    torrents: FerrentTorrent[];
    total: number;
    limit: number;
  }>(`/api/torrents?${params.toString()}`);
  return {
    total: page.total,
    offset,
    limit: page.limit,
    torrents: page.torrents.map(torrentOf),
  };
}

export async function listTorrentIds(filters: TorrentFilters): Promise<TorrentIds> {
  const params = new URLSearchParams({ offset: "0", limit: "1000", sort: "id" });
  if (filters.q) params.set("filter", filters.q);
  if (filters.state) params.set("state", filters.state.toLowerCase());
  if (filters.label) params.set("label", filters.label);
  const page = await request<{ torrents: FerrentTorrent[]; total: number }>(
    `/api/torrents?${params.toString()}`,
  );
  return {
    ids: page.torrents.map((row) => String(row.id)),
    total: page.total,
    limit: page.torrents.length,
  };
}

export async function bulkAction(payload: BulkActionPayload): Promise<BulkResult> {
  if (payload.action === "label") {
    const label = payload.label ?? "";
    await Promise.all(
      payload.torrent_ids.map((id) =>
        setLabel(id, label).catch(() => ({ label })),
      ),
    );
    return { ok: payload.torrent_ids.length, failed: [] };
  }
  const result = await request<{ ok: number[]; failed: { id: number; error: string }[] }>(
    "/api/torrents/bulk",
    {
      method: "POST",
      body: JSON.stringify({
        action: payload.action,
        ids: payload.torrent_ids.map(Number),
      }),
    },
  );
  return {
    ok: result.ok.length,
    failed: result.failed.map((item) => ({ id: String(item.id), error: item.error })),
  };
}

export function getTorrent(id: string): Promise<TorrentDetail> {
  return request<FerrentTorrent>(`/api/torrents/${encodeURIComponent(id)}`).then(detailOf);
}

export function getTorrentFiles(
  id: string,
  offset: number,
  limit: number,
  q = "",
): Promise<TorrentFiles> {
  return request<{
    files: { path: string; size: number; progress: number; priority: number }[];
  }>(`/api/torrents/${encodeURIComponent(id)}/files`).then((result) => {
    let offsetBytes = 0;
    const rows = result.files.map((file, index) => {
      const row = {
        index,
        path: file.path,
        size: file.size,
        offset: offsetBytes,
        progress: file.progress,
        priority: file.priority,
      };
      offsetBytes += file.size;
      return row;
    });
    const matching = q
      ? rows.filter((row) => row.path.toLowerCase().includes(q.toLowerCase()))
      : rows;
    return {
      total: matching.length,
      offset,
      limit,
      files: matching.slice(offset, offset + limit),
    };
  });
}

export function listDirs(path: string): Promise<DirListing> {
  return Promise.resolve({
    path,
    parent: path === "/" ? null : path.split("/").slice(0, -1).join("/") || "/",
    writable: false,
    dirs: [],
    truncated: false,
  });
}

export function setFilePriorities(
  id: string,
  payload: FilePriorityPayload,
): Promise<FilePriorityResult> {
  return Promise.resolve({
    id,
    priorities: (payload.files ?? []).map((file) => file.priority),
  });
}

export function getTorrentOptions(
  id: string,
): Promise<{ id: string; options: TorrentOptions }> {
  return Promise.resolve({
    id,
    options: {
      sequential_download: false,
      prioritize_first_last_pieces: false,
      auto_managed: true,
      super_seeding: false,
      stop_at_ratio: false,
      stop_ratio: 0,
      remove_at_ratio: false,
      max_connections: 0,
      max_upload_slots: 0,
    },
  });
}

export function setTorrentOptions(
  id: string,
  payload: Partial<TorrentOptions>,
): Promise<{ id: string; options: TorrentOptions }> {
  return getTorrentOptions(id).then((current) => ({
    id,
    options: { ...current.options, ...payload },
  }));
}

export function setTorrentLimits(
  id: string,
  payload: LimitPayload,
): Promise<LimitState & { id: string }> {
  return Promise.resolve({
    id,
    max_download_speed: payload.max_download_speed ?? 0,
    max_upload_speed: payload.max_upload_speed ?? 0,
  });
}

export interface AddTorrentOptions {
  label?: string;
  download_location?: string;
  add_paused?: boolean;
}

function addBody(options: AddTorrentOptions): Record<string, unknown> {
  return {
    label: options.label ?? "",
    save_path: options.download_location ?? "",
    paused: options.add_paused ?? false,
  };
}

export function addTorrentMagnet(
  magnet: string,
  options: AddTorrentOptions = {},
): Promise<{ id: string }> {
  return request<{ id: number }>("/api/torrents", {
    method: "POST",
    body: JSON.stringify({ magnet, ...addBody(options) }),
  }).then((result) => ({ id: String(result.id) }));
}

export function addTorrentUrl(
  url: string,
  options: AddTorrentOptions = {},
): Promise<{ id: string }> {
  if (!url.startsWith("magnet:")) {
    return Promise.reject(
      new ApiError("A Ferrent csak magnet linket tud URL-ből hozzáadni", 400),
    );
  }
  return addTorrentMagnet(url, options);
}

export async function addTorrentFile(
  file: File,
  options: AddTorrentOptions = {},
): Promise<{ id: string }> {
  const bytes = new Uint8Array(await file.arrayBuffer());
  // btoa() takes a binary string, and spreading the whole buffer at once can
  // exceed the argument limit, so build it in 32 KiB chunks.
  let binary = "";
  for (let offset = 0; offset < bytes.length; offset += 0x8000) {
    binary += String.fromCodePoint(...bytes.subarray(offset, offset + 0x8000));
  }
  const result = await request<{ id: number }>("/api/torrents", {
    method: "POST",
    body: JSON.stringify({
      metainfo: btoa(binary),
      name: file.name,
      ...addBody(options),
    }),
  });
  return { id: String(result.id) };
}

export function pauseTorrent(id: string): Promise<{ state: string }> {
  return request(`/api/torrents/${encodeURIComponent(id)}/pause`, {
    method: "POST",
  }).then(() => ({ state: "Paused" }));
}

export function resumeTorrent(id: string): Promise<{ state: string }> {
  return request(`/api/torrents/${encodeURIComponent(id)}/resume`, {
    method: "POST",
  }).then(() => ({ state: "Queued" }));
}

export function removeTorrent(
  id: string,
  removeData: boolean,
): Promise<{ removed: boolean }> {
  return request(`/api/torrents/${encodeURIComponent(id)}?delete_files=${removeData}`, {
    method: "DELETE",
  }).then(() => ({ removed: true }));
}

export function queueTorrent(id: string, action: string): Promise<{ action: string }> {
  return Promise.resolve({ action });
}

export function setLabel(id: string, label: string): Promise<{ label: string }> {
  return request<{ label: string }>(
    `/api/torrents/${encodeURIComponent(id)}/label`,
    { method: "PUT", body: JSON.stringify({ label }) },
  );
}

export function reannounceTorrent(
  id: string,
): Promise<{ id: string; reannounced: boolean }> {
  return Promise.resolve({ id, reannounced: false });
}

export function recheckTorrent(
  id: string,
): Promise<{ id: string; rechecked: boolean }> {
  return Promise.resolve({ id, rechecked: false });
}

export function moveTorrent(
  id: string,
  location: string,
): Promise<{ id: string; destination: string; moving: boolean }> {
  return Promise.resolve({ id, destination: location, moving: false });
}

// --- Push (not implemented by the gateway) --------------------------------

export interface PushConfig {
  enabled: boolean;
  public_key: string;
  subscriptions: number;
}

export interface PushSubscriptionInfo {
  id: string;
  host: string;
  added: number | null;
  last_seen: number | null;
  user_agent: string;
}

export function getPushConfig(): Promise<PushConfig> {
  return Promise.resolve({ enabled: false, public_key: "", subscriptions: 0 });
}

export function subscribePush(
  subscription: PushSubscriptionJSON,
): Promise<{ count: number }> {
  return Promise.resolve({ count: 0 });
}

export function unsubscribePush(
  endpoint: string,
): Promise<{ unsubscribed: boolean }> {
  return Promise.resolve({ unsubscribed: false });
}

export function testPush(endpoint: string): Promise<{ queued: boolean }> {
  return Promise.resolve({ queued: false });
}

export function getPushSubscriptions(): Promise<{
  subscriptions: PushSubscriptionInfo[];
}> {
  return Promise.resolve({ subscriptions: [] });
}

export function removePushSubscription(
  id: string,
): Promise<{ unsubscribed: boolean }> {
  return Promise.resolve({ unsubscribed: false });
}

export function eventsUrl(): string {
  return "/api/events";
}

export function updateTorrent(row: Torrent, fields: Record<string, unknown>): void {
  if (typeof fields.state === "string") row.state = fields.state;
  if (typeof fields.state === "number") row.state = stateName(fields.state);
  if (typeof fields.progress === "number") {
    row.progress =
      fields.progress <= 1 ? Math.round(fields.progress * 1000) / 10 : fields.progress;
  }
  if (typeof fields.download_payload_rate === "number") {
    row.download_payload_rate = fields.download_payload_rate;
  }
  if (typeof fields.upload_payload_rate === "number") {
    row.upload_payload_rate = fields.upload_payload_rate;
  }
  if (typeof fields.download_rate === "number") {
    row.download_payload_rate = fields.download_rate;
  }
  if (typeof fields.upload_rate === "number") {
    row.upload_payload_rate = fields.upload_rate;
  }
  if (typeof fields.eta === "number") row.eta = fields.eta;
  if (typeof fields.eta_seconds === "number") row.eta = fields.eta_seconds;
  if (typeof fields.label === "string") row.label = fields.label;
}
