import { expect, test, type Page, type Route } from "@playwright/test";

const status = {
  torrents: 2,
  download_rate: 1024 * 1024,
  upload_rate: 2048,
  free_space_bytes: 8 * 1024 ** 3,
  states: {},
  scheduler: { enabled: false, level: "", next_unix: 0 },
};

const labels = {
  labels: [{ name: "movies", color: "#2f81f7", torrent_count: 1 }],
};

const movies = {
  id: 1,
  name: "Demo Movie 2024",
  total_bytes: 734003200,
  state: 4,
  progress: 1,
  download_rate: 0,
  upload_rate: 512,
  seeds: 3,
  peers: 4,
  eta_seconds: 0,
  save_path: "/data/Filmek",
  label: "movies",
  error: "",
  queue_position: 0,
};

const series = {
  ...movies,
  id: 2,
  name: "Demo Series S01E01",
  state: 3,
  progress: 0.42,
  download_rate: 1048576,
  upload_rate: 128,
  seeds: 1,
  peers: 7,
  label: "",
};

const list = { torrents: [movies, series], total: 2, limit: 50 };

function json(route: Route, body: unknown, status = 200) {
  return route.fulfill({
    status,
    contentType: "application/json",
    body: JSON.stringify(body),
  });
}

async function mockApi(page: Page): Promise<string[]> {
  const errors: string[] = [];
  page.on("console", (message) => {
    if (message.type() === "error") errors.push(message.text());
  });
  page.on("pageerror", (error) => errors.push(error.message));

  await page.route("**/health", (route) =>
    json(route, { status: "ok", daemon: true, read_only: false }),
  );
  await page.route("**/api/login", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      headers: { "set-cookie": "ferrent_session=test; Path=/; HttpOnly" },
      body: JSON.stringify({ read_only: false, ttl: 3600 }),
    }),
  );
  await page.route("**/api/status", (route) => json(route, status));
  await page.route("**/api/labels", (route) => json(route, labels));
  await page.route("**/api/torrents?*", (route) => json(route, list));
  await page.route("**/api/torrents/bulk", (route) =>
    json(route, { ok: [1], failed: [] }),
  );
  await page.route("**/api/torrents/*/pause", (route) =>
    json(route, { id: 1, paused: true }),
  );
  await page.route("**/api/torrents/1", (route) => json(route, movies));
  await page.route("**/api/torrents/1/files", (route) =>
    json(route, {
      files: [{ path: "Demo.Movie.2024.mkv", size: 734003200, progress: 1, priority: 4 }],
    }),
  );
  await page.route("**/api/torrents/1/peers", (route) =>
    json(route, {
      peers: [
        {
          endpoint: "1.2.3.4:51413",
          client: "qBittorrent 4.6",
          progress: 0.5,
          download_rate: 1024,
          upload_rate: 512,
        },
      ],
    }),
  );
  await page.route("**/api/torrents/1/trackers", (route) => {
    if (route.request().method() === "POST") {
      return json(route, {
        id: 1,
        trackers: [
          {
            url: "udp://t.example:6969",
            status: "unverified",
            seeds: 0,
            leechers: 0,
            message: "",
            next_announce: 0,
            fails: 0,
          },
        ],
      });
    }
    return json(route, { id: 1, trackers: [] });
  });
  await page.route("**/api/torrents/1/webseeds", (route) =>
    json(route, { id: 1, web_seeds: [] }),
  );
  await page.route("**/api/config", (route) => {
    if (route.request().method() === "PUT") {
      return json(route, { revision: 2 });
    }
    return json(route, {
      revision: 1,
      env_locked: ["bind"],
      toml: '[general]\ndata_dir = "/data"\n',
    });
  });
  await page.route("**/api/events", (route) =>
    route.fulfill({
      status: 200,
      headers: { "content-type": "text/event-stream", "cache-control": "no-cache" },
      body: 'event: deltas\ndata: {"changed":[],"removed":[]}\n\n',
    }),
  );
  return errors;
}

async function login(page: Page) {
  await page.goto("/");
  await page.getByRole("textbox", { name: "Jelszó" }).fill("test-pass");
  await page.getByRole("button", { name: "Belépés" }).click();
  await expect(page.getByText("Demo Movie 2024")).toBeVisible();
}

test("logs in and lists the torrents at a phone width", async ({ page }) => {
  const errors = await mockApi(page);
  await login(page);

  await expect(page.getByText("Demo Series S01E01")).toBeVisible();

  const overflow = await page.evaluate(
    () => document.documentElement.scrollWidth - window.innerWidth,
  );
  expect(overflow).toBeLessThanOrEqual(0);

  expect(errors).toEqual([]);
});

test("narrows to 360 px without horizontal scroll", async ({ page }) => {
  await mockApi(page);
  await page.setViewportSize({ width: 360, height: 800 });
  await login(page);
  const overflow = await page.evaluate(
    () => document.documentElement.scrollWidth - window.innerWidth,
  );
  expect(overflow).toBeLessThanOrEqual(0);
});

test("switches to English and back", async ({ page }) => {
  await mockApi(page);
  await login(page);
  await page.getByRole("button", { name: "Nyelv" }).click();
  await expect(page.getByPlaceholder("Search by name…")).toBeVisible();
  await page.getByRole("button", { name: "Language" }).click();
  await expect(page.getByPlaceholder("Keresés névben…")).toBeVisible();
});

test("opens the detail, adds a tracker and lists the peers", async ({ page }) => {
  const errors = await mockApi(page);
  await login(page);

  await page.getByText("Demo Movie 2024").click();
  await expect(
    page.getByRole("heading", { name: "Demo Movie 2024" }),
  ).toBeVisible();
  await expect(page.getByText("1.2.3.4:51413")).toBeVisible();

  await page.getByRole("button", { name: /Trackerek/ }).click();
  await page
    .getByPlaceholder("Announce URL (http, https, udp vagy ws)")
    .fill("udp://t.example:6969");
  await page.getByRole("button", { name: "Add tracker" }).click();
  await expect(page.getByText("udp://t.example:6969")).toBeVisible();

  expect(errors).toEqual([]);
});

test("saves the daemon config against its revision", async ({ page }) => {
  const errors = await mockApi(page);
  await login(page);

  await page.getByRole("button", { name: "Beállítások" }).click();
  await expect(page.getByText("Revízió: 1")).toBeVisible();
  await expect(page.getByText("bind")).toBeVisible();

  const editor = page.locator(".settings-sheet textarea");
  await editor.fill('[general]\ndata_dir = "/data2"\n');
  const put = page.waitForRequest(
    (request) =>
      request.url().endsWith("/api/config") && request.method() === "PUT",
  );
  await page.locator(".settings-sheet .sheet-actions .primary").click();
  const request = await put;
  expect(JSON.parse(request.postData() ?? "{}")).toEqual({
    toml: '[general]\ndata_dir = "/data2"\n',
    revision: 1,
  });

  expect(errors).toEqual([]);
});

test("keyboard: ? opens the help and arrows select a row", async ({ page }) => {
  await mockApi(page);
  await login(page);

  await page.keyboard.press("?");
  await expect(
    page.getByRole("heading", { name: "Billentyűparancsok" }),
  ).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(
    page.getByRole("heading", { name: "Billentyűparancsok" }),
  ).toBeHidden();

  await page.keyboard.press("ArrowDown");
  await expect(page.getByText("1 kijelölve")).toBeVisible();
});
