const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { forgeClient, main, parseArguments } = require("./publish-release.cjs");

/** A forge that keeps its releases in memory and records each request. */
function fakeForge({ flavor, releases = [] }) {
  const calls = [];
  let nextId = 100;
  const state = { releases };
  const reply = (status, value) =>
    new Response(value === undefined ? null : JSON.stringify(value), {
      status,
    });
  const fetch = async (address, init) => {
    const url = new URL(address);
    const method = init.method;
    calls.push({ method, url, headers: init.headers, body: init.body });
    const route = url.pathname.replace(/^\/api\/v1\/repos|^\/repos/, "");
    let match;
    if (
      method === "GET" &&
      (match = route.match(/^\/o\/r\/releases\/tags\/(.+)$/))
    ) {
      const tag = decodeURIComponent(match[1]);
      const release = state.releases.find(
        (candidate) =>
          candidate.tag_name === tag &&
          (flavor === "gitea" || !candidate.draft),
      );
      return release
        ? reply(200, release)
        : reply(404, { message: "Not Found" });
    }
    if (method === "GET" && route === "/o/r/releases/latest") {
      const release = state.releases.findLast(
        (candidate) => !candidate.draft && !candidate.prerelease,
      );
      return release ? reply(200, release) : reply(404, {});
    }
    if (method === "POST" && route === "/o/r/releases") {
      const json = JSON.parse(init.body);
      const release = {
        id: nextId++,
        tag_name: json.tag_name,
        draft: json.draft,
        prerelease: json.prerelease,
        body: json.body,
        assets: [],
      };
      state.releases.push(release);
      return reply(201, release);
    }
    if ((match = route.match(/^\/o\/r\/releases\/(\d+)$/))) {
      const index = state.releases.findIndex((r) => r.id === Number(match[1]));
      if (method === "PATCH") {
        Object.assign(state.releases[index], JSON.parse(init.body));
        return reply(200, state.releases[index]);
      }
      if (method === "DELETE") {
        state.releases.splice(index, 1);
        return reply(204);
      }
    }
    if ((match = route.match(/^\/o\/r\/releases\/(\d+)\/assets$/))) {
      const release = state.releases.find((r) => r.id === Number(match[1]));
      if (method === "GET") {
        return reply(200, release.assets);
      }
      if (method === "POST") {
        const name = url.searchParams.get("name");
        if (flavor === "gitea") {
          assert.ok(init.body instanceof FormData);
          assert.ok(init.body.get("attachment"));
        } else {
          assert.equal(url.hostname, "uploads.github.com");
          assert.equal(
            init.headers["Content-Type"],
            "application/octet-stream",
          );
        }
        release.assets.push({ id: nextId++, name });
        return reply(201, {});
      }
    }
    if (method === "DELETE") {
      const id = Number(route.split("/").pop());
      for (const release of state.releases) {
        release.assets = release.assets.filter((asset) => asset.id !== id);
      }
      return reply(204);
    }
    return reply(500, { message: `unexpected ${method} ${route}` });
  };
  return { fetch, calls, state };
}

function files(t, names) {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), "mujin-release-"));
  t.after(() => fs.rmSync(directory, { recursive: true, force: true }));
  return names.map((name) => {
    const file = path.join(directory, name);
    fs.writeFileSync(file, name);
    return file;
  });
}

function environment(flavor) {
  return {
    FORGE: flavor,
    FORGE_URL:
      flavor === "github" ? "https://github.com" : "http://192.168.1.58:3002/",
    FORGE_REPOSITORY: "o/r",
    FORGE_TOKEN: "secret",
  };
}

for (const flavor of ["gitea", "github"]) {
  test(`${flavor}: a new release is published only after every file is uploaded`, async (t) => {
    const forge = fakeForge({ flavor });
    const [notes, ...packages] = files(t, ["notes.md", "a.exe", "latest.yml"]);
    let published = null;
    await main(
      [
        "publish",
        "--tag",
        "v1.2.3",
        "--target",
        "abc",
        "--title",
        "Mujin Music 1.2.3",
        "--notes-file",
        notes,
        ...packages,
      ],
      environment(flavor),
      { fetch: forge.fetch, stdout: () => {} },
    );
    const release = forge.state.releases[0];
    assert.equal(release.draft, false);
    assert.deepEqual(
      release.assets.map((asset) => asset.name),
      ["a.exe", "latest.yml"],
    );
    // The draft flag is cleared by the last request, after the uploads.
    const last = forge.calls.at(-1);
    assert.equal(last.method, "PATCH");
    assert.deepEqual(JSON.parse(last.body), { draft: false });
    published = forge.calls.find((call) => call.method === "POST");
    assert.equal(JSON.parse(published.body).draft, true);
    const authorization = forge.calls[0].headers.Authorization;
    assert.equal(
      authorization,
      flavor === "github" ? "Bearer secret" : "token secret",
    );
    if (flavor === "gitea") {
      assert.equal(forge.calls[0].url.origin, "http://192.168.1.58:3002");
    } else {
      assert.equal(forge.calls[0].url.origin, "https://api.github.com");
    }

    await assert.rejects(
      main(
        [
          "publish",
          "--tag",
          "v1.2.3",
          "--target",
          "abc",
          "--title",
          "x",
          "--notes-file",
          notes,
          ...packages,
        ],
        environment(flavor),
        { fetch: forge.fetch, stdout: () => {} },
      ),
      /already exists/,
    );
  });

  test(`${flavor}: upload waits for the release and replaces files of the same name`, async (t) => {
    const forge = fakeForge({
      flavor,
      releases: [
        {
          id: 7,
          tag_name: "v1.2.3",
          draft: true,
          assets: [
            { id: 70, name: "latest-mac.yml" },
            { id: 71, name: "a.exe" },
          ],
        },
      ],
    });
    const uploads = files(t, ["latest-mac.yml", "Mujin-Music-1.2.3-arm64.zip"]);
    let waits = 0;
    await main(
      ["upload", "--tag", "v1.2.3", "--wait", "30", ...uploads],
      environment(flavor),
      {
        fetch: forge.fetch,
        sleep: async () => {
          waits++;
          forge.state.releases[0].draft = false;
        },
      },
    );
    assert.equal(waits, 1);
    assert.deepEqual(
      forge.state.releases[0].assets.map((asset) => asset.name).sort(),
      ["Mujin-Music-1.2.3-arm64.zip", "a.exe", "latest-mac.yml"],
    );
    assert.ok(!forge.state.releases[0].assets.some((asset) => asset.id === 70));
  });
}

test("a failed upload removes the draft so the tag can be published again", async (t) => {
  const forge = fakeForge({ flavor: "gitea" });
  const [notes, good] = files(t, ["notes.md", "a.exe"]);
  await assert.rejects(
    main(
      [
        "publish",
        "--tag",
        "v1.2.3",
        "--target",
        "abc",
        "--title",
        "x",
        "--notes-file",
        notes,
        good,
        "/does/not/exist.exe",
      ],
      environment("gitea"),
      { fetch: forge.fetch, stdout: () => {} },
    ),
  );
  assert.equal(forge.state.releases.length, 0);
});

test("prints the newest stable tag and sends Cloudflare Access credentials", async () => {
  const forge = fakeForge({
    flavor: "gitea",
    releases: [
      { id: 1, tag_name: "v1.0.0", draft: false, assets: [] },
      { id: 2, tag_name: "v1.1.0", draft: false, assets: [] },
      {
        id: 3,
        tag_name: "nightly",
        draft: false,
        prerelease: true,
        assets: [],
      },
    ],
  });
  let printed = "";
  await main(
    ["latest-tag"],
    {
      ...environment("gitea"),
      CF_ACCESS_CLIENT_ID: "id",
      CF_ACCESS_CLIENT_SECRET: "s",
    },
    { fetch: forge.fetch, stdout: (text) => (printed += text) },
  );
  assert.equal(printed, "v1.1.0\n");
  assert.equal(forge.calls[0].headers["CF-Access-Client-Id"], "id");
  assert.equal(forge.calls[0].headers["CF-Access-Client-Secret"], "s");
});

test("refuses incomplete configuration and unknown options", () => {
  assert.throws(() =>
    forgeClient({
      flavor: "gitlab",
      url: "https://x.test",
      repository: "o/r",
      token: "t",
    }),
  );
  assert.throws(() =>
    forgeClient({
      flavor: "gitea",
      url: "https://x.test",
      repository: "o r",
      token: "t",
    }),
  );
  const anonymous = fakeForge({ flavor: "github" });
  forgeClient({
    flavor: "github",
    url: "https://github.com",
    repository: "o/r",
    fetch: anonymous.fetch,
  }).latest();
  assert.equal(anonymous.calls[0].headers.Authorization, undefined);
  assert.throws(
    () => parseArguments(["publish", "--clobber"]),
    /Unknown option/,
  );
  assert.throws(() => parseArguments(["publish", "--tag"]), /needs a value/);
});
