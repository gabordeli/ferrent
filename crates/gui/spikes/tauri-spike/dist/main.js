const ROWS = 4000;
const ROW_HEIGHT = 26;
const OVERSCAN = 4;
const MEASURE_MS = 5000;

const viewport = document.getElementById("viewport");
const spacer = document.getElementById("spacer");
const rowsElement = document.getElementById("rows");
const rowCount = document.getElementById("row-count");

spacer.style.height = `${ROWS * ROW_HEIGHT}px`;
rowCount.textContent = `${ROWS} rows`;

let renderedStart = -1;
let renderedCount = 0;

function formatRate(bytes) {
  if (bytes < 1024) return `${bytes.toFixed(0)} B/s`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KiB/s`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MiB/s`;
}

function render() {
  const scrollTop = viewport.scrollTop;
  const start = Math.max(0, Math.floor(scrollTop / ROW_HEIGHT) - OVERSCAN);
  const count = Math.ceil(viewport.clientHeight / ROW_HEIGHT) + OVERSCAN * 2;
  if (start === renderedStart && count === renderedCount) {
    return;
  }
  renderedStart = start;
  renderedCount = count;

  rowsElement.style.transform = `translateY(${start * ROW_HEIGHT}px)`;
  const fragment = document.createDocumentFragment();
  const end = Math.min(ROWS, start + count);
  for (let index = start; index < end; index += 1) {
    const progress = (index * 37) % 100;
    const down = (index * 131071) % 5000000;
    const up = (index * 97003) % 900000;

    const row = document.createElement("div");
    row.className = "row";
    row.style.height = `${ROW_HEIGHT}px`;

    const name = document.createElement("span");
    name.className = "name";
    name.textContent = `#${String(index).padStart(4, "0")}  torrent-${String(index).padStart(4, "0")}.mkv`;

    const bar = document.createElement("span");
    bar.className = "bar";
    const fill = document.createElement("span");
    fill.style.width = `${progress}%`;
    bar.appendChild(fill);

    const progressLabel = document.createElement("span");
    progressLabel.className = "num";
    progressLabel.textContent = `${progress.toFixed(1)}%`;

    const downLabel = document.createElement("span");
    downLabel.className = "num";
    downLabel.textContent = `down ${formatRate(down)}`;

    const upLabel = document.createElement("span");
    upLabel.className = "num";
    upLabel.textContent = `up ${formatRate(up)}`;

    row.append(name, bar, progressLabel, downLabel, upLabel);
    fragment.appendChild(row);
  }
  rowsElement.replaceChildren(fragment);
}

viewport.addEventListener("scroll", render, { passive: true });
render();

let firstFrame = null;
let frames = 0;
let done = false;

function tick(now) {
  if (firstFrame === null) {
    firstFrame = now;
    window.__TAURI__.core.invoke("report_startup", { ms: now });
  }
  frames += 1;

  if (!done) {
    const maxScroll = ROWS * ROW_HEIGHT - viewport.clientHeight;
    viewport.scrollTop = (viewport.scrollTop + 18) % maxScroll;
  }

  if (!done && firstFrame !== null && now - firstFrame >= MEASURE_MS) {
    done = true;
    const elapsed = (now - firstFrame) / 1000;
    const metrics = {
      fps: Number((frames / elapsed).toFixed(1)),
      rows: ROWS,
      visible_rows: Math.ceil(viewport.clientHeight / ROW_HEIGHT) + 2,
      virtualized: true,
    };
    window.__TAURI__.core.invoke("report_metrics", {
      json: JSON.stringify(metrics),
    });
    return;
  }

  requestAnimationFrame(tick);
}

requestAnimationFrame(tick);
