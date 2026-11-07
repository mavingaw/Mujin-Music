import {
  api,
  beginSignIn,
  clearConfig,
  completeSignInFromUrl,
  getConfig,
  imageUrl,
  appOrigin,
  isDesktop,
  isSignedIn,
  redirectUrl,
  saveConfig,
  signOut,
} from "./bungie.js";
import { loadManifest } from "./manifest.js";
import {
  ARMOR_SLOTS,
  WEAPON_SLOTS,
  armorForClass,
  buildInventory,
  equippedArmorTotals,
  statNames,
} from "./inventory.js";
import { equipGameLoadout, planEquip, runPlan } from "./equip.js";
import { setCounts } from "./optimizer.js";
import { equippedPower, maxPowerLoadout, POWER_SLOTS } from "./power.js";

const app = document.getElementById("app");
const topbar = document.getElementById("topbar-actions");
const toastEl = document.getElementById("toast");
toastEl.addEventListener("click", () => (toastEl.hidden = true));

const PROFILE_COMPONENTS = [100, 102, 200, 201, 205, 206, 300, 304].join(",");
const LOADOUTS_KEY = "d2lo.loadouts";
const SLOT_LABELS = {
  helmet: "Helmet",
  gauntlets: "Arms",
  chest: "Chest",
  legs: "Legs",
  classItem: "Class item",
  kinetic: "Kinetic",
  energy: "Energy",
  power: "Power",
  subclass: "Subclass",
};

// Weight presets, in stat order: Weapons, Health, Class, Grenade, Super, Melee.
// The solo presets lean on survivability: Health past 100 strengthens shields
// and Class past 100 adds an overshield on every class ability.
const PRESETS = [
  {
    name: "Solo dungeon",
    weights: [6, 10, 8, 5, 3, 4],
    minimums: [0, 100, 100, 0, 0, 0],
  },
  {
    name: "Solo boss damage",
    weights: [10, 7, 4, 3, 8, 2],
    minimums: [0, 70, 0, 0, 0, 0],
  },
  { name: "Balanced", weights: [5, 5, 5, 5, 5, 5] },
  { name: "Survival", weights: [3, 10, 7, 3, 2, 3] },
  { name: "Grenades", weights: [3, 6, 4, 10, 3, 2] },
  { name: "Melee", weights: [2, 6, 4, 3, 2, 10] },
  { name: "Super", weights: [3, 6, 4, 3, 10, 2] },
  { name: "Ability spam", weights: [1, 5, 8, 8, 2, 8] },
  { name: "Weapons", weights: [10, 6, 4, 3, 2, 3] },
];

// What each Armor 3.0 stat does, below and above 100 (stat order as above).
const STAT_HELP = [
  "0–100: reload, handling and damage against regular enemies. 101–200: more ammo from bricks and more damage to bosses.",
  "0–100: healing from Orbs of Power and flinch resistance. 101–200: stronger shields that recharge faster.",
  "0–100: faster class ability. 101–200: an overshield every time you use your class ability.",
  "0–100: faster grenade. 101–200: more grenade damage.",
  "0–100: faster Super. 101–200: more Super damage.",
  "0–100: faster melee. 101–200: more melee damage, including Glaives.",
];

const state = {
  manifest: null,
  membership: null,
  inventory: null,
  characterId: null,
  tab: "power",
  results: null,
  searching: false,
  busy: false,
};

// ---------- helpers ----------

const esc = (value) =>
  String(value ?? "").replace(
    /[&<>"']/g,
    (c) =>
      ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[
        c
      ],
  );

let toastTimer;
function toast(message, { sticky = false, error = false } = {}) {
  clearTimeout(toastTimer);
  toastEl.textContent = message;
  toastEl.classList.toggle("error", error);
  toastEl.hidden = false;
  if (!sticky) toastTimer = setTimeout(() => (toastEl.hidden = true), 6000);
}

function readStore(key, fallback) {
  try {
    return JSON.parse(localStorage.getItem(key) ?? "null") ?? fallback;
  } catch {
    return fallback;
  }
}
function writeStore(key, value) {
  try {
    localStorage.setItem(key, JSON.stringify(value));
  } catch {
    toast("This browser would not save that.", { error: true });
  }
}

const character = () =>
  state.inventory?.characters.find((c) => c.id === state.characterId);
const itemById = (id) => state.inventory?.items.find((i) => i.id === id);
const names = () => statNames(state.manifest);

function locationLabel(item) {
  if (item.owner === "vault") return "Vault";
  if (item.owner === state.characterId) return item.equipped ? "Worn" : "Here";
  const other = state.inventory.characters.find((c) => c.id === item.owner);
  return other ? other.className : "Elsewhere";
}

function tile(item, { label = true } = {}) {
  if (!item) return `<div class="tile tile-empty" title="Missing item"></div>`;
  const where = label
    ? `<span class="tile-where">${esc(locationLabel(item))}</span>`
    : "";
  return `<div class="tile ${item.exotic ? "exotic" : ""}" title="${esc(item.name)} · ${esc(item.typeName)}${item.power ? ` · Power ${item.power}` : ""}">
      <img src="${esc(imageUrl(item.icon))}" alt="" loading="lazy" />
      ${item.watermark ? `<img class="tile-watermark" src="${esc(imageUrl(item.watermark))}" alt="" loading="lazy" />` : ""}
      ${where}
    </div>`;
}

function statBars(totals, compare, cap) {
  return `<div class="stats">${names()
    .map((name, i) => {
      const value = totals[i];
      const pct = Math.min(100, (value / cap) * 100);
      const diff = compare ? value - compare[i] : 0;
      const diffHtml =
        compare && diff !== 0
          ? `<span class="diff ${diff > 0 ? "up" : "down"}">${diff > 0 ? "+" : ""}${diff}</span>`
          : "";
      return `<div class="stat">
          <span class="stat-name">${esc(name)}</span>
          <span class="bar"><span style="width:${pct}%" class="${value > cap ? "over" : ""}"></span></span>
          <span class="stat-value">${value}${diffHtml}</span>
        </div>`;
    })
    .join("")}</div>`;
}

// ---------- screens ----------

function renderSetup(error) {
  topbar.innerHTML = "";
  const config = getConfig() ?? {};
  const origin = appOrigin();
  app.innerHTML = `
    <section class="card narrow">
      <h1>Connect to Bungie.net</h1>
      <p>This app talks to Bungie.net with your own developer app, so it only ever works for you. It takes about two minutes, once.</p>
      <ol class="steps">
        <li>Open <a href="https://www.bungie.net/en/Application" target="_blank" rel="noopener">bungie.net/en/Application</a> and choose <b>Create New App</b>.</li>
        <li>Set <b>OAuth Client Type</b> to <b>Confidential</b>.</li>
        <li>Set <b>Redirect URL</b> to <code>${esc(redirectUrl())}</code></li>
        <li>Under <b>Scope</b>, tick <b>Read your Destiny vault and character inventory</b> and <b>Move or equip Destiny gear and other items</b>.</li>
        <li>Set <b>Origin Header</b> to <code>${esc(origin)}</code></li>
        <li>Save, then copy the values below from the app's page.</li>
      </ol>
      <form id="setup-form" class="form">
        <label>API Key <input name="apiKey" required autocomplete="off" value="${esc(config.apiKey)}" /></label>
        <label>OAuth client_id <input name="clientId" required inputmode="numeric" autocomplete="off" value="${esc(config.clientId)}" /></label>
        <label>OAuth client_secret <input name="clientSecret" type="password" autocomplete="off" value="${esc(config.clientSecret)}" />
          <small>Needed for a Confidential app; it keeps you signed in for 90 days. Stored only in this browser.</small></label>
        ${error ? `<p class="error-text">${esc(error)}</p>` : ""}
        <button class="primary" type="submit">Save and sign in</button>
      </form>
    </section>`;
  document.getElementById("setup-form").addEventListener("submit", (event) => {
    event.preventDefault();
    const data = new FormData(event.target);
    saveConfig({
      apiKey: data.get("apiKey").trim(),
      clientId: data.get("clientId").trim(),
      clientSecret: data.get("clientSecret").trim(),
    });
    signIn();
  });
}

function renderSignIn(error) {
  topbar.innerHTML = `<button class="ghost" id="edit-setup">App settings</button>`;
  document.getElementById("edit-setup").onclick = () => renderSetup();
  app.innerHTML = `
    <section class="card narrow center">
      <h1>Sign in</h1>
      <p>Sign in with Bungie.net to load your characters, vault and gear.</p>
      ${error ? `<p class="error-text">${esc(error)}</p>` : ""}
      <button class="primary" id="sign-in">Sign in with Bungie.net</button>
    </section>`;
  document.getElementById("sign-in").onclick = signIn;
}

async function signIn() {
  try {
    await beginSignIn();
    // In the desktop app sign-in finishes here; a browser has left the page.
    if (isDesktop) await start();
  } catch (error) {
    renderSignIn(error.message);
  }
}

function renderLoading(message) {
  app.innerHTML = `<section class="card narrow center"><div class="spinner" aria-hidden="true"></div><p>${esc(message)}</p></section>`;
}

function renderTopbar() {
  const m = state.membership;
  topbar.innerHTML = `
    <span class="who">${esc(m?.name ?? "")}</span>
    <button class="ghost" id="refresh" ${state.busy ? "disabled" : ""}>Refresh</button>
    <button class="ghost" id="sign-out">Sign out</button>`;
  document.getElementById("refresh").onclick = () => refreshInventory();
  document.getElementById("sign-out").onclick = () => {
    signOut();
    renderSignIn();
  };
}

function renderMain() {
  renderTopbar();
  const chars = state.inventory.characters;
  app.innerHTML = `
    <nav class="characters" aria-label="Characters">
      ${chars
        .map(
          (
            c,
          ) => `<button class="character ${c.id === state.characterId ? "active" : ""}" data-char="${esc(c.id)}"
              style="background-image:url('${esc(imageUrl(c.emblemBackground))}')">
            <span class="char-class">${esc(c.className)}</span>
            <span class="char-light">${esc(c.light)}</span>
          </button>`,
        )
        .join("")}
    </nav>
    <nav class="tabs" role="tablist">
      ${[
        ["power", "Max Power"],
        ["builds", "Best builds"],
        ["loadouts", "Loadouts"],
        ["gear", "Equipped"],
      ]
        .map(
          ([id, label]) =>
            `<button role="tab" aria-selected="${state.tab === id}" class="tab ${state.tab === id ? "active" : ""}" data-tab="${id}">${label}</button>`,
        )
        .join("")}
    </nav>
    <div id="panel"></div>`;

  app.querySelectorAll("[data-char]").forEach((el) => {
    el.onclick = () => {
      state.characterId = el.dataset.char;
      state.results = null;
      renderMain();
    };
  });
  app.querySelectorAll("[data-tab]").forEach((el) => {
    el.onclick = () => {
      state.tab = el.dataset.tab;
      renderMain();
    };
  });

  if (state.tab === "power") renderPower();
  else if (state.tab === "builds") renderBuilds();
  else if (state.tab === "loadouts") renderLoadouts();
  else renderGear();
}

// ---------- builds ----------

function builderSettings() {
  const key = `d2lo.builder.${character().classType}`;
  return {
    key,
    value: readStore(key, {
      weights: PRESETS[0].weights.slice(),
      minimums: PRESETS[0].minimums.slice(),
      exotic: "any",
      set: null,
      cap: 200,
      depth: 14,
    }),
  };
}

function renderBuilds() {
  const panel = document.getElementById("panel");
  const char = character();
  const { key, value: settings } = builderSettings();
  const armor = armorForClass(state.inventory.items, char.classType);
  const current = equippedArmorTotals(state.inventory.items, char.id);

  const exotics = new Map();
  for (const slot of ARMOR_SLOTS) {
    for (const piece of armor[slot]) {
      if (!piece.exotic) continue;
      const item = itemById(piece.id);
      exotics.set(piece.hash, item.name);
    }
  }
  const exoticOptions = [...exotics.entries()].sort((a, b) =>
    a[1].localeCompare(b[1]),
  );
  // Armor sets this class owns pieces of, and in how many different slots.
  const setSlots = new Map();
  for (const slot of ARMOR_SLOTS) {
    for (const piece of armor[slot]) {
      if (!piece.set) continue;
      if (!setSlots.has(piece.set)) setSlots.set(piece.set, new Set());
      setSlots.get(piece.set).add(slot);
    }
  }
  const setOptions = [...setSlots.entries()]
    .filter(([, slots]) => slots.size >= 2)
    .map(([hash, slots]) => ({
      hash,
      slots: slots.size,
      name: state.manifest.sets?.[hash]?.name || "Armor set",
    }))
    .sort((a, b) => a.name.localeCompare(b.name));
  const setValue = settings.set
    ? `${settings.set.hash}:${settings.set.count}`
    : "";
  const counts = ARMOR_SLOTS.map(
    (s) => `${SLOT_LABELS[s]} ${armor[s].length}`,
  ).join(" · ");

  panel.innerHTML = `
    <div class="builds-layout">
      <section class="card settings">
        <h2>What matters to you</h2>
        <p class="muted">Higher weight means the stat matters more. Set a minimum to require it. Points above the cap are ignored.</p>
        <div class="presets">${PRESETS.map((p, i) => `<button class="chip" data-preset="${i}">${p.name}</button>`).join("")}</div>
        <div class="weights">
          ${names()
            .map(
              (name, i) => `<div class="weight-row">
                <label for="w${i}" title="${esc(STAT_HELP[i])}">${esc(name)}</label>
                <input type="range" id="w${i}" min="0" max="10" step="1" value="${settings.weights[i]}" data-weight="${i}" />
                <output id="wo${i}">${settings.weights[i]}</output>
                <label class="min">min <input type="number" min="0" max="300" step="5" value="${settings.minimums[i]}" data-min="${i}" aria-label="Minimum ${esc(name)}" /></label>
              </div>`,
            )
            .join("")}
        </div>
        <details class="stat-help">
          <summary>What each stat does</summary>
          <dl>${names()
            .map(
              (name, i) => `<dt>${esc(name)}</dt><dd>${esc(STAT_HELP[i])}</dd>`,
            )
            .join("")}</dl>
          <p class="muted small">Solo tip: Health 100+ and Class 100+ keep you alive (stronger shields, an overshield on every class ability). Then push the stat your build's ability uses.</p>
        </details>
        <label class="field">Exotic
          <select id="exotic">
            <option value="any" ${settings.exotic === "any" ? "selected" : ""}>Best exotic (or none)</option>
            <option value="none" ${settings.exotic === "none" ? "selected" : ""}>No exotic</option>
            ${exoticOptions
              .map(
                ([hash, name]) =>
                  `<option value="${hash}" ${String(settings.exotic) === String(hash) ? "selected" : ""}>${esc(name)}</option>`,
              )
              .join("")}
          </select>
        </label>
        <label class="field">Set bonus
          <select id="set">
            <option value="" ${setValue === "" ? "selected" : ""}>Any (no set required)</option>
            ${setOptions
              .flatMap((o) =>
                [2, 4]
                  .filter((n) => o.slots >= n)
                  .map(
                    (n) =>
                      `<option value="${o.hash}:${n}" ${setValue === `${o.hash}:${n}` ? "selected" : ""}>${esc(o.name)}: ${n}-piece</option>`,
                  ),
              )
              .join("")}
          </select>
        </label>
        <div class="field-row">
          <label class="field">Stat cap <input type="number" id="cap" min="50" max="300" step="10" value="${settings.cap}" /></label>
          <label class="field">Search
            <select id="depth">
              ${[
                [10, "Quick"],
                [14, "Normal"],
                [20, "Deep"],
              ]
                .map(
                  ([d, l]) =>
                    `<option value="${d}" ${settings.depth === d ? "selected" : ""}>${l}</option>`,
                )
                .join("")}
            </select>
          </label>
        </div>
        <p class="muted small">Armor considered: ${esc(counts)}</p>
        <button class="primary wide" id="find" ${state.searching ? "disabled" : ""}>${state.searching ? "Searching…" : "Find my best builds"}</button>
      </section>
      <section class="results">
        <div class="card current">
          <h3>Wearing now</h3>
          ${statBars(current, null, settings.cap)}
        </div>
        <div id="result-list">${renderResults(current, settings.cap)}</div>
      </section>
    </div>`;

  const save = () => writeStore(key, settings);
  panel.querySelectorAll("[data-weight]").forEach((el) => {
    el.oninput = () => {
      settings.weights[el.dataset.weight] = Number(el.value);
      document.getElementById(`wo${el.dataset.weight}`).textContent = el.value;
      save();
    };
  });
  panel.querySelectorAll("[data-min]").forEach((el) => {
    el.onchange = () => {
      settings.minimums[el.dataset.min] = Math.max(0, Number(el.value) || 0);
      save();
    };
  });
  panel.querySelectorAll("[data-preset]").forEach((el) => {
    el.onclick = () => {
      const preset = PRESETS[el.dataset.preset];
      settings.weights = preset.weights.slice();
      settings.minimums = (preset.minimums ?? [0, 0, 0, 0, 0, 0]).slice();
      save();
      renderBuilds();
    };
  });
  document.getElementById("exotic").onchange = (e) => {
    const v = e.target.value;
    settings.exotic = v === "any" || v === "none" ? v : Number(v);
    save();
  };
  document.getElementById("set").onchange = (e) => {
    const [hash, count] = e.target.value.split(":").map(Number);
    settings.set = hash ? { hash, count } : null;
    save();
  };
  document.getElementById("cap").onchange = (e) => {
    settings.cap = Math.max(50, Number(e.target.value) || 200);
    save();
  };
  document.getElementById("depth").onchange = (e) => {
    settings.depth = Number(e.target.value);
    save();
  };
  document.getElementById("find").onclick = () => findBuilds(armor, settings);
  bindResultButtons();
}

function renderResults(current, cap) {
  const results = state.results;
  if (!results) {
    return `<div class="card empty">Pick what matters, then press <b>Find my best builds</b>.</div>`;
  }
  if (results.builds.length === 0) {
    return `<div class="card empty">No combination of your armor reaches those minimums with that exotic and set bonus. Lower a minimum, or pick a different exotic or set.</div>`;
  }
  return (
    results.builds
      .map((build, index) => {
        const items = build.pieces.map((p) => itemById(p.id));
        const wearing = items.every(
          (i) => i?.owner === state.characterId && i.equipped,
        );
        const total = build.totals.reduce((a, b) => a + b, 0);
        const armorPower = Math.floor(build.power / build.pieces.length);
        const bonuses = setBonusesHtml(build.pieces);
        return `<article class="card build">
        <header class="build-head">
          <span class="rank">#${index + 1}</span>
          <span class="muted">Total ${total}${build.wasted ? ` · ${build.wasted} over cap` : ""}${armorPower ? ` · Armor Power ${armorPower}` : ""}</span>
          ${wearing ? `<span class="badge">Wearing</span>` : ""}
        </header>
        <div class="tiles">${items.map((i) => tile(i)).join("")}</div>
        ${statBars(build.totals, current, cap)}
        ${bonuses}
        <div class="actions">
          <button class="primary" data-equip-build="${index}" ${state.busy || wearing ? "disabled" : ""}>Equip</button>
          <button class="ghost" data-save-build="${index}">Save as loadout</button>
        </div>
      </article>`;
      })
      .join("") +
    `<p class="muted small center">Checked ${results.combinationsChecked.toLocaleString()} combinations.</p>`
  );
}

/** Lists the set bonuses a group of armor pieces turns on. */
function setBonusesHtml(pieces) {
  const lines = [];
  for (const [hash, count] of setCounts(pieces)) {
    const set = state.manifest.sets?.[hash];
    if (!set) continue;
    for (const perk of set.perks) {
      if (count < perk.count) continue;
      lines.push(
        `<li title="${esc(perk.description)}"><b>${esc(set.name)} (${perk.count}-piece):</b> ${esc(perk.name || "Set bonus")}${perk.description ? `<span class="muted"> · ${esc(perk.description)}</span>` : ""}</li>`,
      );
    }
  }
  return lines.length ? `<ul class="set-bonuses">${lines.join("")}</ul>` : "";
}

function bindResultButtons() {
  document.querySelectorAll("[data-equip-build]").forEach((el) => {
    el.onclick = () => {
      const build = state.results.builds[el.dataset.equipBuild];
      equip(
        build.pieces.map((p) => p.id),
        "build",
      );
    };
  });
  document.querySelectorAll("[data-save-build]").forEach((el) => {
    el.onclick = async () => {
      const build = state.results.builds[el.dataset.saveBuild];
      const name = await askName(`${character().className} build`);
      if (!name) return;
      addLoadout(
        name,
        build.pieces.map((p) => p.id),
      );
      toast(`Saved “${name}”.`);
    };
  });
}

let worker;
function findBuilds(armor, settings) {
  state.searching = true;
  renderBuilds();
  worker ??= new Worker(new URL("./optimizer-worker.js", import.meta.url), {
    type: "module",
  });
  worker.onmessage = (event) => {
    state.searching = false;
    if (event.data.ok) state.results = event.data.result;
    else toast(`The search failed: ${event.data.error}`, { error: true });
    if (state.tab === "builds") renderBuilds();
  };
  worker.postMessage({
    piecesBySlot: armor,
    options: {
      weights: settings.weights,
      minimums: settings.minimums,
      exotic: settings.exotic,
      set: settings.set,
      cap: settings.cap,
      depth: settings.depth,
      limit: 10,
    },
  });
}

// ---------- loadouts ----------

const allLoadouts = () => readStore(LOADOUTS_KEY, []);

function addLoadout(name, itemIds) {
  const char = character();
  const loadouts = allLoadouts();
  loadouts.push({
    id: crypto.randomUUID(),
    name,
    classType: char.classType,
    characterId: char.id,
    items: itemIds,
    created: Date.now(),
  });
  writeStore(LOADOUTS_KEY, loadouts);
}

function renderLoadouts() {
  const panel = document.getElementById("panel");
  const char = character();
  const mine = allLoadouts().filter((l) => l.classType === char.classType);
  const game = state.inventory.gameLoadouts[char.id] ?? [];
  const m = state.manifest;

  const gameHtml = game
    .map((loadout, index) => {
      if (
        !loadout.items?.some(
          (i) => i.itemInstanceId && i.itemInstanceId !== "0",
        )
      )
        return "";
      const name =
        m.loadoutNames[loadout.nameHash]?.name || `Loadout ${index + 1}`;
      const color = m.loadoutColors[loadout.colorHash]?.icon;
      const icon = m.loadoutIcons[loadout.iconHash]?.icon;
      return `<div class="game-loadout">
          <span class="game-icon" style="${color ? `background-image:url('${esc(imageUrl(color))}')` : ""}">
            ${icon ? `<img src="${esc(imageUrl(icon))}" alt="" />` : ""}
          </span>
          <span>${esc(name)}</span>
          <button class="ghost" data-game="${index}" ${state.busy ? "disabled" : ""}>Equip</button>
        </div>`;
    })
    .join("");

  panel.innerHTML = `
    <section class="card">
      <div class="section-head">
        <h2>Your loadouts</h2>
        <button class="primary" id="save-current">Save what I'm wearing</button>
      </div>
      ${
        mine.length
          ? mine
              .map((loadout) => {
                const items = loadout.items.map(itemById);
                const missing = items.filter((i) => !i).length;
                return `<article class="loadout">
                  <div class="loadout-head">
                    <h3>${esc(loadout.name)}</h3>
                    ${missing ? `<span class="badge warn">${missing} missing</span>` : ""}
                  </div>
                  <div class="tiles">${items.map((i) => tile(i)).join("")}</div>
                  <div class="actions">
                    <button class="primary" data-equip-loadout="${esc(loadout.id)}" ${state.busy ? "disabled" : ""}>Equip</button>
                    <button class="ghost danger" data-delete-loadout="${esc(loadout.id)}">Delete</button>
                  </div>
                </article>`;
              })
              .join("")
          : `<p class="muted">No saved loadouts for your ${esc(char.className)} yet. Save what you're wearing, or save a build from <b>Best builds</b>.</p>`
      }
    </section>
    <section class="card">
      <h2>In-game loadouts</h2>
      ${gameHtml || `<p class="muted">No in-game loadouts saved on this character.</p>`}
    </section>`;

  document.getElementById("save-current").onclick = async () => {
    const ids = state.inventory.items
      .filter((i) => i.owner === char.id && i.equipped)
      .map((i) => i.id);
    const name = await askName(`${char.className} loadout`);
    if (!name) return;
    addLoadout(name, ids);
    renderLoadouts();
    toast(`Saved “${name}”.`);
  };
  panel.querySelectorAll("[data-equip-loadout]").forEach((el) => {
    el.onclick = () => {
      const loadout = allLoadouts().find(
        (l) => l.id === el.dataset.equipLoadout,
      );
      // Subclasses can't leave the character they belong to.
      const ids = loadout.items.filter((id) => {
        const item = itemById(id);
        return item && (item.slot !== "subclass" || item.owner === char.id);
      });
      equip(ids, loadout.name);
    };
  });
  panel.querySelectorAll("[data-delete-loadout]").forEach((el) => {
    el.onclick = async () => {
      const loadout = allLoadouts().find(
        (l) => l.id === el.dataset.deleteLoadout,
      );
      if (!(await askConfirm(`Delete “${loadout.name}”?`))) return;
      writeStore(
        LOADOUTS_KEY,
        allLoadouts().filter((l) => l.id !== loadout.id),
      );
      renderLoadouts();
    };
  });
  panel.querySelectorAll("[data-game]").forEach((el) => {
    el.onclick = async () => {
      state.busy = true;
      toast("Equipping in-game loadout…", { sticky: true });
      try {
        await equipGameLoadout(
          Number(el.dataset.game),
          char.id,
          state.membership.type,
        );
        toast("Loadout equipped.");
      } catch (error) {
        toast(error.message, { error: true });
      }
      state.busy = false;
      await refreshInventory({ quiet: true });
    };
  });
}

// ---------- max power ----------

function renderPower() {
  const panel = document.getElementById("panel");
  const char = character();
  const now = equippedPower(state.inventory.items, char.id);
  const best = maxPowerLoadout(state.inventory.items, char.classType);
  const ids = POWER_SLOTS.map((slot) => best.items[slot]?.id).filter(Boolean);
  const wearing = ids.every((id) => {
    const item = itemById(id);
    return item.owner === char.id && item.equipped;
  });
  const gain = best.power - now.power;

  panel.innerHTML = `
    <section class="card power-card">
      <div class="power-summary">
        <div><div class="muted small">Wearing now</div><div class="power-number">${now.power}</div></div>
        <div class="power-arrow" aria-hidden="true">→</div>
        <div><div class="muted small">Your max Power</div><div class="power-number max">${best.power}</div></div>
        ${gain > 0 ? `<span class="badge">+${gain}</span>` : `<span class="badge">At max</span>`}
      </div>
      <p class="muted">Power is the average of your 3 weapons and 5 armor pieces. This picks the highest-Power item you own for every slot, from this character, your vault and your other characters, keeping to one exotic weapon and one exotic armor piece. Higher Power means you deal more damage and take less in dungeons.</p>
      ${best.missing.length ? `<p class="error-text">Nothing to equip for: ${esc(best.missing.map((s) => SLOT_LABELS[s]).join(", "))}.</p>` : ""}
      <div class="gear-grid">
        ${POWER_SLOTS.map((slot) => {
          const item = best.items[slot];
          return `<div class="gear-slot">
            ${tile(item)}
            <div><div class="small muted">${SLOT_LABELS[slot]}</div><div>${esc(item?.name ?? "Empty")}</div>
            ${item ? `<div class="small">Power ${item.power}</div>` : ""}</div>
          </div>`;
        }).join("")}
      </div>
      <div class="actions">
        <button class="primary" id="equip-max" ${state.busy || wearing || !ids.length ? "disabled" : ""}>${wearing ? "Already wearing max Power" : "Equip max Power"}</button>
        <button class="ghost" id="save-max" ${ids.length ? "" : "disabled"}>Save as loadout</button>
      </div>
      <p class="muted small">Equipping max Power swaps your armor too, which changes your stats. Use <b>Best builds</b> for the strongest stats, then equip your highest-Power weapons on top.</p>
    </section>`;

  document.getElementById("equip-max").onclick = () => equip(ids, "max Power");
  document.getElementById("save-max").onclick = async () => {
    const name = await askName(`${char.className} max Power`);
    if (!name) return;
    addLoadout(name, ids);
    toast(`Saved “${name}”.`);
  };
}

// ---------- dialogs ----------

const dialog = document.getElementById("dialog");

/** In-app replacement for prompt(), which the desktop app doesn't support. */
function askName(defaultValue) {
  return openDialog({ title: "Name this loadout", input: defaultValue });
}

function askConfirm(message) {
  return openDialog({ title: message, confirmLabel: "Delete" }).then(Boolean);
}

function openDialog({ title, input, confirmLabel = "Save" }) {
  dialog.innerHTML = `<form method="dialog" class="dialog-form">
      <h2>${esc(title)}</h2>
      ${input !== undefined ? `<input name="value" value="${esc(input)}" maxlength="60" required />` : ""}
      <div class="actions">
        <button class="ghost" value="cancel" formnovalidate>Cancel</button>
        <button class="primary" value="ok">${esc(confirmLabel)}</button>
      </div>
    </form>`;
  return new Promise((resolve) => {
    dialog.onclose = () => {
      if (dialog.returnValue !== "ok") return resolve(null);
      const field = dialog.querySelector("input");
      resolve(field ? field.value.trim() || null : true);
    };
    dialog.returnValue = "";
    dialog.showModal();
    dialog.querySelector("input")?.select();
  });
}

// ---------- equipped ----------

function renderGear() {
  const panel = document.getElementById("panel");
  const char = character();
  const worn = (slot) =>
    state.inventory.items.find(
      (i) => i.owner === char.id && i.equipped && i.slot === slot,
    );
  const row = (slots) =>
    slots
      .map(
        (slot) => `<div class="gear-slot">
          ${tile(worn(slot), { label: false })}
          <div><div class="small muted">${SLOT_LABELS[slot]}</div><div>${esc(worn(slot)?.name ?? "Empty")}</div>
          ${worn(slot)?.power ? `<div class="small muted">Power ${worn(slot).power}</div>` : ""}</div>
        </div>`,
      )
      .join("");
  panel.innerHTML = `
    <section class="card">
      <h2>Subclass</h2><div class="gear-grid">${row(["subclass"])}</div>
      <h2>Weapons</h2><div class="gear-grid">${row(WEAPON_SLOTS)}</div>
      <h2>Armor</h2><div class="gear-grid">${row(ARMOR_SLOTS)}</div>
      <h3>Armor stats</h3>
      ${statBars(equippedArmorTotals(state.inventory.items, char.id), null, 200)}
    </section>`;
}

// ---------- actions ----------

async function equip(itemIds, label) {
  const char = character();
  const plan = planEquip(state.inventory.items, char.id, itemIds);
  state.busy = true;
  renderMain();
  toast(`Equipping ${label}…`, { sticky: true });
  try {
    const { failures } = await runPlan(plan, state.membership.type, (line) =>
      toast(line, { sticky: true }),
    );
    if (failures.length) {
      const lines = failures.map(
        (f) => `${f.item?.name ?? "An item"}: ${f.reason}`,
      );
      toast(lines.join("\n"), { error: true, sticky: true });
    } else {
      toast("Equipped.");
    }
  } catch (error) {
    toast(error.message, { error: true });
  }
  state.busy = false;
  await refreshInventory({ quiet: true });
}

async function loadMembership() {
  const data = await api("/User/GetMembershipsForCurrentUser/");
  const memberships = data.destinyMemberships ?? [];
  if (!memberships.length) {
    throw new Error("This Bungie.net account has no Destiny 2 characters.");
  }
  const chosen =
    memberships.find((m) => m.membershipId === data.primaryMembershipId) ??
    memberships.find(
      (m) => !m.crossSaveOverride || m.crossSaveOverride === m.membershipType,
    ) ??
    memberships[0];
  const user = data.bungieNetUser ?? {};
  const code = user.cachedBungieGlobalDisplayNameCode;
  return {
    type: chosen.membershipType,
    id: chosen.membershipId,
    name: user.cachedBungieGlobalDisplayName
      ? `${user.cachedBungieGlobalDisplayName}#${String(code ?? "").padStart(4, "0")}`
      : chosen.displayName,
  };
}

async function refreshInventory({ quiet = false } = {}) {
  if (!quiet) renderLoading("Loading your gear…");
  try {
    const m = state.membership;
    const profile = await api(
      `/Destiny2/${m.type}/Profile/${m.id}/?components=${PROFILE_COMPONENTS}`,
    );
    state.inventory = buildInventory(profile, state.manifest);
    if (!state.inventory.characters.some((c) => c.id === state.characterId)) {
      state.characterId = state.inventory.characters[0]?.id ?? null;
    }
    if (!state.characterId)
      throw new Error("No characters found on this account.");
    renderMain();
  } catch (error) {
    handleError(error);
  }
}

function handleError(error) {
  console.error(error);
  if (error.status === 401 || !isSignedIn()) {
    renderSignIn(error.message);
    return;
  }
  if (state.inventory) {
    toast(error.message, { error: true });
    renderMain();
    return;
  }
  app.innerHTML = `<section class="card narrow center">
      <h1>Something went wrong</h1>
      <p class="error-text">${esc(error.message)}</p>
      <button class="primary" id="retry">Try again</button>
      <button class="ghost" id="settings">App settings</button>
    </section>`;
  document.getElementById("retry").onclick = start;
  document.getElementById("settings").onclick = () => {
    clearConfig();
    signOut();
    renderSetup();
  };
}

async function start() {
  if (!getConfig()?.apiKey) {
    renderSetup();
    return;
  }
  try {
    if (await completeSignInFromUrl()) toast("Signed in.");
  } catch (error) {
    renderSignIn(error.message);
    return;
  }
  if (!isSignedIn()) {
    renderSignIn();
    return;
  }
  try {
    state.manifest = await loadManifest(renderLoading);
    renderLoading("Finding your account…");
    state.membership = await loadMembership();
    await refreshInventory();
  } catch (error) {
    handleError(error);
  }
}

start();
