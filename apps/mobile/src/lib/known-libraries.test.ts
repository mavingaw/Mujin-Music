import { expect, test } from "bun:test";
import {
  forgetLibrary,
  isRemoteRoute,
  isTrustedRoute,
  libraryRoutes,
  libraryRowDetail,
  libraryRows,
  parseKnownLibraries,
  pickReachableRoute,
  rememberLibrary,
  setRemoteOrigins,
  touchLibrary,
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

test("away-from-home addresses are kept per library and trusted only there", () => {
  let libraries = rememberLibrary([], home, 1);
  libraries = setRemoteOrigins(libraries, "home", [
    "http://music-nas.tail1234.ts.net:1993",
    "https://music.example.com",
    home.origin,
    "javascript:alert(1)",
  ]);
  const [library] = libraries;
  expect(library?.remoteOrigins).toEqual([
    "http://music-nas.tail1234.ts.net:1993",
    "https://music.example.com",
  ]);
  expect(libraryRoutes(library!)).toEqual([
    home.origin,
    "http://music-nas.tail1234.ts.net:1993",
    "https://music.example.com",
  ]);
  expect(isRemoteRoute(library, "https://music.example.com")).toBe(true);
  expect(isRemoteRoute(library, home.origin)).toBe(false);
  expect(isTrustedRoute(libraries, "home", "https://music.example.com")).toBe(
    true,
  );
  expect(isTrustedRoute(libraries, "home", "https://evil.example")).toBe(false);
  expect(isTrustedRoute(libraries, "other", home.origin)).toBe(false);
  // Unchanged lists keep the same array (no needless secure-store writes).
  expect(
    setRemoteOrigins(libraries, "home", [
      "http://music-nas.tail1234.ts.net:1993",
      "https://music.example.com",
    ]),
  ).toBe(libraries);
  // Reconnecting at home keeps the away addresses; touching keeps the home one.
  const again = rememberLibrary(libraries, home, 5);
  expect(again[0]?.remoteOrigins?.length).toBe(2);
  const touched = touchLibrary(again, "home", 9);
  expect(touched[0]?.origin).toBe(home.origin);
  expect(touched[0]?.lastConnectedAt).toBe(9);
  // Saved and read back.
  expect(
    parseKnownLibraries(JSON.stringify(touched))[0]?.remoteOrigins,
  ).toEqual(library?.remoteOrigins);
  // Clearing them.
  expect(
    setRemoteOrigins(touched, "home", [])[0]?.remoteOrigins,
  ).toBeUndefined();
});

test("away from home, the first route that answers as the library wins", async () => {
  const routes = [
    home.origin,
    "http://music-nas.tail1234.ts.net:1993",
    "https://music.example.com",
  ];
  const answers: Record<string, string | Error> = {
    [home.origin]: new Error("timeout"),
    "http://music-nas.tail1234.ts.net:1993": "home",
    "https://music.example.com": "home",
  };
  const probe = async (origin: string) => {
    const answer = answers[origin];
    if (answer instanceof Error) throw answer;
    return { instanceId: answer!, manifest: { origin } };
  };
  expect((await pickReachableRoute(routes, "home", probe))?.origin).toBe(
    "http://music-nas.tail1234.ts.net:1993",
  );
  // Back home, the home address wins.
  answers[home.origin] = "home";
  expect((await pickReachableRoute(routes, "home", probe))?.origin).toBe(
    home.origin,
  );
  // A different library answering an address is never used.
  answers[home.origin] = new Error("down");
  answers["http://music-nas.tail1234.ts.net:1993"] = "someone-else";
  expect((await pickReachableRoute(routes, "home", probe))?.origin).toBe(
    "https://music.example.com",
  );
  answers["https://music.example.com"] = new Error("down");
  expect(await pickReachableRoute(routes, "home", probe)).toBeNull();
});
