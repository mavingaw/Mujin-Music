# Guardian Loadouts

A personal Destiny 2 tool. Sign in with your Bungie.net account and it:

- loads every weapon, armor piece and subclass on your characters and in your
  vault;
- finds your **strongest armor builds** from the real stat numbers on your
  gear, using the stats you care about;
- **equips a build or loadout in one click**, the way DIM does: it pulls items
  from the vault or your other characters, makes room when a slot is full and
  equips exotics last so they don't clash;
- saves your own loadouts and equips your in-game loadouts.

It runs on your own computer and uses your own Bungie developer app, so only
you can use it.

## Run it

You need [Node.js](https://nodejs.org) 20 or newer and OpenSSL (built into
macOS and Linux; on Windows it comes with Git for Windows).

```bash
cd destiny-loadouts
```

```bash
npm start
```

Open <https://localhost:55555/>. The first time, your browser warns that the
certificate is self-signed. It's made on your machine for `localhost` only:
choose **Advanced** and then **Proceed to localhost**. (Bungie only sends
sign-ins back to HTTPS addresses, which is why it isn't plain `http`.)

## Connect it to Bungie.net (once)

1. Open <https://www.bungie.net/en/Application> and choose **Create New App**.
2. **OAuth Client Type**: Confidential.
3. **Redirect URL**: `https://localhost:55555/`
4. **Scope**: tick _Read your Destiny vault and character inventory_ and
   _Move or equip Destiny gear and other items_.
5. **Origin Header**: `https://localhost:55555`
6. Save, then paste the **API Key**, **OAuth client_id** and
   **OAuth client_secret** into the app's first screen.

These values and your sign-in are kept only in your browser's local storage.

## Using it

- **Best builds**: drag the weight sliders for the stats you want (or tap a
  preset like _Grenades_ or _Survival_), add minimums if a stat must reach a
  number, pick an exotic or let it choose, then press _Find my best builds_.
  Each result shows the stats against what you're wearing now. _Equip_ puts it
  on; _Save as loadout_ keeps it.
- **Loadouts**: save what you're wearing (weapons, armor and subclass) and
  equip it again later, or equip one of your in-game loadouts.
- **Equipped**: what the character has on right now.

The game only lets gear be equipped in orbit, a social space, or when you're
offline; in an activity, items still move to the character but stay unequipped.

## How builds are scored

A build's score is the sum of each stat total times its weight. Points above
the **stat cap** (200 by default, the Armor 3.0 maximum) don't count, so the
optimizer won't waste points. With every weight at 0 it maximises the total of
all stats. It checks every combination of your best candidates in each slot
(armor that another piece beats in every stat is skipped), never puts two
exotics together, and drops builds that miss a minimum. _Search: Deep_ looks
at more candidates per slot.

Stats are the numbers Bungie reports for each piece as it is now, so stat mods
already on a piece are counted.

## Tests

```bash
npm test
```
