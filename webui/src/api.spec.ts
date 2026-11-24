import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import {
  ForbiddenError,
  UnauthorizedError,
  addLabel,
  addTorrentFile,
  addTracker,
  addTorrentMagnet,
  addTorrentUrl,
  bulkAction,
  eventsUrl,
  getAuthInfo,
  getHealth,
  getLabels,
  getPushConfig,
  getPushSubscriptions,
  getDaemonConfig,
  getSession,
  getToken,
  getTorrent,
  getTorrentFiles,
  getTorrentOptions,
  getTorrentPeers,
  getTorrentTrackers,
  getWebSeeds,
  listDirs,
  listTorrentIds,
  listTorrents,
  login,
  logout,
  moveTorrent,
  pauseSession,
  pauseTorrent,
  queueTorrent,
  reannounceTorrent,
  recheckTorrent,
  removeLabel,
  removePushSubscription,
  removeTorrent,
  removeTracker,
  renameFile,
  resumeSession,
  resumeTorrent,
  setFilePriorities,
  setDaemonConfig,
  setLabel,
  setSequential,
  setSessionLimits,
  setSessionQueue,
  setToken,
  setSuperSeeding,
  setTorrentLimits,
  setTorrentOptions,
  setWebSeeds,
  stateName,
  subscribePush,
  testPush,
  unsubscribePush,
  updateTorrent,
} from "./api";

const ROW = {
  id: 7,
  name: "movie.mkv",
  total_bytes: 1000,
  state: 3,
  progress: 0.5,
  download_rate: 12,
  upload_rate: 3,
  seeds: 1,
  peers: 2,
  eta_seconds: 30,
  save_path: "/data",
  label: "movies",
  error: "",
};

type Call = { url: string; init: RequestInit };

let calls: Call[] = [];

function mockFetch(body: unknown, status = 200) {
  calls = [];
  vi.stubGlobal(
    "fetch",
    vi.fn(async (url: string, init: RequestInit = {}) => {
      calls.push({ url, init });
      return new Response(JSON.stringify(body), {
        status,
        headers: { "content-type": "application/json" },
      });
    }),
  );
}

beforeEach(() => {
  localStorage.clear();
  mockFetch({});
});

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("request", () => {
  it("maps 401 and 403 to typed errors", async () => {
    mockFetch({ error: "login required" }, 401);
    await expect(getSession()).rejects.toBeInstanceOf(UnauthorizedError);

    mockFetch({ error: "read only" }, 403);
    await expect(pauseTorrent("1")).rejects.toBeInstanceOf(ForbiddenError);
  });

  it("surfaces the daemon message", async () => {
    mockFetch({ error: "label is not defined" }, 400);
    await expect(setLabel("1", "x")).rejects.toThrow("label is not defined");
  });
});

describe("torrents", () => {
  it("maps the filters and the rows", async () => {
    mockFetch({
      torrents: [
        {
          id: 7,
          name: "movie.mkv",
          total_bytes: 1000,
          state: 3,
          progress: 0.5,
          download_rate: 12,
          upload_rate: 3,
          seeds: 1,
          peers: 2,
          eta_seconds: 30,
          save_path: "/data",
          label: "movies",
          error: "",
        },
      ],
      total: 1,
      limit: 50,
    });
    const page = await listTorrents(
      { q: "movie", state: "Downloading", label: "movies", sort: "download_payload_rate" },
      0,
      50,
    );
    const url = decodeURIComponent(calls[0].url);
    expect(url).toContain("filter=movie");
    expect(url).toContain("state=downloading");
    expect(url).toContain("sort=download_rate");
    expect(url).toContain("label=movies");
    expect(page.total).toBe(1);
    expect(page.torrents[0]).toMatchObject({
      id: "7",
      state: "Downloading",
      progress: 50,
      download_payload_rate: 12,
      label: "movies",
    });
  });

  it("removes with the file flag and pauses via the REST routes", async () => {
    mockFetch({ id: 7, removed: true });
    await removeTorrent("7", true);
    expect(calls[0].url).toBe("/api/torrents/7?delete_files=true");
    expect(calls[0].init.method).toBe("DELETE");

    mockFetch({ id: 7, paused: true });
    await expect(pauseTorrent("7")).resolves.toEqual({ state: "Paused" });
    expect(calls[0].url).toBe("/api/torrents/7/pause");
  });

  it("adds a magnet with the staging options", async () => {
    mockFetch({ id: 9 });
    await addTorrentMagnet("magnet:?xt=urn:btih:aa", {
      label: "movies",
      download_location: "/data/_progress",
      add_paused: true,
    });
    const body = JSON.parse(String(calls[0].init.body));
    expect(body).toMatchObject({
      magnet: "magnet:?xt=urn:btih:aa",
      label: "movies",
      save_path: "/data/_progress",
      paused: true,
    });
  });

  it("uploads a .torrent as base64 metainfo", async () => {
    mockFetch({ id: 5 });
    const file = new File([new Uint8Array([1, 2, 3])], "x.torrent");
    await addTorrentFile(file, { label: "series" });
    const body = JSON.parse(String(calls[0].init.body));
    expect(body.metainfo).toBe(btoa(String.fromCharCode(1, 2, 3)));
    expect(body.name).toBe("x.torrent");
    expect(body.label).toBe("series");
  });

  it("bulk actions report the partial result", async () => {
    mockFetch({ ok: [1, 2], failed: [{ id: 3, error: "missing" }] });
    const result = await bulkAction({ action: "pause", torrent_ids: ["1", "2", "3"] });
    expect(calls[0].url).toBe("/api/torrents/bulk");
    expect(JSON.parse(String(calls[0].init.body))).toEqual({ action: "pause", ids: [1, 2, 3] });
    expect(result.ok).toBe(2);
    expect(result.failed[0]).toEqual({ id: "3", error: "missing" });
  });
});

describe("session, labels and events", () => {
  it("maps the status into the session shape", async () => {
    mockFetch({ torrents: 3, download_rate: 5, upload_rate: 2, free_space_bytes: 100 });
    const session = await getSession();
    expect(session.num_torrents).toBe(3);
    expect(session.payload_download_rate).toBe(5);
    expect(session.free_space).toBe(100);
  });

  it("counts the unlabelled torrents", async () => {
    calls = [];
    vi.stubGlobal(
      "fetch",
      vi.fn(async (url: string) => {
        calls.push({ url, init: {} });
        if (url === "/api/labels") {
          return new Response(
            JSON.stringify({ labels: [{ name: "movies", torrent_count: 2 }] }),
          );
        }
        return new Response(JSON.stringify({ torrents: 5 }));
      }),
    );
    const labels = await getLabels();
    expect(labels.labels).toEqual([{ id: "movies", count: 2 }]);
    expect(labels.no_label).toBe(3);
  });

  it("keeps the SSE URL cookie-based", () => {
    expect(eventsUrl()).toBe("/api/events");
  });

  it("logs in with the password only", async () => {
    mockFetch({ read_only: false, ttl: 60 });
    await login("web", "secret");
    expect(JSON.parse(String(calls[0].init.body))).toEqual({ password: "secret" });
  });
});

describe("mapping helpers", () => {
  it("names the states and merges SSE fields", () => {
    expect(stateName(4)).toBe("Seeding");
    expect(stateName(5)).toBe("Paused");
    const row = {
      id: "1",
      name: "x",
      state: "Queued",
      progress: 0,
      label: "",
      download_payload_rate: 0,
      upload_payload_rate: 0,
      eta: 0,
      ratio: 0,
      total_size: 0,
      save_path: "",
      time_added: 0,
      message: "",
    };
    updateTorrent(row, { state: 3, progress: 0.25, download_rate: 9, label: "movies" });
    expect(row.state).toBe("Downloading");
    expect(row.progress).toBe(25);
    expect(row.download_payload_rate).toBe(9);
    expect(row.label).toBe("movies");
  });
});

describe("token", () => {
  it("stores and reads the cookie marker", () => {
    setToken("cookie");
    expect(getToken()).toBe("cookie");
    setToken("");
    expect(getToken()).toBe("");
  });
});

describe("gateway compatibility shims", () => {
  it("reports health and the auth mode without a daemon round trip", async () => {
    mockFetch({ ok: true });
    await expect(getHealth()).resolves.toMatchObject({ ok: true, port: null });
    await expect(getAuthInfo()).resolves.toMatchObject({
      mode: "password",
      modes: ["password"],
    });
    await expect(pauseSession()).resolves.toEqual({ paused: false });
    await expect(resumeSession()).resolves.toEqual({ paused: false });
    await expect(
      setSessionLimits({ max_download_speed: 5, max_upload_speed: 2 }),
    ).resolves.toEqual({ max_download_speed: 5, max_upload_speed: 2 });
    await expect(setSessionQueue({ max_active_downloading: 3 })).resolves.toEqual({
      max_active_downloading: 3,
    });
  });

  it("adds and removes a label over the REST routes", async () => {
    mockFetch({ name: "series" });
    await expect(addLabel("series")).resolves.toEqual({ id: "series" });
    expect(calls[0].url).toBe("/api/labels");
    expect(JSON.parse(String(calls[0].init.body))).toEqual({ name: "series" });

    mockFetch({ removed: true });
    await expect(removeLabel("series")).resolves.toEqual({
      id: "series",
      removed: true,
    });
    expect(calls[0].url).toBe("/api/labels/series");
    expect(calls[0].init.method).toBe("DELETE");
  });

  it("clears the token on logout", async () => {
    mockFetch({});
    setToken("cookie");
    await expect(logout()).resolves.toEqual({ ok: true, session: false });
    expect(getToken()).toBe("");
  });

  it("resumes, queues and moves torrents", async () => {
    mockFetch({});
    await expect(resumeTorrent("7")).resolves.toEqual({ state: "Queued" });
    expect(calls[0].url).toBe("/api/torrents/7/resume");
    await expect(queueTorrent("7", "top")).resolves.toEqual({ action: "top" });
    await expect(reannounceTorrent("7")).resolves.toEqual({
      id: "7",
      reannounced: false,
    });
    await expect(recheckTorrent("7")).resolves.toEqual({
      id: "7",
      rechecked: false,
    });
    await expect(moveTorrent("7", "/data")).resolves.toEqual({
      id: "7",
      destination: "/data",
      moving: false,
    });
  });

  it("only accepts magnet URLs from the URL field", async () => {
    mockFetch({ id: 9 });
    await expect(addTorrentUrl("http://example.test/x.torrent")).rejects.toThrow(
      "magnet",
    );
    await expect(addTorrentUrl("magnet:?xt=urn:btih:aa")).resolves.toEqual({
      id: "9",
    });
    expect(calls[0].url).toBe("/api/torrents");
  });

  it("maps a single torrent and its files", async () => {
    mockFetch(ROW);
    const detail = await getTorrent("7");
    expect(detail).toMatchObject({ id: "7", progress: 50, num_peers: 2 });

    mockFetch({
      files: [
        { path: "a.mkv", size: 10, progress: 1, priority: 4 },
        { path: "b.srt", size: 5, progress: 1, priority: 0 },
      ],
    });
    const files = await getTorrentFiles("7", 0, 10);
    expect(files.total).toBe(2);
    expect(files.files[1]).toMatchObject({ path: "b.srt", offset: 10 });
    const filtered = await getTorrentFiles("7", 0, 10, "B.");
    expect(filtered.total).toBe(1);
  });

  it("lists ids with the paging filters", async () => {
    mockFetch({ torrents: [ROW], total: 1 });
    await expect(
      listTorrentIds({
        q: "movie",
        state: "Downloading",
        label: "movies",
        sort: "id",
      }),
    ).resolves.toEqual({ ids: ["7"], total: 1, limit: 1 });
    const url = decodeURIComponent(calls[0].url);
    expect(url).toContain("limit=1000");
    expect(url).toContain("filter=movie");
    expect(url).toContain("state=downloading");
  });

  it("keeps the unimplemented endpoints neutral", async () => {
    await expect(listDirs("/data")).resolves.toMatchObject({
      path: "/data",
      parent: "/",
      dirs: [],
    });
    await expect(listDirs("/")).resolves.toMatchObject({ parent: null });
    await expect(
      setFilePriorities("7", { files: [{ index: 0, priority: 7 }] }),
    ).resolves.toEqual({ id: "7", priorities: [7] });
    await expect(
      setTorrentLimits("7", { max_download_speed: 1 }),
    ).resolves.toEqual({ id: "7", max_download_speed: 1, max_upload_speed: 0 });
  });
});

describe("push endpoints", () => {
  it("reads the VAPID config and manages subscriptions", async () => {
    mockFetch({ enabled: true, public_key: "AQAB", subscriptions: 1 });
    await expect(getPushConfig()).resolves.toEqual({
      enabled: true,
      public_key: "AQAB",
      subscriptions: 1,
    });
    expect(calls[0].url).toBe("/api/push/config");

    const subscription = {
      endpoint: "https://push.test/1",
      keys: { p256dh: "key", auth: "auth" },
    };
    mockFetch({ count: 1 });
    await expect(subscribePush(subscription)).resolves.toEqual({ count: 1 });
    expect(calls[0].url).toBe("/api/push/subscribe");
    expect(calls[0].init.method).toBe("POST");
    expect(JSON.parse(String(calls[0].init.body))).toEqual(subscription);

    mockFetch({ unsubscribed: true });
    await expect(unsubscribePush("https://push.test/1")).resolves.toEqual({
      unsubscribed: true,
    });
    expect(calls[0].init.method).toBe("DELETE");
    expect(JSON.parse(String(calls[0].init.body))).toEqual({
      endpoint: "https://push.test/1",
    });

    mockFetch({ queued: true });
    await expect(testPush("https://push.test/1")).resolves.toEqual({
      queued: true,
    });
    expect(calls[0].url).toBe("/api/push/test");

    mockFetch({
      subscriptions: [
        {
          id: "s1",
          host: "push.test",
          added: 1,
          last_seen: 2,
          user_agent: "UA",
        },
      ],
    });
    await expect(getPushSubscriptions()).resolves.toMatchObject({
      subscriptions: [{ id: "s1", host: "push.test" }],
    });
    expect(calls[0].url).toBe("/api/push/subscriptions");

    mockFetch({ unsubscribed: true });
    await expect(removePushSubscription("s1")).resolves.toEqual({
      unsubscribed: true,
    });
    expect(calls[0].url).toBe("/api/push/subscriptions/s1");
    expect(calls[0].init.method).toBe("DELETE");
  });
});

describe("daemon detail endpoints", () => {
  it("reads peers, trackers and web seeds", async () => {
    mockFetch({
      peers: [
        {
          endpoint: "1.2.3.4:51413",
          client: "qBittorrent",
          progress: 0.5,
          download_rate: 10,
          upload_rate: 2,
        },
      ],
    });
    await expect(getTorrentPeers("7")).resolves.toMatchObject({
      peers: [{ endpoint: "1.2.3.4:51413" }],
    });
    expect(calls[0].url).toBe("/api/torrents/7/peers");

    mockFetch({ id: 7, trackers: [{ url: "udp://t:1", status: "working" }] });
    await expect(getTorrentTrackers("7")).resolves.toMatchObject({
      trackers: [{ url: "udp://t:1", status: "working" }],
    });
    expect(calls[0].url).toBe("/api/torrents/7/trackers");

    mockFetch({ id: 7, web_seeds: ["https://m/x.iso"] });
    await expect(getWebSeeds("7")).resolves.toMatchObject({
      web_seeds: ["https://m/x.iso"],
    });
    expect(calls[0].url).toBe("/api/torrents/7/webseeds");
  });

  it("adds and removes a tracker, replaces the web seeds", async () => {
    mockFetch({ id: 7, trackers: [] });
    await addTracker("7", "udp://t:1");
    expect(calls[0].init.method).toBe("POST");
    expect(JSON.parse(String(calls[0].init.body))).toEqual({ url: "udp://t:1" });

    mockFetch({ id: 7, trackers: [] });
    await removeTracker("7", "udp://t:1");
    expect(calls[0].init.method).toBe("DELETE");
    expect(calls[0].url).toBe("/api/torrents/7/trackers?url=udp%3A%2F%2Ft%3A1");

    mockFetch({ id: 7, web_seeds: ["https://m/x.iso"] });
    await setWebSeeds("7", ["https://m/x.iso"]);
    expect(calls[0].init.method).toBe("PUT");
    expect(JSON.parse(String(calls[0].init.body))).toEqual({
      urls: ["https://m/x.iso"],
    });
  });

  it("toggles sequential and super-seeding on their own endpoints", async () => {
    mockFetch({ id: 7, sequential: true });
    await setSequential("7", true);
    expect(calls[0].url).toBe("/api/torrents/7/sequential");
    expect(JSON.parse(String(calls[0].init.body))).toEqual({ on: true });

    mockFetch({ id: 7, super_seeding: false });
    await setSuperSeeding("7", false);
    expect(calls[0].url).toBe("/api/torrents/7/super-seeding");
    expect(JSON.parse(String(calls[0].init.body))).toEqual({ on: false });
  });

  it("reads the options from the torrent row", async () => {
    mockFetch({ ...ROW, sequential: true, super_seeding: true });
    const result = await getTorrentOptions("7");
    expect(result.options.sequential_download).toBe(true);
    expect(result.options.super_seeding).toBe(true);
    expect(calls[0].url).toBe("/api/torrents/7");
  });

  it("renames a file and edits the daemon config", async () => {
    mockFetch({ id: 7, files: [{ path: "new.mkv" }] });
    await renameFile("7", 1, "new.mkv");
    expect(calls[0].url).toBe("/api/torrents/7/rename-file");
    expect(JSON.parse(String(calls[0].init.body))).toEqual({
      index: 1,
      name: "new.mkv",
    });

    mockFetch({ revision: 4, env_locked: ["bind"], toml: "a = 1" });
    await expect(getDaemonConfig()).resolves.toMatchObject({ revision: 4 });
    expect(calls[0].url).toBe("/api/config");

    mockFetch({ revision: 5 });
    await setDaemonConfig("a = 2", 4);
    expect(calls[0].init.method).toBe("PUT");
    expect(JSON.parse(String(calls[0].init.body))).toEqual({
      toml: "a = 2",
      revision: 4,
    });
  });
});
