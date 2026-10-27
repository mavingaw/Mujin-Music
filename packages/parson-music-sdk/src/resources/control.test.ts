import { afterEach, expect, mock, spyOn, test } from "bun:test";
import api from "../core/http";
import {
  browseLibrary,
  homeAssistantAutomationExample,
  homeAssistantConnection,
  isLoopbackHost,
  playInZone,
  rateZoneTrack,
  searchLibraryItems,
  startZoneRadio,
} from "./control";

afterEach(() => mock.restore());

test("browse and search use the controller browse route", async () => {
  const get = spyOn(api, "get").mockResolvedValue({
    data: { item: { id: "root" }, children: [], total: 0 },
  } as never);
  await browseLibrary();
  expect(get).toHaveBeenCalledWith("/browse", { params: { item: "root" } });
  await browseLibrary("album:a1");
  expect(get).toHaveBeenCalledWith("/browse", { params: { item: "album:a1" } });
  await searchLibraryItems("miles");
  expect(get).toHaveBeenCalledWith("/browse", { params: { query: "miles" } });
});

test("play, radio and ratings address the zone by its encoded id", async () => {
  const post = spyOn(api, "post").mockResolvedValue({
    data: { accepted: true },
  } as never);
  await playInZone("sonos:RINCON_1", { item: "album:a1", action: "next" });
  expect(post).toHaveBeenCalledWith(
    "/zones/sonos%3ARINCON_1/play",
    { item: "album:a1", action: "next" },
    { timeout: 15_000 },
  );
  await startZoneRadio("client:dev", { query: "jazz", kind: "genre" });
  expect(post).toHaveBeenCalledWith(
    "/zones/client%3Adev/radio",
    { query: "jazz", kind: "genre", action: "radio" },
    { timeout: 15_000 },
  );
  await rateZoneTrack("client:dev", "down");
  expect(post).toHaveBeenCalledWith("/zones/client%3Adev/rating", {
    rating: "down",
  });
  await rateZoneTrack("client:dev", "up", "song-9");
  expect(post).toHaveBeenCalledWith("/zones/client%3Adev/rating", {
    rating: "up",
    song_id: "song-9",
  });
});

test("Home Assistant connection details come from the app's own origin", () => {
  expect(homeAssistantConnection("http://192.168.1.20:1993")).toEqual({
    host: "192.168.1.20",
    port: 1993,
    ssl: false,
  });
  expect(homeAssistantConnection("https://music.example.com")).toEqual({
    host: "music.example.com",
    port: 443,
    ssl: true,
  });
  expect(homeAssistantConnection("http://[fd00::5]:1993")?.host).toBe(
    "fd00::5",
  );
  expect(homeAssistantConnection("file:///tmp")).toBeNull();
  expect(homeAssistantConnection("not a url")).toBeNull();
  expect(isLoopbackHost("127.0.0.1")).toBe(true);
  expect(isLoopbackHost("localhost")).toBe(true);
  expect(isLoopbackHost("::1")).toBe(true);
  expect(isLoopbackHost("192.168.1.20")).toBe(false);
});

test("the automation example targets the given player with integration services", () => {
  const yaml = homeAssistantAutomationExample("media_player.kitchen");
  expect(yaml).toContain("action: mujin_music.start_radio");
  expect(yaml).toContain("entity_id: media_player.kitchen");
  expect(yaml.split("\n").every((line) => !line.includes("\t"))).toBe(true);
});
