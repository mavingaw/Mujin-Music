import { afterEach, expect, mock, spyOn, test } from "bun:test";
import api from "../core/http";
import {
  addNetworkPlayer,
  deleteConvolutionFilter,
  dsdRateLabel,
  getConvolutionFilterFile,
  getConvolutionFilters,
  getNetworkPlayers,
  networkOutputIdentity,
  outputErrorMessage,
  removeNetworkPlayer,
  uploadConvolutionFilter,
  zoneIconUrl,
  type NetworkOutputInfo,
} from "./outputs";

afterEach(() => mock.restore());

const ok = (data: unknown) =>
  ({ data, status: 200, headers: new Headers() }) as never;

test("lists, adds and removes network players by address", async () => {
  const players = { discovery: true, manual: [] };
  const get = spyOn(api, "get").mockResolvedValue(ok(players));
  const post = spyOn(api, "post").mockResolvedValue(ok(players));
  const del = spyOn(api, "delete").mockResolvedValue(ok(players));
  expect(await getNetworkPlayers()).toBe(players as never);
  expect(get).toHaveBeenCalledWith("/zones/outputs/network");
  await addNetworkPlayer({
    protocol: "air_play",
    address: " 10.0.0.9:7000 ",
    name: " Kitchen ",
  });
  expect(post).toHaveBeenCalledWith(
    "/zones/outputs/network",
    { protocol: "air_play", address: "10.0.0.9:7000", name: "Kitchen" },
    { timeout: 30_000 },
  );
  await addNetworkPlayer({ protocol: "upnp_av", address: "10.0.0.40" });
  expect(post).toHaveBeenLastCalledWith(
    "/zones/outputs/network",
    { protocol: "upnp_av", address: "10.0.0.40", name: null },
    { timeout: 30_000 },
  );
  await removeNetworkPlayer("10.0.0.9:7000");
  expect(del).toHaveBeenCalledWith("/zones/outputs/network", {
    data: { address: "10.0.0.9:7000" },
    timeout: 30_000,
  });
});

test("names network players and their pictures", () => {
  expect(zoneIconUrl("upnp:abc def")).toEndWith("/zones/upnp%3Aabc%20def/icon");
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
  expect(dsdRateLabel(5_644_800)).toBe("DSD128");
});

test("uploads, lists, fetches and deletes convolution filters", async () => {
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
  expect(post).toHaveBeenCalledWith("/outputs/admin/convolution", file, {
    params: { name: "Room", file_name: "room.wav" },
    headers: { "Content-Type": "audio/wav" },
    timeout: 120_000,
  });
  expect(await getConvolutionFilters()).toEqual([filter] as never);
  expect(get).toHaveBeenCalledWith("/outputs/convolution");
  get.mockResolvedValue(ok(new Blob([new Uint8Array([7])])));
  expect((await getConvolutionFilterFile("ab12")).byteLength).toBe(1);
  expect(get).toHaveBeenLastCalledWith("/outputs/convolution/ab12/file", {
    responseType: "blob",
    timeout: 60_000,
  });
  await deleteConvolutionFilter("ab12");
  expect(del).toHaveBeenCalledWith("/outputs/admin/convolution/ab12");
});

test("explains failed requests in the server's words", () => {
  expect(outputErrorMessage(new Error("offline"), "fallback")).toBe("offline");
  expect(outputErrorMessage("nope", "fallback")).toBe("fallback");
});
