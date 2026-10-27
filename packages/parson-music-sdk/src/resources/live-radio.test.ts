import { afterEach, expect, mock, spyOn, test } from "bun:test";
import api from "../core/http";
import {
  addLiveStation,
  checkLiveStream,
  getLiveFacets,
  getLiveNowPlaying,
  getLiveRadioHome,
  getLiveStation,
  isLiveRadioId,
  liveRadioSongId,
  liveSearchParams,
  liveStationIdFromSongId,
  liveStationLogoUrl,
  removeLiveStation,
  reorderLiveStations,
  saveLiveStation,
  searchLiveStations,
  verifyLiveStation,
} from "./live-radio";

const ID = "d1a54d2e-623e-4970-ab11-35f7b56c5ec3";

afterEach(() => {
  mock.restore();
});

const reply = (data: unknown) =>
  ({ data, status: 200, headers: new Headers() }) as never;

test("station song ids are radio:<uuid> and nothing else", () => {
  expect(liveRadioSongId(ID)).toBe(`radio:${ID}`);
  expect(liveStationIdFromSongId(`radio:${ID}`)).toBe(ID);
  expect(isLiveRadioId(`radio:${ID.toUpperCase()}`)).toBeTrue();
  for (const other of [
    ID,
    "tidal:123",
    "radio:../../etc/passwd",
    `radio:${ID}x`,
    "",
    null,
    undefined,
  ]) {
    expect(isLiveRadioId(other)).toBeFalse();
  }
});

test("search sends only real filters", async () => {
  expect(
    liveSearchParams({
      name: "  jazz ",
      tag: "",
      lossless: false,
      min_bitrate: 256,
      order: "votes",
      country_code: undefined,
    }),
  ).toEqual({ name: "jazz", min_bitrate: "256", order: "votes" });
  const get = spyOn(api, "get").mockResolvedValue(
    reply({ items: [], offset: 0, limit: 40, has_more: false }),
  );
  await searchLiveStations({ lossless: true, tag: "classical" });
  expect(get).toHaveBeenCalledWith("/live-radio/stations", {
    params: { lossless: "true", tag: "classical" },
  });
});

test("home, detail, facets and now playing use the live radio routes", async () => {
  const get = spyOn(api, "get").mockImplementation(async (path) => {
    if (path === "/live-radio/now-playing") {
      return reply({ items: { [ID]: { station_id: ID } } });
    }
    if (path.startsWith("/live-radio/facets/")) {
      return reply({ items: [{ name: "jazz", station_count: 4 }] });
    }
    return reply({});
  });
  await getLiveRadioHome("fr");
  expect(get).toHaveBeenCalledWith("/live-radio/home", {
    params: { country: "fr" },
  });
  await getLiveStation(ID);
  expect(get).toHaveBeenCalledWith(`/live-radio/stations/${ID}`);
  expect((await getLiveFacets("tags", 50))[0]?.name).toBe("jazz");
  expect(get).toHaveBeenCalledWith("/live-radio/facets/tags", {
    params: { limit: 50 },
  });
  const playing = await getLiveNowPlaying([ID, ID]);
  expect(playing[ID]?.station_id).toBe(ID);
  expect(get).toHaveBeenLastCalledWith("/live-radio/now-playing", {
    params: { ids: ID },
    timeout: 8_000,
  });
  get.mockClear();
  expect(await getLiveNowPlaying([])).toEqual({});
  expect(get).not.toHaveBeenCalled();
});

test("my live radio edits go to /live-radio/my", async () => {
  const put = spyOn(api, "put").mockResolvedValue(reply({ id: ID }));
  const post = spyOn(api, "post").mockResolvedValue(
    reply({ station: { id: ID }, check: {} }),
  );
  const remove = spyOn(api, "delete").mockResolvedValue(reply(null));
  await saveLiveStation(ID);
  expect(put).toHaveBeenCalledWith(`/live-radio/my/${ID}`);
  await addLiveStation({ url: "https://radio.example/live.pls" });
  expect(post).toHaveBeenCalledWith(
    "/live-radio/my",
    { url: "https://radio.example/live.pls" },
    { timeout: 30_000 },
  );
  await checkLiveStream("https://radio.example/live.m3u8", true);
  expect(post).toHaveBeenLastCalledWith(
    "/live-radio/check",
    { url: "https://radio.example/live.m3u8", allow_lan: true },
    { timeout: 30_000 },
  );
  await reorderLiveStations([ID]);
  expect(put).toHaveBeenLastCalledWith("/live-radio/my-order", { ids: [ID] });
  await removeLiveStation(ID);
  expect(remove).toHaveBeenCalledWith(`/live-radio/my/${ID}`);
  await verifyLiveStation(ID);
  expect(post).toHaveBeenLastCalledWith(
    `/live-radio/stations/${ID}/verify`,
    undefined,
    { timeout: 20_000 },
  );
});

test("logo paths become absolute against the server origin", () => {
  const station = { logo_url: `/api/v1/live-radio/stations/${ID}/logo` };
  expect(liveStationLogoUrl(station, "https://music.example:1993/api/v1")).toBe(
    `https://music.example:1993/api/v1/live-radio/stations/${ID}/logo`,
  );
  expect(liveStationLogoUrl({ logo_url: null }, "http://x/api/v1")).toBeNull();
  expect(
    liveStationLogoUrl({ logo_url: "https://cdn.example/a.png" }, "http://x"),
  ).toBe("https://cdn.example/a.png");
});
