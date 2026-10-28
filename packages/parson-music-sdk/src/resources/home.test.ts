import { afterEach, expect, mock, spyOn, test } from "bun:test";
import api from "../core/http";
import {
  albumQualityLabel,
  getHomeEssentials,
  getSimilarArtists,
  greetingForHour,
} from "./home";

afterEach(() => {
  mock.restore();
});

test("home and similar artists use the server's paths", async () => {
  const get = spyOn(api, "get").mockResolvedValue({
    data: [],
    status: 200,
    headers: new Headers(),
  } as never);
  await getHomeEssentials();
  await getSimilarArtists("nine inch/nails");
  expect(get).toHaveBeenNthCalledWith(1, "/home");
  expect(get).toHaveBeenNthCalledWith(
    2,
    "/artists/nine%20inch%2Fnails/similar",
  );
});

test("album quality labels show bit depth and rate for lossless files", () => {
  const base = { codec: "flac", lossless: true, hi_res: true };
  expect(
    albumQualityLabel({ ...base, bit_depth: 24, sample_rate: 96_000 }),
  ).toBe("24/96");
  expect(
    albumQualityLabel({ ...base, bit_depth: 16, sample_rate: 44_100 }),
  ).toBe("16/44.1");
  expect(
    albumQualityLabel({
      codec: "mp3",
      lossless: false,
      hi_res: false,
      bit_depth: null,
      sample_rate: 44_100,
    }),
  ).toBe("MP3");
  expect(
    albumQualityLabel({
      codec: null,
      lossless: false,
      hi_res: false,
      bit_depth: null,
      sample_rate: null,
    }),
  ).toBeNull();
  expect(albumQualityLabel(null)).toBeNull();
});

test("the server's label wins, so Home and Library show one badge", () => {
  expect(
    albumQualityLabel({
      codec: "mp3",
      lossless: false,
      hi_res: false,
      bit_depth: null,
      sample_rate: 44_100,
      mixed: true,
      label: "Mixed",
    }),
  ).toBe("Mixed");
  expect(
    albumQualityLabel({
      codec: "flac",
      lossless: true,
      hi_res: true,
      bit_depth: 24,
      sample_rate: 96_000,
      label: "24/96 FLAC",
    }),
  ).toBe("24/96 FLAC");
});

test("greetings follow the local hour", () => {
  expect(greetingForHour(6)).toBe("Good morning");
  expect(greetingForHour(12)).toBe("Good afternoon");
  expect(greetingForHour(19)).toBe("Good evening");
  expect(greetingForHour(2)).toBe("Good evening");
});
