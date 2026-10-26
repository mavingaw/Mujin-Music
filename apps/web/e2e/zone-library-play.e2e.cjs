#!/usr/bin/env node
/**
 * End to end: library Play actions follow the zone picker.
 *
 * Starts a real Parson server and a real Parson Bridge (simulated DACs, the
 * same protocol and pacing as a Raspberry Pi with a USB DAC), builds a small
 * FLAC album with ffmpeg, signs in with Chromium, picks the bridge DAC in the
 * zone picker and presses the album's Play button. Then it checks that:
 *
 * 1. the bridge zone's queue and now playing are the album, on the server;
 * 2. the browser's own audio element never played;
 * 3. the player bar shows the album's first track, "Playing on" the DAC;
 *
 * and the same for a track row's Play and the album's "Add to queue".
 *
 * Needs: `apps/web/out` (bun --filter parson-music-web build), the debug
 * binaries (cargo build -p parson-music --bin parson-music-server &&
 * cargo build -p parson-bridge --bin parson-bridge), ffmpeg and Playwright's
 * Chromium. Run: node apps/web/e2e/zone-library-play.e2e.cjs [screenshot-dir]
 */
const { spawn, execFileSync } = require("node:child_process");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const net = require("node:net");
const { chromium } = require("playwright");

const repo = path.resolve(__dirname, "../../..");
const target = process.env.CARGO_TARGET_DIR ?? path.join(repo, "target");
const serverBin =
  process.env.PARSON_SERVER_BIN ??
  path.join(target, "debug/parson-music-server");
const bridgeBin =
  process.env.PARSON_BRIDGE_BIN ?? path.join(target, "debug/parson-bridge");
const shots = process.argv[2] ?? null;

function freePort() {
  return new Promise((resolve, reject) => {
    const server = net.createServer();
    server.listen(0, "127.0.0.1", () => {
      const { port } = server.address();
      server.close(() => resolve(port));
    });
    server.on("error", reject);
  });
}

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

async function until(what, check, timeoutMs = 20_000) {
  const deadline = Date.now() + timeoutMs;
  let last;
  while (Date.now() < deadline) {
    try {
      last = await check();
      if (last) return last;
    } catch (error) {
      last = String(error);
    }
    await sleep(200);
  }
  throw new Error(`Timed out waiting for ${what}: ${JSON.stringify(last)}`);
}

function assert(condition, message) {
  if (!condition) throw new Error(`FAILED: ${message}`);
  console.log(`ok - ${message}`);
}

function makeAlbum(directory) {
  const album = path.join(directory, "Bridge Test Artist", "Bridge Test Album");
  fs.mkdirSync(album, { recursive: true });
  const tracks = [
    ["01 First Light.flac", "First Light", 1, 440],
    ["02 Second Wind.flac", "Second Wind", 2, 550],
    ["03 Third Coast.flac", "Third Coast", 3, 660],
  ];
  for (const [file, title, number, tone] of tracks) {
    execFileSync(
      "ffmpeg",
      [
        "-v",
        "error",
        "-y",
        "-f",
        "lavfi",
        "-i",
        `sine=frequency=${tone}:sample_rate=96000:duration=20`,
        "-ac",
        "2",
        "-c:a",
        "flac",
        "-sample_fmt",
        "s32",
        "-bits_per_raw_sample",
        "24",
        "-metadata",
        `title=${title}`,
        "-metadata",
        "artist=Bridge Test Artist",
        "-metadata",
        "album_artist=Bridge Test Artist",
        "-metadata",
        "album=Bridge Test Album",
        "-metadata",
        `track=${number}`,
        "-metadata",
        "date=2026",
        path.join(album, file),
      ],
      { stdio: "inherit" },
    );
  }
  return tracks.map(([, title]) => title);
}

async function main() {
  for (const bin of [serverBin, bridgeBin]) {
    if (!fs.existsSync(bin)) throw new Error(`Missing ${bin}; build it first`);
  }
  const work = fs.mkdtempSync(path.join(os.tmpdir(), "parson-zone-e2e-"));
  const children = [];
  const cleanup = () => {
    for (const child of children) child.kill("SIGTERM");
    if (process.env.E2E_KEEP) console.log(`kept ${work}`);
    else fs.rmSync(work, { recursive: true, force: true });
  };
  process.on("exit", cleanup);
  try {
    const music = path.join(work, "music");
    const titles = makeAlbum(music);
    const bridgePort = await freePort();
    const serverPort = await freePort();
    const log = fs.openSync(path.join(work, "processes.log"), "a");
    children.push(
      spawn(
        bridgeBin,
        [
          "--simulated",
          "--no-mdns",
          "--name",
          "Listening Room",
          "--port",
          String(bridgePort),
          "--bind",
          "127.0.0.1",
        ],
        {
          stdio: ["ignore", log, log],
          env: { ...process.env, PARSON_BRIDGE_ID: "e2e-bridge" },
        },
      ),
    );
    children.push(
      spawn(serverBin, [], {
        stdio: ["ignore", log, log],
        env: {
          ...process.env,
          PARSON_DATA_DIR: path.join(work, "data"),
          PARSON_PORT: String(serverPort),
          PARSON_BIND_ADDRESS: "127.0.0.1",
          PARSON_BRIDGES: `127.0.0.1:${bridgePort}`,
          PARSON_SONOS: "off",
          PARSON_UPNP: "off",
          PARSON_AIRPLAY: "off",
          PARSON_CHROMECAST: "off",
          PARSON_LOCAL_AUDIO: "off",
        },
      }),
    );
    const base = `http://127.0.0.1:${serverPort}`;
    let cookie = "";
    const api = async (method, route, body) => {
      const response = await fetch(`${base}/api/v1${route}`, {
        method,
        headers: {
          "Content-Type": "application/json",
          Origin: base,
          ...(cookie ? { Cookie: cookie } : {}),
        },
        body: body ? JSON.stringify(body) : undefined,
      });
      const setCookie = response.headers.getSetCookie?.() ?? [];
      if (setCookie.length) {
        cookie = setCookie.map((value) => value.split(";")[0]).join("; ");
      }
      const text = await response.text();
      if (!response.ok)
        throw new Error(`${method} ${route}: ${response.status} ${text}`);
      return text ? JSON.parse(text) : null;
    };
    await until("the server", () =>
      fetch(`${base}/.well-known/parson`).then((r) => r.ok),
    );
    const account = { username: "demo", password: "parson-demo-pass" };
    await api("POST", "/auth/register", { ...account, role: "admin" });
    await api("POST", "/auth/login", account);
    await api("POST", "/setup/library", { path: music });
    await until(
      "the library",
      async () => (await api("GET", "/setup/status")).library_state === "ready",
      60_000,
    );
    const album = await until(
      "the album",
      async () => {
        const found = (await api("GET", "/albums/random/10")).find(
          (entry) => entry.name === "Bridge Test Album",
        );
        if (!found) return null;
        // The scanner settles albums in passes; wait for the whole album.
        const info = await api(
          "GET",
          `/albums/${encodeURIComponent(found.id)}`,
        );
        const full = info?.Full ?? info;
        return full?.songs?.length === titles.length ? full : null;
      },
      60_000,
    );
    const dac = await until("the bridge DAC zone", async () =>
      (await api("GET", "/zones")).zones.find(
        (zone) => zone.kind === "bridge" && zone.id.endsWith(":sim-usb-dac"),
      ),
    );
    console.log(`bridge zone ${dac.id} (${dac.name}), album ${album.id}`);

    const browser = await chromium.launch({
      args: ["--autoplay-policy=no-user-gesture-required"],
    });
    const page = await browser.newPage({
      viewport: { width: 1440, height: 900 },
    });
    page.on("pageerror", (error) => console.log("pageerror", error.message));
    // Record every play() the page makes on a media element.
    await page.addInitScript(() => {
      window.__parsonPlays = [];
      const play = HTMLMediaElement.prototype.play;
      HTMLMediaElement.prototype.play = function (...args) {
        window.__parsonPlays.push(this.currentSrc || this.src || "(no source)");
        return play.apply(this, args);
      };
    });
    await page.goto(`${base}/login`, { waitUntil: "networkidle" });
    await page.getByPlaceholder("Username").fill(account.username);
    await page.getByPlaceholder("Password").fill(account.password);
    await page.keyboard.press("Enter");
    await page.waitForURL((url) => !url.pathname.startsWith("/login"), {
      timeout: 20_000,
    });

    // Pick the DAC in the zone picker.
    await page.locator("[data-zone-picker]").first().click();
    await page.locator(`[data-zone-select="${dac.id}"]`).click();
    await page
      .getByRole("button", {
        name: `Zone: ${dac.name}. Choose where music plays`,
      })
      .first()
      .waitFor();

    // Album Play.
    await page.goto(`${base}/album?id=${encodeURIComponent(album.id)}`, {
      waitUntil: "networkidle",
    });
    await page
      .getByRole("button", { name: "Play Bridge Test Album" })
      .click()
      .catch(async (error) => {
        if (shots)
          await page.screenshot({ path: path.join(shots, "failure.png") });
        throw error;
      });
    const playing = await until("the DAC to play the album", async () => {
      const zone = (await api("GET", "/zones")).zones.find(
        (entry) => entry.id === dac.id,
      );
      return zone?.status === "playing" && zone.now_playing?.title === titles[0]
        ? zone
        : null;
    });
    assert(
      playing.queue.song_ids.length === album.songs.length &&
        playing.queue.song_ids.every(
          (id, index) => id === album.songs[index].id,
        ),
      "the bridge zone's queue is the album, in order",
    );
    assert(
      playing.queue.current_position === 0,
      "the bridge zone starts at track 1",
    );
    assert(
      playing.output?.stream?.output?.startsWith("96 kHz/24-bit"),
      `the DAC plays the file's own format (${playing.output?.stream?.output})`,
    );
    const bar = page
      .locator("section", { hasText: `Playing on ${dac.name}` })
      .first();
    await bar
      .getByText(titles[0], { exact: true })
      .filter({ visible: true })
      .first()
      .waitFor({ timeout: 10_000 });
    assert(true, `the player bar shows "${titles[0]}" playing on ${dac.name}`);
    if (shots) {
      fs.mkdirSync(shots, { recursive: true });
      await page.screenshot({
        path: path.join(shots, "album-play-on-bridge-zone.png"),
      });
    }

    // A track row's Play replaces the queue from that track.
    await page
      .getByRole("button", { name: `Play ${titles[2]}` })
      .first()
      .click();
    const third = await until("the DAC to play track 3", async () => {
      const zone = (await api("GET", "/zones")).zones.find(
        (entry) => entry.id === dac.id,
      );
      return zone?.now_playing?.title === titles[2] ? zone : null;
    });
    assert(
      third.queue.current_position === 2,
      "a track's Play starts the album at that track",
    );
    await bar
      .getByText(titles[2], { exact: true })
      .filter({ visible: true })
      .first()
      .waitFor({ timeout: 10_000 });
    assert(true, `the player bar follows to "${titles[2]}"`);

    // Add to queue appends on the zone, without restarting what plays.
    await page.getByRole("button", { name: "Album actions" }).click();
    await page.getByRole("menuitem", { name: "Add to queue" }).click();
    const grown = await until("the DAC queue to grow", async () => {
      const zone = (await api("GET", "/zones")).zones.find(
        (entry) => entry.id === dac.id,
      );
      return zone?.queue.song_ids.length === album.songs.length * 2
        ? zone
        : null;
    });
    assert(
      grown.now_playing?.title === titles[2],
      "Add to queue keeps the current track playing",
    );

    const plays = await page.evaluate(() => window.__parsonPlays);
    const silent = await page.evaluate(() =>
      [...document.querySelectorAll("audio, video")].every(
        (element) => element.paused && element.currentTime === 0,
      ),
    );
    assert(
      plays.length === 0 && silent,
      `the browser stayed silent (plays: ${JSON.stringify(plays)})`,
    );
    await browser.close();
    console.log("zone library play e2e: all checks passed");
  } finally {
    cleanup();
  }
}

main().catch((error) => {
  console.error(error);
  process.exit(1);
});
