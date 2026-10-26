import { afterEach, expect, mock, spyOn, test } from "bun:test";
import api from "../core/http";
import {
  addNetworkOutput,
  deleteConvolutionFilter,
  getConvolutionFilters,
  isNetworkDriver,
  nativeOutputIconUrl,
  networkOutputIdentity,
  removeNetworkOutput,
  uploadConvolutionFilter,
  type NetworkOutputInfo,
} from "./outputs";

afterEach(() => mock.restore());

const ok = (data: unknown) =>
  ({ data, status: 200, headers: new Headers() }) as never;

test("adds and removes network players by address", async () => {
  const post = spyOn(api, "post").mockResolvedValue(ok({ devices: [] }));
  const del = spyOn(api, "delete").mockResolvedValue(ok({ devices: [] }));
  await addNetworkOutput({
    protocol: "air_play",
    address: " 10.0.0.9:7000 ",
    name: " Kitchen ",
  });
  expect(post).toHaveBeenCalledWith(
    "/outputs/native/admin/network",
    { protocol: "air_play", address: "10.0.0.9:7000", name: "Kitchen" },
    { timeout: 30_000 },
  );
  await removeNetworkOutput("10.0.0.9:7000");
  expect(del).toHaveBeenCalledWith("/outputs/native/admin/network", {
    data: { address: "10.0.0.9:7000" },
    timeout: 30_000,
  });
});

test("names network players and their pictures", () => {
  expect(isNetworkDriver("upnp")).toBeTrue();
  expect(isNetworkDriver("airplay")).toBeTrue();
  expect(isNetworkDriver("alsa")).toBeFalse();
  expect(nativeOutputIconUrl("upnp:abc def")).toEndWith(
    "/outputs/native/icon?device_id=upnp%3Aabc%20def",
  );
  const network = {
    protocol: "open_home",
    manufacturer: "Linn",
    model: "Klimax DSM",
  } as NetworkOutputInfo;
  expect(networkOutputIdentity(network)).toBe("Linn Klimax DSM · OpenHome");
  expect(
    networkOutputIdentity({
      ...network,
      protocol: "air_play",
      manufacturer: "Apple",
      model: "Apple TV 4K",
    }),
  ).toBe("Apple TV 4K · AirPlay");
});

test("uploads, lists and deletes convolution filters", async () => {
  const filter = { id: "ab12", name: "Room" };
  const post = spyOn(api, "post").mockResolvedValue(ok(filter));
  const get = spyOn(api, "get").mockResolvedValue(ok([filter]));
  const del = spyOn(api, "delete").mockResolvedValue(ok(undefined));
  const file = new Blob([new Uint8Array([1, 2, 3])], { type: "audio/wav" });
  expect(
    await uploadConvolutionFilter(file, {
      name: " Room ",
      fileName: "room.wav",
    }),
  ).toBe(filter as never);
  expect(post).toHaveBeenCalledWith("/outputs/native/admin/convolution", file, {
    params: { name: "Room", file_name: "room.wav" },
    headers: { "Content-Type": "audio/wav" },
    timeout: 120_000,
  });
  expect(await getConvolutionFilters()).toEqual([filter] as never);
  expect(get).toHaveBeenCalledWith("/outputs/native/convolution");
  await deleteConvolutionFilter("ab12");
  expect(del).toHaveBeenCalledWith("/outputs/native/admin/convolution/ab12");
});
