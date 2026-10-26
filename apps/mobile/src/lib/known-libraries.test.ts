import { expect, test } from "bun:test";
import {
  forgetLibrary,
  libraryRowDetail,
  libraryRows,
  parseKnownLibraries,
  rememberLibrary,
} from "./known-libraries";

const home = {
  instanceId: "home",
  origin: "http://192.168.1.20:1993",
  name: "Home NAS",
};

test("saved libraries are validated", () => {
  expect(parseKnownLibraries(null)).toEqual([]);
  expect(parseKnownLibraries("not json")).toEqual([]);
  expect(
    parseKnownLibraries(
      JSON.stringify([
        { ...home, lastConnectedAt: 1 },
        { ...home, lastConnectedAt: 2 },
        {
          ...home,
          instanceId: "x",
          origin: "javascript:alert(1)",
          lastConnectedAt: 1,
        },
        {
          ...home,
          instanceId: "y",
          origin: "http://a:1/path",
          lastConnectedAt: 1,
        },
      ]),
    ),
  ).toEqual([{ ...home, lastConnectedAt: 1 }]);
});

test("remembering follows a library to its new address", () => {
  let libraries = rememberLibrary([], home, 1);
  libraries = rememberLibrary(
    libraries,
    { ...home, origin: "http://192.168.1.77:1993" },
    2,
  );
  expect(libraries).toEqual([
    { ...home, origin: "http://192.168.1.77:1993", lastConnectedAt: 2 },
  ]);
  expect(forgetLibrary(libraries, "home")).toEqual([]);
});

test("rows put the current library first and mark what is online", () => {
  const known = [
    { ...home, lastConnectedAt: 10 },
    {
      instanceId: "studio",
      origin: "http://10.0.0.5:1993",
      name: "Studio",
      lastConnectedAt: 20,
    },
  ];
  const rows = libraryRows({
    known,
    nearby: [
      {
        instanceId: "home",
        origin: "http://192.168.1.99:1993",
        name: "Home NAS",
      },
      {
        instanceId: "friend",
        origin: "http://192.168.1.50:1993",
        name: "Friend",
      },
    ],
    currentInstanceId: "home",
    accountInstanceIds: ["home", "home", "studio"],
  });
  expect(
    rows.map((row) => [
      row.name,
      row.current,
      row.known,
      row.nearby,
      row.accounts,
    ]),
  ).toEqual([
    ["Home NAS", true, true, true, 2],
    ["Studio", false, true, false, 1],
    ["Friend", false, false, true, 0],
  ]);
  expect(rows[0]!.origin).toBe("http://192.168.1.99:1993");
});

test("row details read naturally", () => {
  const now = 10 * 86_400_000;
  const [row] = libraryRows({
    known: [{ ...home, lastConnectedAt: now - 3 * 86_400_000 }],
    nearby: [],
    currentInstanceId: null,
    accountInstanceIds: ["home"],
  });
  expect(libraryRowDetail(row!, now)).toBe(
    "192.168.1.20:1993 · 1 account · used 3 days ago",
  );
});
