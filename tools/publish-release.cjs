const fs = require("node:fs");
const path = require("node:path");

/**
 * Creates releases and uploads their files on Gitea (or Forgejo) and on
 * GitHub through their REST APIs, so the release workflows need neither the
 * gh CLI nor GitHub's release actions.
 *
 *   node tools/publish-release.cjs publish --tag v1.2.3 --target <sha>
 *       --title "Mujin Music 1.2.3" --notes-file notes.md [--replace]
 *       [--prerelease] <files...>
 *   node tools/publish-release.cjs upload --tag v1.2.3 [--wait <minutes>] <files...>
 *   node tools/publish-release.cjs latest-tag
 *   node tools/publish-release.cjs exists --tag v1.2.3
 *   node tools/publish-release.cjs delete --tag nightly
 *
 * The environment names the forge:
 *
 *   FORGE              gitea (also Forgejo) or github
 *   FORGE_URL          the forge's web address, e.g. http://192.168.1.58:3002
 *                      or https://github.com
 *   FORGE_REPOSITORY   owner/name
 *   FORGE_TOKEN        a token that may write releases (reading a public
 *                      repository's releases needs none)
 *   CF_ACCESS_CLIENT_ID, CF_ACCESS_CLIENT_SECRET
 *                      optional Cloudflare Access service token, sent with
 *                      every request when the forge sits behind Access
 *
 * A new release is created as a draft, receives all of its files and only
 * then is published, so update checks never see a release whose feed names
 * a file that is not uploaded yet. Uploading a file whose name the release
 * already has replaces it (upload) or is refused (publish without
 * --replace).
 */

const flavors = ["gitea", "github"];

class ForgeError extends Error {
  constructor(message, status) {
    super(message);
    this.status = status;
  }
}

function forgeClient({
  flavor,
  url,
  repository,
  token,
  fetch = globalThis.fetch,
  headers: extraHeaders = {},
  openFile = (file) => fs.openAsBlob(file),
}) {
  if (!flavors.includes(flavor)) {
    throw new Error(`FORGE must be one of ${flavors.join(", ")}.`);
  }
  if (!/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(repository ?? "")) {
    throw new Error(`Not a repository name: ${repository}`);
  }
  const server = new URL(url).href.replace(/\/+$/, "");
  const github = flavor === "github";
  const api = github
    ? `${server === "https://github.com" ? "https://api.github.com" : `${server}/api/v3`}/repos/${repository}`
    : `${server}/api/v1/repos/${repository}`;
  const uploads = github
    ? `${server === "https://github.com" ? "https://uploads.github.com" : `${server}/api/uploads`}/repos/${repository}`
    : api;
  const headers = {
    ...extraHeaders,
    Accept: github ? "application/vnd.github+json" : "application/json",
    // Reading a public repository's releases needs no token.
    ...(token
      ? { Authorization: github ? `Bearer ${token}` : `token ${token}` }
      : {}),
    ...(github ? { "X-GitHub-Api-Version": "2022-11-28" } : {}),
  };

  async function request(method, address, { json, body, contentType } = {}) {
    const response = await fetch(address, {
      method,
      headers: {
        ...headers,
        ...(json !== undefined ? { "Content-Type": "application/json" } : {}),
        ...(contentType ? { "Content-Type": contentType } : {}),
      },
      body: json !== undefined ? JSON.stringify(json) : body,
    });
    if (response.status === 204) {
      return null;
    }
    const text = await response.text();
    if (!response.ok) {
      throw new ForgeError(
        `${method} ${address.replace(/\?.*$/, "")} failed with ${response.status}: ${text.slice(0, 300)}`,
        response.status,
      );
    }
    return text ? JSON.parse(text) : null;
  }

  async function orNull(promise) {
    try {
      return await promise;
    } catch (error) {
      if (error.status === 404) {
        return null;
      }
      throw error;
    }
  }

  const tagPath = (tag) => `/releases/tags/${encodeURIComponent(tag)}`;

  async function assets(release) {
    if (!github) {
      return release.assets ?? [];
    }
    const all = [];
    for (let page = 1; ; page++) {
      const batch = await request(
        "GET",
        `${api}/releases/${release.id}/assets?per_page=100&page=${page}`,
      );
      all.push(...batch);
      if (batch.length < 100) {
        return all;
      }
    }
  }

  return {
    releaseByTag: (tag) => orNull(request("GET", `${api}${tagPath(tag)}`)),
    release: (id) => request("GET", `${api}/releases/${id}`),
    latest: () => orNull(request("GET", `${api}/releases/latest`)),
    create: ({ tag, target, title, notes, draft, prerelease }) =>
      request("POST", `${api}/releases`, {
        json: {
          tag_name: tag,
          target_commitish: target,
          name: title,
          body: notes,
          draft,
          prerelease,
        },
      }),
    update: (id, patch) =>
      request("PATCH", `${api}/releases/${id}`, { json: patch }),
    remove: (id) => request("DELETE", `${api}/releases/${id}`),
    assets,
    deleteAsset: (release, asset) =>
      request(
        "DELETE",
        github
          ? `${api}/releases/assets/${asset.id}`
          : `${api}/releases/${release.id}/assets/${asset.id}`,
      ),
    async uploadAsset(release, file) {
      const name = path.basename(file);
      const blob = await openFile(file);
      const address = `${uploads}/releases/${release.id}/assets?name=${encodeURIComponent(name)}`;
      if (github) {
        return request("POST", address, {
          body: blob,
          contentType: "application/octet-stream",
        });
      }
      const form = new FormData();
      form.append("attachment", blob, name);
      return request("POST", address, { body: form });
    },
  };
}

function assetNames(files) {
  const names = files.map((file) => path.basename(file));
  const duplicate = names.find((name, index) => names.indexOf(name) !== index);
  if (duplicate) {
    throw new Error(`Two files are both named ${duplicate}.`);
  }
  return names;
}

async function uploadAll(client, release, files, { replace }) {
  assetNames(files);
  const existing = await client.assets(release);
  for (const file of files) {
    const name = path.basename(file);
    for (const asset of existing.filter(
      (candidate) => candidate.name === name,
    )) {
      if (!replace) {
        throw new Error(`The release already has ${name}.`);
      }
      await client.deleteAsset(release, asset);
    }
    await client.uploadAsset(release, file);
    console.error(`Uploaded ${name}`);
  }
}

async function publish(
  client,
  { tag, target, title, notes, files, replace, prerelease },
) {
  assetNames(files);
  const existing = await client.releaseByTag(tag);
  if (existing) {
    if (!replace) {
      throw new Error(
        `Release ${tag} already exists; rebuild it with replace.`,
      );
    }
    await uploadAll(client, existing, files, { replace: true });
    await client.update(existing.id, { body: notes });
    return existing;
  }
  const draft = await client.create({
    tag,
    target,
    title,
    notes,
    draft: true,
    prerelease: Boolean(prerelease),
  });
  try {
    await uploadAll(client, draft, files, { replace: false });
  } catch (error) {
    await client.remove(draft.id).catch(() => {});
    throw error;
  }
  return client.update(draft.id, { draft: false });
}

async function waitForRelease(
  client,
  tag,
  { minutes, sleep, log = console.error },
) {
  const deadline = Date.now() + minutes * 60_000;
  for (;;) {
    const release = await client.releaseByTag(tag);
    if (release && !release.draft) {
      return release;
    }
    if (Date.now() >= deadline) {
      throw new Error(
        `Release ${tag} was not published within ${minutes} minutes.`,
      );
    }
    log(`Waiting for release ${tag} to be published...`);
    await sleep();
  }
}

function parseArguments(argv) {
  const [command, ...rest] = argv;
  const options = { files: [] };
  const valued = ["--tag", "--target", "--title", "--notes-file", "--wait"];
  for (let index = 0; index < rest.length; index++) {
    const argument = rest[index];
    if (valued.includes(argument)) {
      const value = rest[++index];
      if (value === undefined) {
        throw new Error(`${argument} needs a value.`);
      }
      options[argument.slice(2)] = value;
    } else if (argument === "--replace" || argument === "--prerelease") {
      options[argument.slice(2)] = true;
    } else if (argument.startsWith("--")) {
      throw new Error(`Unknown option ${argument}.`);
    } else {
      options.files.push(argument);
    }
  }
  return { command, options };
}

async function main(argv, environment = process.env, dependencies = {}) {
  const { command, options } = parseArguments(argv);
  const headers = {};
  if (environment.CF_ACCESS_CLIENT_ID && environment.CF_ACCESS_CLIENT_SECRET) {
    headers["CF-Access-Client-Id"] = environment.CF_ACCESS_CLIENT_ID;
    headers["CF-Access-Client-Secret"] = environment.CF_ACCESS_CLIENT_SECRET;
  }
  const client = forgeClient({
    flavor: environment.FORGE,
    url: environment.FORGE_URL,
    repository: environment.FORGE_REPOSITORY,
    token: environment.FORGE_TOKEN,
    headers,
    ...dependencies,
  });
  const needTag = () => {
    if (!options.tag) {
      throw new Error(`${command} needs --tag.`);
    }
    return options.tag;
  };
  const out = dependencies.stdout ?? ((text) => process.stdout.write(text));

  switch (command) {
    case "publish": {
      for (const name of ["target", "title", "notes-file"]) {
        if (!options[name]) {
          throw new Error(`publish needs --${name}.`);
        }
      }
      if (options.files.length === 0) {
        throw new Error("publish needs the release files.");
      }
      const release = await publish(client, {
        tag: needTag(),
        target: options.target,
        title: options.title,
        notes: fs.readFileSync(options["notes-file"], "utf8"),
        files: options.files,
        replace: Boolean(options.replace),
        prerelease: Boolean(options.prerelease),
      });
      out(`${release.html_url ?? release.id}\n`);
      return;
    }
    case "upload": {
      if (options.files.length === 0) {
        throw new Error("upload needs the files.");
      }
      const tag = needTag();
      const minutes = Number(options.wait ?? 0);
      if (!Number.isFinite(minutes) || minutes < 0) {
        throw new Error("--wait takes a number of minutes.");
      }
      const release =
        minutes > 0
          ? await waitForRelease(client, tag, {
              minutes,
              sleep:
                dependencies.sleep ??
                (() => new Promise((resolve) => setTimeout(resolve, 60_000))),
            })
          : await client.releaseByTag(tag);
      if (!release) {
        throw new Error(`There is no release ${tag}.`);
      }
      await uploadAll(client, release, options.files, { replace: true });
      return;
    }
    case "latest-tag": {
      const release = await client.latest();
      out(`${release?.tag_name ?? ""}\n`);
      return;
    }
    case "exists": {
      out(`${Boolean(await client.releaseByTag(needTag()))}\n`);
      return;
    }
    case "delete": {
      const release = await client.releaseByTag(needTag());
      if (release) {
        await client.remove(release.id);
      }
      return;
    }
    default:
      throw new Error(
        "Usage: publish-release.cjs publish|upload|latest-tag|exists|delete [options] [files]",
      );
  }
}

if (require.main === module) {
  main(process.argv.slice(2)).catch((error) => {
    console.error(error.message);
    process.exitCode = 1;
  });
}

module.exports = {
  forgeClient,
  main,
  parseArguments,
  publish,
  uploadAll,
  waitForRelease,
};
