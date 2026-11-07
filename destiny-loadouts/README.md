# Guardian Loadouts

A personal Destiny 2 app for Windows. Sign in with your Bungie.net account and
it:

- finds your **max Power**: the highest-Power weapon and armor piece you own
  for every slot, across your characters and vault, keeping to one exotic
  weapon and one exotic armor piece. Equip it in one click;
- finds your **strongest armor builds** from the real stat numbers on your
  gear, with presets made for **solo dungeons**, and can build around an
  exotic and a 2- or 4-piece **armor set bonus**;
- **equips anything in one click**, the way DIM does: it pulls items from the
  vault or your other characters, makes room when a slot is full and equips
  exotics last so they don't clash;
- saves your own loadouts and equips your in-game loadouts.

It uses your own Bungie developer app, so only you can use it. Nothing is sent
anywhere except to bungie.net. Your API key, client secret and sign-in are
stored encrypted by Windows, tied to your Windows account.

## Install on Windows

1. Download **Guardian-Loadouts-Setup-x.y.z.exe**:
   - from the repository's **Releases** page (`guardian-loadouts-v…`), or
   - from the **Guardian Loadouts (Windows app)** run on the repository's
     **Actions** tab, under _Artifacts_ (it comes as a .zip with the
     installer inside).
2. Run it. The installer isn't code-signed (a certificate costs money every
   year), so Windows may show _Windows protected your PC_. Choose
   **More info**, then **Run anyway**.
3. Pick where to install and finish. Open **Guardian Loadouts** from the Start
   menu or desktop.

## Connect it to Bungie.net (once)

1. Open <https://www.bungie.net/en/Application> and choose **Create New App**.
2. **OAuth Client Type**: Confidential.
3. **Redirect URL**: `https://localhost:55555/`
4. **Scope**: tick _Read your Destiny vault and character inventory_ and
   _Move or equip Destiny gear and other items_.
5. **Origin Header**: `https://localhost:55555`
6. Save, then paste the **API Key**, **OAuth client_id** and
   **OAuth client_secret** into the app's first screen.
7. Press **Save and sign in**. A Bungie.net window opens: sign in with Steam,
   PlayStation, Xbox or Epic as usual. You stay signed in for 90 days.

The `localhost` address is never actually opened: the app watches for Bungie
sending you back to it and finishes signing in itself.

## Using it

- **Max Power**: your current Power next to the most you can reach with what
  you own. _Equip max Power_ puts it all on.
- **Best builds**: tap a preset (start with _Solo dungeon_), or drag the
  weight sliders for the stats you want and add minimums. Pick an exotic and,
  if you like, a set bonus, then press _Find my best builds_. Each result shows
  its stats against what you're wearing, its armor Power, and the set bonuses
  it turns on. _Equip_ puts it on; _Save as loadout_ keeps it.
- **Loadouts**: save what you're wearing (weapons, armor and subclass) and
  equip it again later, or equip one of your in-game loadouts.
- **Equipped**: what the character has on right now.

The game only lets gear be equipped in orbit, a social space, or when you're
offline. In an activity, items still move to your character but stay
unequipped.

## How it picks builds

**Power** is the average of your 3 weapons and 5 armor pieces (the cap is 550).
Higher Power means you deal more damage and take less.

**Stats** (Armor 3.0) go up to 200. Points up to 100 make abilities recharge
faster; points past 100 add a second effect:

| Stat    | 0–100                                   | 101–200                                 |
| ------- | --------------------------------------- | --------------------------------------- |
| Weapons | Reload, handling, damage vs. enemies    | More ammo from bricks, more boss damage |
| Health  | Orb of Power healing, flinch resistance | Stronger shields that recharge faster   |
| Class   | Faster class ability                    | Overshield every time you use it        |
| Grenade | Faster grenade                          | More grenade damage                     |
| Super   | Faster Super                            | More Super damage                       |
| Melee   | Faster melee                            | More melee damage, including Glaives    |

The **Solo dungeon** preset asks for Health 100+ and Class 100+, the two stats
that keep you alive when nobody can revive you, then weighs the rest. **Solo
boss damage** leans on Weapons and Super for damage phases.

A build's score is the sum of each stat total times its weight. Points above
the **stat cap** (200) don't count, so stats aren't wasted. The search checks
every combination of your best candidates in each slot (armor that another
piece beats in every stat is skipped), never pairs two exotics, honours a
required set bonus, drops builds that miss a minimum, and breaks ties by
Power. _Search: Deep_ looks at more candidates per slot.

Stats are the numbers Bungie reports for each piece as it is now, so stat mods
already on a piece are counted.

Sources used for the stat and Power rules: Bungie's
[Armor 3.0 preview](https://www.bungie.net/7/en/News/Article/dev_insights_abilities_armor_preview),
[PC Gamer](https://www.pcgamer.com/games/fps/destiny-2-armour-3-0-stats-archetypes/),
[GamesRadar](https://www.gamesradar.com/games/destiny/destiny-2-armor-3-0/),
[TheGamer](https://www.thegamer.com/destiny-2-edge-of-fate-armor-rework-guide/),
[Sportskeeda](https://www.sportskeeda.com/mmo/destiny-2-returning-player-guide-2026-power-cap-activities-gear-tier-start)
and [light.gg set bonuses](https://www.light.gg/armor/set-bonuses/).

## For development

```bash
npm install
```

```bash
npm start
```

`npm test` runs the tests, `npm run dist:win` builds the Windows installer
(on Windows, or anywhere with Wine), and `npm run web` serves the same app in
a browser at `https://localhost:55555/` instead of as a desktop app.
