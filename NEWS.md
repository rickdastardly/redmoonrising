# Red Moon Rising patch notes

Every update, as published on the server dashboard and the character selection screen. The server adds these to its news when it starts.

## Bots that play: roaming gangs, wingmen, mining fleets and traders

*2026-11-12 15:35 UTC*

### Roaming gangs

- Pirate and null-sec bot alliances now send **gangs of 5 to 12 fighters roaming** low and null-sec for about two hours, staying 8 to 18 minutes in each system before moving on, 3 to 6 systems in all.
- With hunting on, they head for **systems where players are in space**. They attack players outside high-sec on sight and leave high-sec alone, and stragglers regroup with their gang.
- Gangs show on the dashboard's Events, raids and gangs page beside raids, with what they killed and lost, and in the news when they set out and go home. Start one yourself with the **Roaming gang** button or `/bots event roam`.

### Wingmen

- Invite an armed bot ship in your system to your gang and it flies with you, follows you through gates and fights what you fight. You can now choose whether bots join **for free**, for a quarter of their rate, at their usual hourly rate, or not at all.

### Mining fleets

- Miners from the same station now **mine together** in the same belt, each on its own asteroid, instead of spreading out.
- Where you can see them, bot miners **mine the asteroids down**, with mining beams. Rocks shrink and vanish when they are mined out, and the miners move on to the next, so busy belts get cleared.

### Bot traders

- Trader bots now keep **sell and buy orders** at the ten busiest hubs and at any station where a player is docked, for everything the NPC market there sells: ships, modules, ammunition, drones, implants, deployables, minerals and ore.
- Their prices sit around the market's, a little over or under, and buy orders bid below it. Each order stands for one to three days, and new ones go up every five minutes or so. Buy from them and sell to them like any player's order.

### Dashboard

- The **Bots** page has new **Gangs and fleets** and **Mining and the market** sections: how often gangs roam, whether they hunt players, what wingmen cost, mining fleets, belt clearing, and how busy the bot traders are. Changes take effect at once and are saved to rmr.yml.

## Server health on the Overview

*2026-11-12 15:08 UTC*

### Dashboard

- The **Overview** page now shows the **Server Health** window (CPU, memory, game loop and bot load, database and the live charts) below server status, updating every two seconds. It is still on the Server status page too.

## Killboard portraits and logos, GM command list, a redder moon

*2026-11-12 02:46 UTC*

### Killboard

- **Pilot portraits.** Pilot pages, killmails, final blow, top damage and every attacker now show the pilot's real portrait, taken from your game client's portrait cache. A pilot your client has never seen keeps their initial until it has.
- **Alliance logos.** Alliances that uploaded a logo in game show it on the killboard and their alliance page.
- **NPC corporation and faction logos** come from CCP's image server. Player corporations keep their coloured badge.

### GM commands

- A new **GM commands** page under Administration lists every slash command, what it does and an example, grouped and searchable. It also says which account roles can use them.

### Look

- A red moon rises behind the masthead, with crimson accents on page headings, window corners, the masthead edge and the background nebula.

## Sovereignty map the right way up, and dashboard polish

*2026-11-12 02:35 UTC*

### Fixed

- **The sovereignty map is the right way up.** Northern regions such as Venal, Tribute and Vale of the Silent are at the top again, and the south at the bottom. A small compass in the corner shows north.

### Sovereignty map

- **Find a system:** type a name in the map's toolbar (or press `/`). The map flies there and marks the system with a pulsing ring.
- Zoom buttons, plus keyboard shortcuts: `+` and `-` zoom, `R` resets the view, `Esc` clears the alliance focus.
- Your Stargates, Regions, System names and Empire space choices are remembered between visits.
- A faint starfield sits behind the map and drifts as you pan.

### Dashboard

- Client-style thin scrollbars, a lit edge along the top of every window, and corner brackets that brighten on hover.
- Windows slide in gently when you change page. This is skipped if your system asks for reduced motion.
- Buttons glow when you hover over them.

## Code review: Local roster and /maxstandings

*2026-11-12 02:28 UTC*

### Fixes

- **Local member list:** the client hook that keeps Local's pilot list and war-target badges up to date no longer errors on every refresh. Local now refreshes its list and badges as intended.
- **/maxstandings** now updates your Standings window live for every NPC corporation and faction. Before, you had to relog to see the new standings.

### Checked

- Starbases (POS), warp disruptors and interdiction bubbles, deployables and pirate NPCs from the earlier merge are all still in place.

## Directional scanner: V and click

*2026-11-12 02:13 UTC*

### Directional scanner

- Press `V`, then **click anywhere in space** (or on any object): your camera turns that way and the directional scanner scans along it, out to **14.1 AU**. This now works whether the scanner window is open or closed; before, with the window open, V ignored your click.
- After pressing `V` a message reminds you to click. You have 30 seconds to pick a direction.
- Press `V` twice to scan straight along where your camera already points.
- Ships **in warp** now show on the scanner, where they are at that moment.
- The shortcut is under **Shortcuts** in the Escape menu, where you can change the key.

## Escrow fixed, and bots trade in it

*2026-11-12 02:06 UTC*

### Escrow

- **Making an escrow works again.** After you picked a station, the Escrow tab sat on "Loading..." forever instead of listing your items.
- An escrow or courier mission with an **empty description** no longer fails when you submit it.

### Bots trade in escrow

- **Bot traders put up escrows** at the busiest trade hubs and at any station where a player is docked: bundles of ships, modules, drones, ammunition and materials from that market, usually a little under or over market price. Look in **Browse Escrow**.
- **Bots buy your escrows** when the price is fair for what is inside. A bargain sells within minutes, a fair price in a while, an expensive one rarely, and a rip-off never. You are paid straight away and told who bought it.
- Bots take down their own escrows that nobody buys before they expire.

## Patch notes for everyone

*2026-11-12 01:50 UTC*

### Patch notes for every server

- **Every server now gets these news articles.** They ship with the server in `news/articles.json`, and a server adds any it is missing when it starts, so a fresh download shows the full history of patch notes on the dashboard and the character selection screen.
- Articles published from the dashboard's **News publishing** page are saved to that file as well, so they travel with the next update.
- The same notes are on GitHub in **NEWS.md**, newest first, so anyone can see what each update changed.
- News is now listed by publish date, so older notes added later sit in their proper place.

## Launcher build fix

*2026-11-12 01:46 UTC*

### Launcher

- Fixed a launcher bug from the previous update that stopped the server from building, with `ld: cannot find crt2.o` and many `cannot find -lkernel32` errors.
- The fix for first-time setup on PCs without MinGW (`dlltool.exe` not found) still applies.

## Killmails show your carrier, not your drones

*2026-11-12 01:40 UTC*

### Killmails

- **Drone kills show your ship again.** When your drones or fighters did the damage, killmails listed the drone (an Ogre II, a Templar) as your ship. They now show the carrier, supercarrier or whatever you were flying, with the drone as the **weapon**, as on EVE's own killmails.
- Bot ships with drones are recorded the same way.
- Older killmails are corrected after the restart: the drone becomes the weapon, and the ship is the one you flew in your nearest other kill, or the ship you fly now.
- Pilot, corporation and alliance pages count those kills under the right ship in **Top ships**.

## First-time setup fix for new servers

*2026-11-12 01:33 UTC*

### Launcher

- **First-time setup no longer fails with `error calling dlltool 'dlltool.exe': program not found`.** On PCs without MinGW installed, the build stopped at `getrandom`, `windows-sys`, `chrono` and `parking_lot_core`.
- Start.bat now uses the `dlltool.exe` that comes with its own portable Rust compiler, and reinstalls that part of the compiler if it is missing.

## Corporation and alliance pages on the killboard

*2026-11-12 01:30 UTC*

### Corporation and alliance pages

- **Corporations and alliances are now clickable** across the killboard: the kill list, kill reports (victim and every pilot involved), pilot pages, the top corporations and alliances, and the Alliances and Corporations pages. Their badges link too.
- Each opens a **zKillboard-style page**: name and ticker, CEO, alliance, members and outposts for a corporation; executor, member corporations, members, systems held and founding date for an alliance.
- The same stats as a pilot: **ships and ISK destroyed and lost** with efficiency, **Dangerous / Snuggly** and **gang / solo** bars.
- **Overview**, **Kills** and **Losses** tabs, their six most valuable kills, and the full list page by page. Their kills are green and their losses red.
- Sidebar: when they fight, the last 7 days (pilots, ships, systems, regions, kills), top pilots, top ships, top systems, and an alliance's member corporations.

## Faster drone recall, bot fittings on killmails and new pilot pages

*2026-11-12 01:24 UTC*

### Drones

- **Return to Drone Bay is much faster.** Fast drones like Warrior IIs used to crawl through their last few kilometres and could take half a minute to dock. Now they dock as soon as they reach your ship.
- Pressing **Return to Drone Bay** (or `Shift+R`) again on drones that are already home no longer gives a "could not accept the command" message.

### Killboard

- **Bot killmails now show their fitting.** High, medium and low slot modules sit on the fitting wheel and in their slot sections, drones in the drone bay, and only charges and real cargo under Cargo.
- Bot killmails list what **dropped** (green) as well as what was destroyed, so the value split is right. Older bot killmails are sorted into their slots automatically after the restart.

### Pilot pages, zKillboard style

- A new layout: portrait, corporation, alliance, security status and birthday, beside a table of **ships and ISK destroyed and lost** with efficiency.
- **Dangerous / Snuggly** and **gang / solo** bars, plus final blows and solo kills.
- **Overview**, **Kills** and **Losses** tabs, with the pilot's six most valuable kills on top and the full list underneath, page by page. Kills are green and losses red.
- Sidebar: when the pilot fights over the last 90 days, the last 7 days' activity, top ships, top systems and who they fly with.

### Local

- Fixed another error that stopped **War Target** icons showing in Local.

## Player outposts, Station Management and smoother big fights

*2026-11-12 01:08 UTC*

### Build your own outpost

- Launch an **outpost construction platform** (like the Amarr Factory Outpost Platform) and anchor it in null-sec space your alliance holds. Only one outpost per system.
- Anchoring shows a **countdown** when it starts, reminders as it runs, and a message when it finishes.
- Fill its **resources hold** with the materials listed under Show Info, then press **Build**. The platform becomes a dockable station owned by your corporation, with factory and research lines, and leftovers go into your station hangar.
- Inside, the hangar matches the outpost's race: Amarr, Caldari, Gallente or Minmatar.
- GMs: the platform's **GM / WM Extras** menu shows the anchoring time left, with **Rush anchoring** and **Spawn outpost materials** (also `/rushanchor` and `/outpostmats`).

### Station Management

- The **Station Management** window now opens properly for stations your corporation owns. You need the Station Manager or Director role.
- **Station Details:** rename the station, write a description, and set the docking cost, office rental cost and reprocessing tax.
- **Service Access Control:** set the minimum standing and security status for each service. The **Docking** rule decides who may dock: your own corporation and alliance always can, and anyone else must meet your standing and security limits.
- The **Offices** and **Clone Contracts** tabs list who rents offices and who has clones at your station. **Revoke** sends their clones back to their corporation's headquarters.
- A new station name shows after you log in again.

### Science and Industry

- Outpost platforms now show their **Material Requirements**.
- GMs, BH and ISD BH get a **Rush Job** button above Cancel Job.

### Space and Local

- Fixed big fights making your ship **snap back** to an earlier spot, and the overview filling up with **duplicate rows** and stale distances.
- War targets in **Local** now get the war target icon, the same as on your overview.

### Dashboard

- A new look, closer to an EVE admin console.
- The **Killboard** now looks like zKillboard: big kills, ship and structure losses, recent activity and top pilots, corporations and alliances.
- A live **Sovereignty map** shows who holds which space, updated as systems change hands.
- **Market seeding:** pick any station (your new outposts are listed first) and stock its market with NPC sell orders for every item. Seeded stations are topped up at every restart.

## Real ships, proper fits, drones and fight calls

*2026-11-12 00:43 UTC*

### Bot ships

- Bots now only fly hulls sold under the market's **Ships** tab. About 52,000 bots that were flying something else have moved to proper market hulls.

### Proper fits

- Guns of the hull's race and size in its turret hardpoints, and **missile launchers** with missiles in its launcher hardpoints. Caldari missile boats finally shoot missiles.
- Afterburners or microwarpdrives, shield boosters and extenders or armor repairers, plates and hardeners, damage controls and damage mods. Pirates fit **warp scramblers and webs**.
- Miners fit strip miners or mining lasers and mining upgrades; explorers carry a probe launcher and probes.
- Every fit stays within the hull's **CPU and powergrid**, and richer bots fit named modules.

### Drones

- Hulls with a drone bay carry **combat drones** of the right size, preferring their own race's. Miners carry mining drones.
- Bots **launch their drones** when they engage, switch them to new targets, and recall them when the fight ends.

### Fight calls in Local

- Call a fight in Local, like `meet at top belt to fight`, `1v1 me at belt 3`, `fight at IV-2`, `last belt pvp`, `fight at the sun` or `come fight me here`.
- Some bots say they're coming and **warp there**, ready to fight. Pirates come most often, miners almost never, and others decline or laugh. With bots set to **calm**, nobody comes.

## Bots that talk back, alliance applications and server health

*2026-11-12 00:25 UTC*

### Local chat

- Bots now have **personalities** that fit what they do: friendly pilots, grizzled veterans, excited newbies, trolls, gloating pirates, nervous miners, market spammers, roleplayers and grumps.
- **Talk to them.** Say hi, ask how they are, what they fly, how to make isk, or call someone out. A bot answers after a moment of typing, and keeps talking to you for a few minutes. Use a bot's name to talk to that bot.
- Bots chat among themselves, start conversations, troll Local and shout during fights nearby.
- After a kill, the winner and loser have their say: `gf`, gloating, rage or excuses, and onlookers chime in with `o7` or `F`.

### Corporations and alliances

- **Bot alliances now answer your corporation's applications.** Accepted memberships start a day later, as in EVE. Bot corporations answer yours with real replies.
- Bot corporations now apply to **your alliances**, and bots' applications to your corporations describe what they actually do.

### Dashboard: Bots page

- New settings that change the bots at once: **Local chat** (off to chatty), **trash talk**, how often bots apply to your corporations and alliances, how bot corps answer applications, pirates attacking on sight, **online activity** (0.1x to 4x) and ship limit per system.
- Switching to **All bots calm** now makes bots chasing you let go, unless you shot them first.
- The bot limit is now **1,000,000**.

### Dashboard: Server health

- The **Server status** page shows machine and server CPU, memory, game loop load, bot simulation load and database speed, with charts of the last five minutes.

## Dashboard restart now matches the launcher

*2026-11-11 23:58 UTC*

### Dashboard restart

- **Restart now** and **Restart in 5 minutes** on the dashboard now click the launcher window's own **Restart server** button. The whole launcher restarts: it saves the universe, stops the database, rebuilds any changed code and starts everything again.

## Bigger news text and more formatting

*2026-11-11 23:49 UTC*

### Easier to read

- News text on the character selection screen is **two sizes larger**.
- Articles can change text size and colour: `<font size="5">` (sizes 1 to 7, normal is 4), `<big>`, `<font color="#ff8800">`, plus `<h1>` headings and `<center>`.
- The dashboard's publishing form has **Bigger**, **Smaller**, **Colour** and **Centre** buttons for these.

## Login news and dashboard restarts fixed

*2026-11-11 23:43 UTC*

### News on the login screen

- The character selection news panel now shows each article with a **gold title**, its date, **blue section headings**, bullet lists and `yellow commands`.
- Apostrophes in titles show correctly.

### Dashboard restarts

- **Restart now** on the dashboard presses the launcher window's own **Restart server** button, so it always rebuilds and restarts exactly as that button does, even in a launcher window opened before this update.

## Heavy drones, shortcuts and bot controls

*2026-11-11 23:21 UTC*

### Drones

- **Heavy drones fire again.** Ogres, Praetors, Berserkers and the rest lock their target in about five seconds and engage out to 5 km, like light and medium drones.
- Drone Control Units raise your drone limit while they are **active**.

### Shortcuts

- `F` engages your drones on your target, `Shift+R` returns them to the drone bay and `Ctrl+B` bookmarks your current location.
- With the directional scanner open, `V` scans again where your camera faces and refreshes the results.
- All four appear in the **Shortcuts** tab of the Escape menu, where you can change them.
- Typing in chat or any text field still types those letters.

### Server administration

- **Restart now** on the dashboard does exactly what the launcher's **Restart server** button does: it saves, stops the database, rebuilds if needed and starts again.
- A new **Bots** page sets how many bots the server plays (saved for the next restart, or **Save and restart now**).
- Bot aggression changes at once: **All bots calm**, **Some bots aggressive** (pirates outside high-sec, angered sides and their hunters) or **All bots aggressive**. Bots always fight back when shot.

### News

- Articles can now have headings, bold and italic text, lists, links, dividers and `yellow commands`, like these notes.
- The dashboard's publishing form has formatting buttons and a live preview.
