# Red Moon Rising patch notes

Every update, as published on the server dashboard and the character selection screen. The server adds these to its news when it starts.

## Capital battles in open space, anywhere

*2026-11-13 20:19 UTC*

- **A capital battle can now be started in any system.** Where no bot alliance holds a tower, two bot alliances meet in open space instead: a *Battlefield* beacon appears about 45 km from you and both fleets form up round it. Before, the request was turned down.
- **Dreadnoughts fight each other.** In siege mode, with no tower to bombard, they fire heavy volleys at the enemy's capitals first, then at the rest of the enemy fleet. Round a tower, the attackers still bombard it.
- An open-space battle decides no sovereignty: the news says who won, and the beacon goes when it ends.
- An open-space battle forms up wherever a pilot is in space in that system; with nobody in space there it is fought on paper until someone undocks.

## Capitals arrive at a cyno with the jump drive's flash

*2026-11-13 18:58 UTC*

- **Capitals jumping to a cynosural field, and ships coming through a titan's bridge, now arrive with the jump drive's own effect** (the bright flash with streaks trailing from it) instead of the stargate flash.
- This one is on trial: if you see no flash when the capitals appear, or the server log shows a client error about it, tell the server's owner so it can be put right.

## Start a capital battle from the dashboard

*2026-11-13 18:42 UTC*

- **The Bots page has a Capital battle window.** Type a system (or leave it blank for anywhere a bot alliance holds a tower), pick Decisive, Battle or Skirmish, and press **Start capital battle**. It does the same as `/bots battle` in game, and the window is also on the Events page.
- The GM commands page lists `/bots battle`, with an example.
- Like the other bot controls, it needs the dashboard's bot controls switched on.

## Capital battles on request, and a clearer log of them

*2026-11-13 18:37 UTC*

- **Start a capital battle whenever you like.** A GM can type `/bots battle` to start a decisive siege at once in a bot alliance's space, or `/bots battle here` for the system they are in (it needs a bot alliance's claiming tower there), or name a system. Add `skirmish`, `battle` or `decisive` to choose how much each side brings. Both sides bring every kind of capital ship: at least one titan, dreadnoughts, carriers, logistics and a cyno ship.
- **Watch it from the defenders' tower.** A cyno ship lights a cynosural field, the capitals jump to it, the titan bridges the rest of the fleet in, dreadnoughts go into siege mode and bombard the tower, logistics repair their side, and titans fire their doomsday.
- **The server log follows every step:** each cyno lit, each wave of capitals jumping in, every titan bridge, each dreadnought entering siege, and every 30 seconds what is on the grid with the tower's shield, armour and structure. Capitals left waiting with nobody to light a cyno say so too.
- A server restart in the middle of a battle no longer leaves the bots who flew capitals stuck in them: they go back to their own ships first.

## Sell window and reprocessing plant fixed

*2026-11-13 18:20 UTC*

- **The sell window opens again.** Some bot buy orders reached a number of jumps the game's market window has no name for, and that stopped the sell window from drawing. Bot buy orders now reach their station, system, constellation or the whole region, the four ranges the game knows, and existing orders were moved to the nearest of those.
- **Reprocess works in the reprocessing plant.** Pressing Reprocess in the plant window now shows the quote for each item and reprocesses it, like reprocessing an item on its own already did.

## Splitting stacks works; the sell and reprocessing windows report what goes wrong

*2026-11-13 18:13 UTC*

- **Splitting a stack works again:** hold Shift and drag a stack into the window it's already in, then enter how many to split off. You get a new stack of that many beside the old one. Before, the server treated it as a move to where the stack already was and did nothing.
- **Moving part of a stack** no longer confuses the item's own location with a quantity.
- **Sell window and Reprocess:** on the server's side these work. The sell window gets its prices and orders without error. When you click Reprocess, nothing even reaches the server. So the fault is inside the game client, which fails without logging anything. The client now reports every error inside the market and reprocessing windows and services, anything they wait on for more than eight seconds, and the steps the windows take, to the server log as *client hook: trace*. Try them once more after restarting, and the cause will be in the log for the next fix.

## Bot carriers launch fighters

*2026-11-13 18:02 UTC*

- **Bot carriers now fight with real fighters** instead of guns. Archons launch Templars, Chimeras launch Dragonflies, Nidhoggurs launch Firbolgs and Thanatoses launch Einherjis.
- **How many:** ten fighters per carrier, as a Carrier V pilot flew in 2007, and fifteen per mothership.
- **How they fight:** the carrier picks a target up to 100 km away and holds back about 45 km from it. Its fighters fly out to it, with each fighter's own stats, speed and damage from the game data.
- **You can fight them:** you can shoot fighters down, and a titan's doomsday wipes them out. They go back to the carrier when the fight is over and leave with it.
- **Wrecks:** a carrier's wreck and killmail now show the fighters in its bay.

## Capital warfare: cynos, dreadnoughts, carriers, logistics, titans and battle reports

*2026-11-13 17:48 UTC*

- **The bot alliances now bring capitals to their sovereignty battles.** Each side brings dreadnoughts and carriers, plus titans in the biggest fights. How many depends on the fleet's size, what is at stake and how rich the alliance is. Their richest pilots fly the capitals, and any who survive go back to their own ships afterwards.
- **Cynos:** a force recon (Pilgrim, Falcon, Arazu or Rapier) warps in with the fleet and lights a *Cynosural Field I* you can see on the grid and the overview. The capitals then jump in on it with the jump flash. If the cyno ship doesn't make it, any ship of the side lights one after 45 seconds. The cyno goes out when the fight ends or moves out of sight.
- **Titan bridges:** once a side's titan is on the grid, the rest of its fleet comes through the titan's jump portal around it, instead of warping in.
- **Dread bombing:** dreadnoughts go into siege mode. The attackers' dreads bombard the defenders' control tower for about 9,000 damage every 5 seconds, so the bots now bash each other's POSes to take sov.
- **Logistics:** Guardians, Oneiros, Basilisks and Scimitars repair their side's most damaged ship, with shield transfers or remote armour repair by race.
- **Titan battles:** titans fire their doomsday 90 seconds into a fight, then every 10 minutes. It hits every enemy subcapital within 250 km. Capitals are immune, as in 2007.
- **All of this happens in real time, whether anyone is logged in or not.** A battle nobody watches is still fought, second by second:

- Capitals count for many ships and are hard to kill.
- Logistics cut their side's losses by up to a third.
- Each titan's doomsday takes out a share of the enemy fleet.
Every ship lost, capitals included, is a real killmail on the killboard. Fly to the system and the fight comes into view, ship against ship.
- **Battle reports on the dashboard killboard.** Every sov battle gets a report at the top of the Killboard page, marked LIVE while it is fought. It updates every minute and shows:

- Both sides and their allies, and the fleet each brought.
- The capitals each side brought and lost.
- Ships and ISK lost, broken down by ship type from the killmails.
- How the battle ended.

- **Bigger battles:** the dashboard's *Bot ships shown per system* now goes up to 5,000 (it was 2,500). Set it to 4,000 or more, with battle size at 2,000, for 2,000 against 2,000 fights on one grid. Expect a lot of work for the server and your client at that size.
- **Note:** this is bot behaviour only. Players' own cyno generators, jump drives and titan bridges aren't in yet.

## Faction and officer gear back on the market; unreleased items gone

*2026-11-13 17:24 UTC*

- **Faction, officer and deadspace gear is back on the main market**, beside the ordinary versions: Caldari Navy, Shadow Serpentis, Gist, Pith, Dread Guristas, officer modules and the pirate faction ships. They were all in the game in 2007 from DED complexes and LP stores. The last update wrongly moved them to Additional Items.
- **Only unreleased items are removed**, unlisted and never stocked again (about 380 in all):

- Anything named "test", such as *test hardener*, *TEST Smartbomb* and the *Uber Test* towers. Real items with "test" in the name stay: *Test Dummies*, *Serpentis Test Reports* and *'Protest' Large Armor Repairer I*.
- The NPC *Boss Rank* placeholders for Serpentis, Sansha and Blood Raider hardeners and membranes.
- Developer copies and tools: *Dummy*, *UNUSED* and *OLD* items, the *GM* modules and the *Pimp* fittings.
- NPC and placeholder hulls: CONCORD, Polaris and police ships, Jove ships, *Elite* and *Tier3* test hulls, *Atron much HP* and the like. Odd drones such as the *lewt drone* are gone too.
- Skills that were never put into play, such as the Jove ship skills, *Omnipotent*, *Smuggling* and *Black Market Trading*.

- Everything else stays as it was. The real market tree is kept exactly as the 2007 data has it, and the remaining extras are still in **Additional Items**. Anything already in someone's hangar stays there.

*Takes effect when Start.bat restarts the server.*

## The market tab is Red Moon Rising's again

*2026-11-13 17:10 UTC*

- **The main market tab now lists exactly what it did in 2007.** To fill gaps, the server had been slotting items the 2007 market never sold in beside their ordinary versions: faction, officer and deadspace modules, pirate faction ships and unreleased placeholders. Every item now sits in the market group the 2007 data gives it, and nothing else goes in the main tab.
- Items the 2007 market never listed go to the **Additional Items** group instead, so the extras are still there to find, kept apart from the real market.
- Developer test items and the NPC *Boss Rank* placeholders stay off the market completely, as in the previous note.
- Player orders aren't touched. Items you own stay as they are.

*Takes effect when Start.bat restarts the server.*

## Train skills straight from the skill book

*2026-11-13 17:08 UTC*

- **Fixed:** **Train Skill** on a skill book in your hangar or cargo said *That is not one of your skills*, even when you met every requirement. Training only worked on skills you already knew, so a new skill could never be learned without a GM command.
- Now **Train Skill** on a book teaches you the skill and starts training it at once, from level 0. One book is used up; the rest of a stack stays where it is.
- If you already know the skill, its training starts and the book is left alone.
- If you're missing a prerequisite, the message names the skill and level you need, and the book isn't used up.

*Takes effect when Start.bat restarts the server.*

## Developer test items taken off the market

*2026-11-13 17:07 UTC*

- **Test items are gone from the market.** Everything CCP made for testing is no longer published, listed or sold, and is never seeded again: *TEST Smartbomb*, the *Uber Test* control towers, *New EW Test* modules, *Gfx_turret_test*, *Skunks Cruise Test* and the rest. Any you already own stay in your hangar.
- **The "Boss Rank" items are gone too.** Items such as *Serpentis Bship Boss Rank1 Armor EM Hardener* were never Shadow Serpentis gear. They're unreleased placeholders CCP used to set NPC boss stats. The real faction modules, such as *Shadow Serpentis Armor EM Hardener*, stay on the market as before.
- Leftover developer modules named *Dummy* or *UNUSED* are also gone.
- Real items that just have "test" in the name stay: *Test Dummies*, *Serpentis Test Reports* and *'Protest' Large Armor Repairer I*.
- The rest of the **Additional Items** market group stays, including the GM modules: real items from the game's data that had no market group in 2007.

*Takes effect when Start.bat restarts the server.*

## Turrets you can see and hear, fairer rats, docking and agent list fixes

*2026-11-13 17:01 UTC*

- **Your turrets show and fire again.** The server never told the client what was in your ship's high slots, so it mounted no turrets or launchers on the hull, and firing showed no shots and made no sound. Your high-slot modules are now sent with your ship, as they were in 2007, for you and for everyone who sees you.
- **Rats keep shooting while you align.** They stopped the moment you clicked warp. They now keep firing until you're actually in warp, and you can still be shot while lining up to leave.
- **Rats take a moment to notice you.** Belt rats locked you the instant you landed. Now they take 5 to 12 seconds to react to a pilot who hasn't fired on them. Shoot first and they answer at once.
- **Docking goes where you ask.** Asking to dock while your ship was still lining up for another warp made it finish that warp first, so you flew back to wherever you'd been heading and couldn't dock. A dock request now turns the ship toward the station straight away. A warp you're already in still finishes first, since nothing can stop a warp.
- **People & Places: mission places sorted properly.** Two missions with the same name, such as two *Sealed Documents* from different agents, each listed both missions' drop-offs and pickups. Missions sharing a name now show their agent's name too, and each lists only its own places.
- **People & Places: no more freezing on the Agents tab.** The few thousand agents who'll work with you were kept in your chat contact list, which the client reads through every time it checks whether someone is a contact (Local, mail, People & Places). Now the server sends that list only when you open the **Level N agents available to you** groups, and the contact list holds only your real contacts and mission agents.

*Restart Start.bat to rebuild the server, and start the game from Client.bat.*

## Fewer client errors: bot miners, market in space, jumps and undocking

*2026-11-13 16:52 UTC*

- **Bot miners:** bots mining asteroids caused *AttributeError: StartMining* and *DroneMining: no ball found* errors whenever you were in a belt with them. Their mining was drawn with the mining drone effect, which the client looks for on a drone and can't find on a ship. They now use the mining laser effect, as a player's ship does.
- **Market in space:** using the market window after undocking failed with *Must be in station*. It now shows no station orders while you're in space, instead of failing.
- **Jumping:** clicking Jump again while a jump was already under way raised the *sensors are still tuning in on subspace frequencies* error. The extra click is now ignored, since the first jump goes ahead.
- **Jumping, docking and undocking:** the client checked your drone bay and cargo hold while it was still switching location, which logged *session is mutating or changing* warnings. Those checks now wait until you've arrived.
- The star map's harmless Python notices (*hex() of negative int*, *integer argument expected*) are no longer written to the log as stack traces.
- Warping, docking and turrets were all working in the log: every warp landed, every dock and undock completed, and the hybrid turrets killed their targets. The errors around them were the ones above.

*Restart Start.bat to rebuild the server, and start the game from Client.bat.*

## Update roundup: everything new since 12 November

*2026-11-13 15:22 UTC*

Everything new since the sovereignty warfare update, in one place. Each change also has its own note below this one.

### Launcher

- **Three launchers, one job each.** Start.bat runs the server, **Client.bat** sets up and starts the game, and **Dashboard.bat** puts the dashboard behind a sign-in. Five wrong passwords lock an address out for 15 minutes.
- **Client.bat is a new EVE-style launcher**, with your servers and their online lights, the chosen server's news, and a big PLAY button. It can also change or repair your EVE client.
- Dashboard.bat keeps nothing between runs. The sign-in lives only in memory, and its firewall opening closes when you stop it.
- Start.bat builds on PCs without MinGW, since the tools it needs now come with the download. It keeps your PC responsive by building and running the server at a lower priority.

### Accounts and setup

- **Passwords are stored hashed** (salted PBKDF2-SHA256). A password typed into `rmr.yml` is hashed at the next start.
- `rmr.yml` is now each server's own file. A new server starts with **100,000 bots**, accounts made at first login, and the dashboard's bot controls on. An old file you never edited is brought up to these defaults; one you edited is left alone.

### Skills and items

- **Skills train**, online or off, at a speed set by your attributes, implants and learning skills. Skills and modules need their prerequisites, and new pilots get the prerequisites of their starting skills.
- A clone bought as an item is used up when installed.
- **Show Info shows the real values:** 680 negative numbers had been stored wrongly (Gunnery's -2% showed as 254%). Skills train, check prerequisites and give bonuses exactly as Show Info says.

### Market

- **Selling into buy orders works wherever the order reaches.** Bot buy orders now cover their whole range, and bots put up bids for items in your hangar that nobody nearby is buying. Order ranges count real jumps.

### Bots, chat and corporations

- In high-sec, bots fight only their war targets, or someone who shot first. CONCORD handles the rest.
- Corp mates show a green star and alliance mates a blue one in Local and in corp and alliance chat. Those channels list their members, and bots in your corporation chat among themselves.
- Executor corporations can pay alliance bills from the corp wallet. Offices get a rental bill five days before the rent runs out.
- The dashboard Overview uses wide screens, and bot applications have an **Extra high** setting.

### Agents and mail

- **People & Places** no longer freezes. Your contacts and mission agents are in the normal list, and every other agent who'll work with you is under **Level N agents available to you**, loaded one small group at a time.
- The inbox loads your newest 250 mails.

### Fixes from a player's server log

- The market no longer fails on some items: attributes this client doesn't know are no longer sent.
- The Channels window opens again: tutorials with no pages are no longer offered.
- Stargate jumps no longer cause a *No ballpark for update* error, and the System Scanner waits until a jump finishes.
- Guards for the chat star and for the character selection screen's tables, and quieter server logs.

*Restart Start.bat to rebuild the server, and start the game from Client.bat.*

## Fixes from a player's server log: market, chat window, jumps and more

*2026-11-13 15:17 UTC*

Fixes for the errors in a player's server log:

- **Market:** opening some market groups failed with an error. The item data gave 211 values for seven attributes (1112 to 1126) that this game client has never heard of, on implants among others. They're no longer sent, so the market's skill check and Show Info work on those items again.
- **Channels window:** opening it failed with an error. The game tried to show its "Creating a chat channel" tutorial, which has no pages in the data, and 19 other tutorials are the same. The server no longer offers the empty ones, the tutorial chain stops before them, and the client skips one if asked for it anyway.
- **Jumping through a stargate:** the gate flash and jump-out effect sometimes reached your client after you had arrived in the next system, causing a *No ballpark for update* error. Your own copy is now sent before the jump.
- **System Scanner:** it refreshed while you were jumping and tripped the client's session check. It now waits until you've arrived.
- **Local and chat member lists:** fixed an error in the war/corp/alliance star when a row closed while it was loading.
- **Character selection:** the game's HTML tables could fail with *KeyError 'spacing-right'* when a line was too wide for its column. The client now keeps the layout it has instead of failing.
- **Server log:** Client.bat checks every few seconds whether each server is online, and the server logged each check as a lost connection: 157 warnings in one evening. Those checks are now logged only at debug level. Real players connecting, disconnecting and dropping are logged as before.

*Restart Start.bat to rebuild the server. Start the game from Client.bat to pick up the client fixes.*

## Item and skill attributes show their real values

*2026-11-13 15:02 UTC*

- **Negative numbers in Show Info are right again.** The item data the game reads had small negative values stored as if they could not go below zero, so Gunnery's *-2%* turret speed bonus showed as *254%*, and a few large ones (such as -20,000) showed as 18-digit numbers. All 680 affected values, on skills, modules, ships and charges, now show and work as intended.
- **Skills work the way the game shows them.** Where the server's copy of a skill differed from what Show Info says (training rank, prerequisites, bonuses), the server now uses what you see. Training times match the skill's listed rank, and no skill asks for a prerequisite its info sheet does not list.

## Skill training, skill prerequisites, fitting requirements and clones

*2026-11-13 14:53 UTC*

### Skill training

- **Skills train.** Starting a skill now begins real training: it finishes at the time shown, whether you are logged in or not, and the skill goes up a level. Starting a different skill, or stopping, keeps the skill points earned so far.
- A skill whose prerequisites you do not have cannot be started; the message says which skills and levels you need.
- Training speed comes from your attributes: the skill's primary attribute plus half its secondary, with implants and learning skills counted.

### Prerequisites

- **New pilots get the skills their starting skills need**, such as Gunnery for a turret skill.
- Pilots made before this get any prerequisite skills they are missing the next time the server starts.

### Fitting

- **Modules need their skills.** A module cannot be fitted unless you have its required skills; the message names the ones you are missing.

### Clones

- **A clone bought as an item is used up** when it becomes your clone, instead of staying in your hangar. Only the clone you have installed counts for the skill points you keep.

## Selling to buy orders, bot buy orders, account creation and a calmer Start.bat

*2026-11-13 14:44 UTC*

### Market

- **Selling into buy orders works where you are.** The bots' buy orders only took items at their own station, and they were only placed at the busiest trade hubs, so the market showed buy orders you could not sell into. Bot buy orders now reach the whole region, a number of jumps, the system or the station, like players' orders.
- **Bots bid on what you have.** Where you are docked, bot traders put up buy orders for the items in your station hangar that nobody nearby is buying: a fair bit under the NPC price, or under the base price for loot the NPCs do not sell. Blueprints are left out. Give it a few minutes after docking.
- Order ranges are now counted in real jumps through the gates, and the market window shows how many jumps each order is from you.

### Accounts

- **New names get an account at their first login again.** A server whose `rmr.yml` was written by an earlier version and never edited had account creation, bots and the dashboard bot controls switched off. That file is now brought up to today's defaults when the server starts: accounts made at first login, 100,000 bots and the Bots page controls on. A file you have edited yourself is left exactly as it is.

### Start.bat

- **Your PC stays responsive.** Building the server, PostgreSQL and the game server now run at a lower priority, so the game and your other programs get the processor first.
- The build and the game server leave a quarter of the processor free.
- The launchers' animated backgrounds redraw less often, and only while their window is in front.

## New servers start with 100,000 bots again

*2026-11-13 13:02 UTC*

A fresh server now starts the way the shared setup always did: 100,000 bots in New Eden, accounts made automatically the first time someone logs in, and the bot controls switched on in the dashboard.  Servers that are already set up keep their own rmr.yml, so nothing changes there; the bots count in that file still decides how many fly. To get the new defaults on an existing server, set bots count to 100000 in rmr.yml and restart.  The startup changes from earlier today are also in: the market check now prepares the database before it runs, and Start.bat waits for the server for as long as its log keeps moving instead of giving up after a fixed time.

## Passwords stored hashed

*2026-11-13 12:04 UTC*

**Passwords are now stored hashed.**

- Game account passwords are kept only as salted **PBKDF2-SHA256** hashes (600,000 iterations). Existing accounts are converted when the server starts.
- A password typed into `rmr.yml` is hashed at the next start and written back as `passwordHash`, so the file no longer holds it in the clear. Weak passwords get a warning in the server log.
- `rmr.yml` is now each server's own file: the server writes a default one if there is none.
- The Dashboard.bat sign-in uses the same hashing and stays in memory only.

*Restart Start.bat to rebuild.*

## Dashboard.bat keeps nothing

*2026-11-13 11:55 UTC*

**Dashboard.bat keeps nothing between runs.**

- Each start asks for a new sign-in, address and port.
- The sign-in goes straight to the dashboard program and stays in its memory only. Nothing is written to disk.
- The program runs from a temporary copy, and the Windows Firewall opening lasts only for the session.
- Stop it with **Ctrl+C** and both are removed. If the window was closed with its X instead, the next start cleans up.

*Restart Start.bat once so it rebuilds the dashboard program.*

## Start.bat build fix for PCs without MinGW

*2026-11-13 11:47 UTC*

**Start.bat builds on any PC again.** On PCs without MinGW installed, the first build failed with *dlltool could not create import library*. Rust's toolchain includes `dlltool` but not the GNU assembler (`as.exe`) that `dlltool` needs.

- The download now includes GNU `as.exe` and a matching `dlltool.exe` in `vendor\binutils`, along with the DLLs they need.
- Start.bat installs them into its tools folder if any are missing or changed, checking each against its checksum. Nothing is installed elsewhere on the PC.

*Download the latest version, or run git pull, and start Start.bat again.*

## Launcher and dashboard fixes

*2026-11-13 01:32 UTC*

- **Launcher:** EVE is given exactly the server address you pick, and the online/offline lights check that address as entered.
- **Dashboard.bat** keeps nothing between runs. Every start asks for a new sign-in, which is held only in memory, and closing the window forgets it.

## New EVE-style launcher

*2026-11-13 01:13 UTC*

**Client.bat** is now a launcher laid out like today's EVE launcher:

- **Servers** on the left: *This PC* plus any you add, each with a live online/offline light. Add, Edit and Remove are under the list.
- **News** from the chosen server in the middle. Double-click a story to read it.
- A big **PLAY** button that points your EVE client at the chosen server and starts it.
- An **EVE client** panel to change the client folder or *Repair client* (re-apply the patches), and an option to close the launcher when EVE starts.

## Start.bat, Client.bat and Dashboard.bat

*2026-11-13 01:02 UTC*

The launcher is now three files, each with one job.

- **Start.bat** runs the server. It no longer touches the game client, and its **Launch EVE** button opens Client.bat.
- **Client.bat** sets up and starts the game: it finds and backs up your EVE 2.10 client, points it at the server, and starts it.
- **Dashboard.bat** puts this dashboard behind a sign-in. Five wrong passwords lock an address out for 15 minutes.

*Run Start.bat once first so it builds the dashboard program.*

## Agents tab and channel member fixes

*2026-11-13 00:49 UTC*

- **People & Places, Agents tab:** a broken agent group saved by an earlier version stopped the tab loading partway, so the **Level N agents available to you** groups never appeared. That group is now removed while the tab loads, and the level groups always get added.
- **Corp and alliance chat members:** the member-list refresh was reading the wrong object for those channels and failing. It now uses the same lookup as Local.

*Restart the game from Start.bat to pick this up.*

## People & Places agents no longer freeze

*2026-11-13 00:45 UTC*

The real cause of the People & Places freeze: the Agents tab was building one stock **All agents** group holding every agent you can use, all **10,006** of them. An old saved group also made the tab error out halfway through loading.

- The stock list now holds only your real contacts and the agents you have missions with.
- Every other agent is in the **Level N agents available to you** groups. These open one level, faction and corporation at a time, so only a small batch of names loads at once, much like the Character Sheet's agent standings.
- The broken saved group is cleared automatically.

*Restart the game from Start.bat to pick this up.*

## High-sec, chat stars, corp chatter and bills

*2026-11-13 00:43 UTC*

### High-sec is safe unless you're at war

In 0.5 and above, bots now only start fights with their **war targets**. Grudges, raids, fight calls and belt quarrels all stay out of high-sec. Beyond that, a bot only fights back against someone who shot it first, and CONCORD deals with them.

### Chat

- Corporation mates show a **green star** and alliance mates a **blue star** in Local, corp and alliance chat, like the red star for war targets.
- Corp and alliance chat now fill in their member lists (alliance chat was showing none) and keep them up to date.
- Bots in your corporation and alliance now chat to each other: friendly, polite talk about ops, hauling, missions and warnings, and now and then they answer you.

### Wallet

- If your corporation is its alliance's executor, the corp wallet's **Bills** now lists the alliance's maintenance and war bills, and you can pay them from there.
- Corporation offices now get a **Rental Bill** five days before their rent runs out. Paying it keeps the office another period.

### Dashboard

- The overview now uses wide screens: server status sits beside server health, wars beside pilots online, and news runs along the bottom.
- Bot settings: an **Extra high** option for bots applying to your corporations and alliances, which means many more applicants waiting at once.

*Restart the game from Start.bat for the chat icons and member lists. Press Ctrl+F5 on the dashboard to load the new layout.*

## Faster agent lists and mail

*2026-11-13 00:30 UTC*

Two sources of client lag are gone.

### People & Places agent lists

The **Level N agents available to you** groups now list each agent as a plain row: name and home station. Before, every row was a contact card, and the client drew the agent's face in 3D on the spot. Most of those faces failed to draw, so they were never cached and were redrawn every time you scrolled. Right-click still gives the agent's normal menu, and double-click starts a conversation.

### Mail

The inbox now loads your **newest 250 mails** instead of every mail you have ever received; killmails had piled into the thousands. Older mails show up again as you delete newer ones.

*Restart the game from Start.bat to pick up the client change. The mail limit is already live.*

## 2,500 m to loot, jump and take acceleration gates

*2026-11-12 23:54 UTC*

- **Fixed:** containers and wrecks could be opened and looted from too far away. You now have to be within **2,500 m** to open one or take anything out of it. Starbase structures follow the same rule.
- **Fixed:** stargates let you jump from up to about 3,100 m. It's now **2,500 m**, as the game says.
- **Fixed:** acceleration gates in deadspace complexes and missions worked from about 5,000 m. They now need you within **2,500 m** of the gate itself, and tell you how far away you are if you're not.
- All distances are measured from the edge of your ship to the edge of the object, as the overview shows them.

## Dashboard: control sovereignty warfare and battle size

*2026-11-12 23:46 UTC*

The Bots page has a new **Sovereignty warfare** section, in case the null-sec wars weigh on the server. Both settings apply at once and are saved for the next start.

### Sov warfare: low, moderate or war

- **Low**: sieges are rare, at most 2 at once across New Eden, with one allied alliance a side. Alliances claim systems and found outposts slowly.
- **Moderate** (the default, as before): regular sieges, at most 6 at once, with up to 3 allies a side.
- **War**: sieges all over null-sec, up to 24 at once, with up to 5 allies a side. Alliances expand fast. This is the heaviest setting for the server.
- Turning it down stops new sieges. Those already announced or being fought play out.

### Battle size

- Choose the most ships one side brings to a battle, allies included: 50, 100, 250, 500, 1,000 or 2,000. You can also type any number from 10 to 3,000.
- It takes effect from the next battle. A side still brings fewer when the stakes are low or it hasn't the pilots.
- How many ships you see on grid is still capped by **Bot ships shown per system**. The rest come in as reserves or fight in the background.

## The System Scanner names the complexes it finds

*2026-11-12 23:34 UTC*

- **Fixed:** the System Scanner listed each result by a long ID number. It now lists the complex by name and rating, e.g. **Pith's Penal Complex (DED 8/10)**.
- Complexes you give yourself from the dashboard are named straight away.
- A complex you scan down shows as **Cosmic Signature** until your probes pin it close enough to warp to. Then its name appears.
- Warp To from the results list works as before.

## Null-sec security reads 0.0 again

*2026-11-12 23:29 UTC*

- **Fixed:** null-sec systems showed their true security (−0.0 down to −1.0) everywhere: the location panel, the star map, the route, Show Info and killmails. As in 2007, every null-sec system now reads **0.0**.
- Null-sec systems also had the wrong colour on the star map and route. The client looked their negative value up from the far end of its colour table and painted them like high-sec. They're now the deep red of 0.0.
- The dashboard shows 0.0 for null-sec too.
- Display only: the true security still decides rats, ores, belts and everything else about a system.

## Bots are friendly in high-sec, and CONCORD always gets its criminal

*2026-11-12 23:27 UTC*

### Bots in high-sec

- **Fixed:** bots attacked players in high-sec (1.0 to 0.5). They no longer start a fight there for any reason: not war targets, grudges and hunters, pirate raids, fight calls in Local, or the dashboard's aggressive mode.
- In high-sec a bot only shoots back, at a pilot who attacked it or its corporation. It lets go of anyone else at once, even mid-fight.
- Hunters sent after you wait for you to leave high-sec instead of following you around it. Bots from different corporations no longer start fights in high-sec asteroid belts either.
- Pirate **raids** now hit a low-sec system (0.1 to 0.4) near the pirates' space instead of high-sec.
- Low and null-sec are unchanged: pirates, roaming gangs, hunters and war targets still come for you there.

### CONCORD

- **Fixed:** CONCORD only ever went after players. A bot that commits a crime in high-sec is now hunted and destroyed like anyone else.
- **Fixed:** docking, jumping out or warping away before CONCORD arrived got a criminal off. Now the warrant follows you for the 15 minutes of your criminal flag: CONCORD comes for your ship in any high-sec system you fly in. It warps in again wherever you land if you warp off.
- The criminal flag counts everywhere, so other pilots can shoot a criminal in any system, not just the one where the crime happened.
- Once CONCORD has destroyed the criminal's ship the hunt is over, but the flag stays until it runs out. Response times are unchanged: about 5 seconds in 1.0, up to about 25 in 0.5.

## People and Places lists every agent who will work with you

*2026-11-12 23:20 UTC*

### Agents tab

- **Fixed:** the **Agents** tab in **People and Places** only showed agents you already had a mission with. It now lists **every mission agent who will work with you**, using the same standing rule the agents use: level 1 for anyone, level 2 at 1.0, level 3 at 3.0, level 4 at 5.0 and level 5 at 7.0. That's your best standing with the agent, its corporation or its faction. An agent at −2 or worse with you on any of those won't be listed.
- The agents are arranged like the Character Sheet: one group per level, then by faction, then by corporation, each group showing its count. Names are fetched a group at a time, so opening a big group doesn't freeze the window.
- The built-in **All** group still holds the full list, and it now loads its names in a few batches instead of one at a time.
- Right-click any agent to start a conversation, show info or set a route to their station. The list is built when you log in, so a new level unlocked by a standing gain appears the next time you log in.

## Corporation and alliance chat list their members

*2026-11-12 23:13 UTC*

### Fixes

- **Fixed:** the **Corp** and **Alliance** chat channels listed nobody but you. They only counted pilots who had opened the channel themselves, and never the bot corporations and pilots in your corporation or alliance.
- Each now lists everyone in the corporation or alliance who is online, players and bots alike (up to 3,000), and the member count matches.
- Members carry their standing icons as they do in Local: the green star for your corporation, the blue star for your alliance.

## Sovereignty warfare: thousand-ship battles, and bots who hold space for you

*2026-11-12 23:09 UTC*

### Battles at real scale

- Sieges are now fought by whole fleets. Each one is a **skirmish** (around a tenth of an alliance's fleet), a **battle** (a third or more) or a **decisive battle** (nearly everything they have), so you'll see fights from **100 against 100** to **well over 1,000 a side**.
- **Coalitions:** alliances at war with one side often join the other, so many battles are several alliances against several more.
- **Watch it in space:** fly to a battle and it is fought ship against ship. The front line is as big as your system can show, and as ships die, reinforcements warp in from the rest of the fleet until a side is spent or routs. It also plays out if you're **docked** in the system, so you can undock into it.
- **Or follow it from anywhere:** where no one is watching, the battle is still fought out second by second with the full numbers, and **every ship lost is a killmail**. It switches seamlessly between the two as pilots arrive and leave.
- A fleet worn down to a fifth and outnumbered two to one withdraws. Sieges come round more than twice as often as before.

### Bots who hold space for your alliance

- Once your alliance takes in bot corporations, they claim null-sec for it: a home region of its own (where your towers already stand, or free space), with control towers anchored and fuelled by them.
- They **defend your space**: shoot one of your alliance's towers and all its bots turn out, not just the corporation that owns it.
- Your alliance becomes part of null-sec politics: neighbouring bot alliances go to war with it and besiege its systems, and your bots besiege theirs.
- With four or more settled systems, an alliance's bots **found outposts** in its space, one for every four systems held.

### Space draws numbers

- The more systems an alliance holds, the more corporations want in: recruits favour corporations in big alliances, free bot corporations join the bot alliances holding the most space, and more corporations apply to player alliances holding space.

### Watching it all

- The dashboard's **Sieges and battles** page shows every siege massing and every battle under way: the stakes, both sides and their allies, ships left of what each brought, losses, and whether it's being fought in view or beyond sight. Every battle and its result also goes in the null-sec news.
- To see bigger battles on one grid, raise **Bot ships shown per system** on the Bots page. It now goes up to 2,500.

## Mission and DED complex guides on the dashboard

*2026-11-12 22:44 UTC*

### A guide for everything

- The dashboard's **New Eden** section has a new page, **Mission & DED guides**, with a walkthrough for every agent mission (375) and every DED complex (45).
- Each guide gives the objective, the enemy, the damage they deal and what to shoot them with, and which agents offer the mission.
- It then goes pocket by pocket: what is waiting when you land, what joins as you close in, and what arrives once a group dies, ship by ship, with overseers and trigger ships marked.
- For every acceleration gate it says how far away it is and whether it stays locked until the pocket is clear.
- Complexes laid out from CCP's originals also list the structures in each pocket.

### Finding a guide

- Show **missions**, **DED complexes** or both.
- Narrow down by **mission level** (1 to 4) or **DED rating** (1/10 to 10/10), and by **faction**.
- **Search** by mission or complex name, or by any ship or pocket in it: search a ship's name to see every mission and complex it turns up in.
- The guides are written from the same files the server spawns from, so they always match what you meet in space.

## The Strike chains, more real DED layouts, and every mission spawning

*2026-11-12 22:05 UTC*

### The Strike chains

- Level 3 agents now offer the fighting parts of **Angel Strike** and **Guristas Strike**: Ambush in the Dark, The Kidnapping, Incriminating Evidence, The Secret Meeting, Defend the Civilian Convoy, Retrieve the Prisoners and The Final Battle.
- Each comes in its Blood Raider, Guristas, Sansha or Serpentis version, with the Darkonnen, Komni, Maru and Odamian gangs, Drazin Jaruk and Zerak Cheryn as they were.
- **Kidnappers Strike** is the same set of missions, so it is covered too.
- Each part is offered as a mission of its own. The courier parts in between have no fighting and aren't offered.

### More missions

- Level 3: all three parts of **Missing Reporters**, **Intercept the Saboteurs** (Sansha) and **What Comes Around Goes Around**.
- Level 4: **Intercept the Saboteurs** (Serpentis).
- Level 2: all three parts of **Minmatar Plot**.
- Agents now hand out **375** missions: 78 at level 1, 86 at level 2, 126 at level 3 and 85 at level 4.

### DED complexes

- **28 of the 45** complexes now use CCP's own layouts. 21 of them have their acceleration gates where they really stood, and 24 have their original structures and scenery: over 2,000 pieces in all.
- Newly laid out include the **Serpentis Corporation Hydroponics Site**, **Serpentis Logistical Outpost**, **Serpentis Prison Camp**, **Angel Repurposed Outpost**, **Angel Cartel Occupied Mining Colony**, **Blood Raider Intelligence Collection Point**, **Pith's Penal Complex**, **Sansha's Nation Occupied Mining Colony** and **Sansha Prison Camp**.

### Fixes

- **Vigilance** (Gallente, level 2) and **No Honor** now spawn their suspicious industrial, as a Gallente transport ship.
- The R.S. Officer in **Illegal Activity** now always appears.
- **For Justice** could be offered but never spawned anything: its ships came later than this era, so it has been removed.

### Kept true to the era

- Left out because their ships came later: **New Frontiers** (Professor Delainen's drones), **Head Hunter Threat** (Malbreth) and **For Justice**.

## DED complexes get their real layouts, and 33 more missions

*2026-11-12 21:58 UTC*

### DED complexes: CCP's own layouts

- Eight complexes are now laid out as CCP built them: **Serpentis Narcotic Warehouses** (3/10), **Serpentis Fleet Shipyard** (10/10), **Angel's Red Light District** (5/10), **Crimson Hand Supply Depot** (6/10), **Guristas Scout Outpost** (4/10), **Guristas Troop Reinvigoration Camp** (6/10), **Sansha Military Outpost** (1/10) and **Sansha's Command Relay Outpost** (3/10).
- Their acceleration gates stand where and how far away they really were.
- Their rooms are no longer empty space: bunkers, junctions, elevators, lookouts, habitation modules, neon signs, rock fields and gas clouds sit where the original layouts put them, up to 80 per room.
- The other complexes keep their current layouts until their rooms can be matched reliably.

### Level 3: now 99 missions

- New: **Id & Egonics Inc.**, **Scout It Out** and both parts of **Secret Project X**; **Lights Out**, **A Million Little Pieces**, **No Excuse** and **Illegal Activity** (Gallente); **Revenge Attack**, **Trespassers** and both parts of **Technological Secrets** (Thukker and Minmatar); **Mannar Mining Colony** (rogue drones).

### Level 2: now 82 missions

- New: **Ammatar Scouts Spotted**, **Imperial Scouts Spotted**, **Break the Slave Trade**, **Delving Into the Past**, **Human Cattle**, both **Deadly Arrival**s, **Clearing a Path**, **Communications Cold War**, **Elopement**, **Illegal Activity**, **Intercept the Saboteurs** and **Pirate Business** (Minmatar), **Freedom Must Not Prevail**, **Eliminate the Pirate Campers** (Thukker), **Trimming the Fat**, **Save a Man's Career**, **Preliminary Strike**, **The Seven's Brothel** and **No Honor**.
- Minmatar agents now also send pilots against the **Khanid** and **Thukker** renegades, so missions like The Score (Khanid) can finally be offered.

### Kept true to the era

- Left out because their ships came later: **Lasers Softly Beaming**, plus multi-part chains and pages with no ship lists.

## 30 more missions, and transports that carry the goods

*2026-11-12 21:45 UTC*

### Level 4: now 84 missions

- New: **Pot and Kettle** (Amarr), **Khanid Nobleman** (mercenaries and Khanid), **Exploited Sensitivities** and **Federal Confidence** (Caldari).

### Level 3: now 86 missions

- All four parts of **The Uprising** against Mordu's Legion and its militia.
- **Intercept the Saboteurs**, **Probing Question** and **Deadly Arrival** against the Amarr; **Retribution**, **Transport Duty** and **Pirate Business** against the Caldari; **Friendly Spies** (Gallente); **Pirate Radio**, **The Slaves Must Be Returned** and the other **Deadly Arrival** (Minmatar); **Ammatar Detective Spotted**.
- Both parts of **The Rogue Slave Trader** (Blood Raiders), **Silence Deep as Death** (Angels), **The Space Telescope** and **Success Comes at a Price** (Guristas), **Striking Down Two Flies With One Swing**.
- Mercenary jobs: **The Black Market Hub**, **Operation Hamstring**, **Crystal Dreams Shattered**, **The Seven's Prison Facility**; and **Down with the Legion**.

### Kept true to the era

- Left out because their ships came later: **Dark Alliance**, **Amarrian Tyrants** and **Save the Miners**.

### Fixes

- **Transports and industrials now turn up in missions at every level.** They were being left out, so in Smuggler Interception, Unauthorized Military Presence and the like the ship carrying the mission item never appeared.
- **Vigilance** (Guristas, level 3) can be offered again: its suspicious industrial is flown by a Guristas cruiser.

## 32 more level 4 missions

*2026-11-12 21:35 UTC*

### New level 4 missions

Level 4 security agents now hand out **80** missions, up from 48. The new ones, rebuilt pocket by pocket from the old mission reports:

- **Massive Attack** (Amarr, Sansha, Serpentis), **Pirate Slaughter** (Angels, Guristas) and **Cargo Delivery** (Blood Raiders, Serpentis).
- **Intercept the Saboteurs** (Amarr, Blood Raiders, Guristas, Minmatar), **Navy Armada** (Caldari, Gallente), **Police Invasion** and **The Score** (Minmatar, Ammatar).
- **Smash the Supplier**, **Smash the Shipyard**, **Diplomatic Incident**, **War Situation** and **Surprise Surprise** against the Amarr Empire.
- **Ambush the Convoy**, **Buzz Kill**, **Ritualist Raids**, **Mining Misappropriation**, **Worlds Collide** (Serpentis and Guristas) and both parts of **The Rogue Slave Trader**.
- **Gone Berserk** and **The Kidnapping** against the EoM, **Mordus Folly** and **The Right Hand of Zazzmatazz**.

### Kept true to the era

- Missions whose ships this client doesn't have came later, so they are left out: **Serpentis Ship Builders** (Colonel 'Bat' Guano) and **The Wildcat Strike** (reprogrammed mining drones).
- Where a mission report only lists ship classes, you meet that faction's own ships of each class.

### Fixes

- Ships the reports spell differently from the game, such as Angel Centurions and Phalanxes or Serpentis Guardian SafeGuards, now spawn in missions at every level instead of being replaced.

## 248 classic agent missions added

*2026-11-12 20:28 UTC*

### Real agent missions are here

Security agents now hand out **248 classic combat missions** from the Revelations to Apocrypha era, rebuilt pocket by pocket from the old mission reports.

### What's in

- **Level 1:** 79 missions, including The Blockade, Worlds Collide, Pirate Intrusion, Hidden Stash and the Spies missions.
- **Level 2:** 62 missions, including Recon, The Uprising (all 4 parts), Silence the Informant and Police Intrusion.
- **Level 3:** 59 missions, including the Angel, Guristas and Serpentis Extravaganzas, Duo of Death, Pirate Invasion and The Missing Convoy.
- **Level 4:** 48 missions, including The Blockade, Angel Extravaganza, The Assault, Vengeance, Worlds Collide, Recon and Attack of the Drones.

### How they play

- Every mission has its own pockets, acceleration gates and gate distances.
- Waves come the way they used to: some when you arrive, some when the room is clear, and many when a particular *trigger ship* dies. Pick your targets carefully!
- Locked gates stay shut until their pocket is cleared.
- You'll fight the pirates who raid your agent's space, plus rogue drones, mercenaries, Mordu's Legion and the EoM. Empire agents also send you against their rival navy.

*More missions, and refreshed DED complex layouts, are on the way.*

## Tech II missiles no longer drain your capacitor

*2026-11-12 19:35 UTC*

### Tech II missiles no longer drain your capacitor

Capacitor Power Relays were working all along. Something else was quietly undoing them.

### What was happening

- Every Tech II missile (**Fury**, **Precision**, **Javelin**, **Rage** and the rest) carried a hidden drawback: **+20% capacitor recharge time**.
- It applied once *per loaded launcher*, so a full rack of Fury cruise missiles made your capacitor recharge over three times slower.
- On a Raven Navy Issue with five Capacitor Power Relay IIs, that wiped out nearly the whole relay bonus.

### What's changed

- Tech II missiles no longer affect your capacitor at all.
- Capacitor Power Relays now give their full bonus whatever you have loaded.
- Example: that Raven Navy Issue goes from about **13.5 GJ/s** peak regen to about **48.5 GJ/s**.

*Load the good missiles without worrying about your cap.*

## Missile visuals and one-drag ammo loading

*2026-11-12 19:10 UTC*

### Missiles you can see, and one-drag ammo loading

### Missile visuals

- Missiles now fly across space from the launcher to their target instead of only being heard.
- When a missile hits, it explodes on the target ship.
- A missile fired at a target beyond its flight range burns out and vanishes without a hit.
- This covers your launchers, rat missiles and starbase missile batteries.

### Ammo loading

- Dropping ammo on the fitting window now fills **every** matching weapon to the brim in a single drag.
- No more dragging the stack over again and again.

*See you in the belts.*

## Reload all and drag-and-drop ammo loading

*2026-11-12 19:02 UTC*

### Ammo loading fixed

You can reload all your guns from space again, and docked loading now works the way you'd expect.

### Reload all in space

- Right-click any weapon or launcher on your HUD and pick **Reload all**.
- Every module of that type gets topped up from your cargo hold, using the ammo already loaded.
- If a module is empty, it loads the first matching ammo it finds in cargo.

### Drag ammo onto the fitting window

- While docked, drag an ammo stack from your hangar or cargo and drop it anywhere on the fitting window.
- All fitted weapons that can use that ammo get loaded, filled to capacity.
- Any different ammo they held goes back to your cargo hold.
- Dropping onto a single slot loads just that weapon, swapping out its old ammo.
- Wrong ammo for your guns? You'll get a clear message, and nothing moves.

*Fly safe, and keep your hold stocked.*

## The repair shop fixes armor

*2026-11-12 18:01 UTC*

### Fixes

- **Fixed:** the station **Repair Items** window only saw structure damage, so armor damage could not be repaired. A quote now covers a ship's armor and structure together, and **Repair** restores both.
- Your ship shows repaired straight away, in the fitting window and elsewhere, with no need to undock first.
- Repairs are still free.

## Unfitting and moving items in built outposts

*2026-11-12 17:54 UTC*

### Fixes

- **Fixed:** in an outpost built from a construction platform, nothing could be moved into the item hangar. Unfitting a module by dragging it or with **Unfit** silently failed. Outposts were built without the station item that NPC stations have, so the client could not tell what kind of container the hangar was.
- New outposts get that item when they are built, and the server adds it at start-up to any station missing one, so existing outposts are fixed with nothing else changed.

## Spawned complexes show in the scanner and open from their bookmark

*2026-11-12 17:43 UTC*

### Fixes

- **Fixed:** a complex spawned from the dashboard did not appear in the **System Scanner**. The window threw away its whole results list because each result's position arrived in a form it could not measure the distance to. Results now arrive as positions it can use, so spawned complexes and scanned signatures are listed with their distance and accuracy.
- **Fixed:** warping to a complex's **DED bookmark** landed you in empty space. A bookmark warp that lands near a complex you were given, or have scanned down, now opens it as you arrive, gates and defenders included.

### Good to know

- Signatures and spawned complexes are kept in memory, so a server restart clears them. Spawn a fresh one from the dashboard after restarting.

## Gallente outposts get a proper hangar

*2026-11-12 17:17 UTC*

### Fixes

- **Fixed:** docking in a **Gallente Administrative Outpost** showed the unfinished CONCORD hangar. Its docking platform was see-through, and your ship struggled to load as you zoomed. Gallente outposts now use the Gallente station hangar, as Caldari, Minmatar and Amarr outposts already use their own race's.

## Outposts keep their names and hangars

*2026-11-12 16:51 UTC*

### Fixes

- **Fixed:** an outpost could show up as **"Strip bar at the bottom of the universe"** after a restart or relog, and items bought there did not show in its hangar. The game's static data has a few named locations sharing outpost IDs, and outposts were being given those IDs. The client took the static location's name and position over the station's.
- At start-up an outpost's static location is now set to the outpost's own name and position, so existing outposts are fixed with no changes to their items, offices or market. Location lookups prefer the station's own name, and new outposts get IDs clear of the static ones.
- **Buying at your own station delivers there.** A market buy now fills from sell orders at the station you are buying from first, and only reaches other stations in range for anything that station cannot cover. Before, it took the cheapest order anywhere in range, so purchases at an outpost often went to an NPC hub instead.

## Exploration: scan down DED complexes, with deadspace and faction loot

*2026-11-12 16:40 UTC*

### Scanning

- Every system now hides **cosmic signatures**: DED complexes of the local pirate faction. High-sec holds 1/10 to 5/10, low-sec 3/10 to 8/10 and null-sec 6/10 to 10/10, somewhere between 3 and 25 AU from the sun. They last one to three days, and a completed complex is gone for good.
- Fit a **Scan Probe Launcher**, load scanner probes and activate it **once**: it launches all its probes together, **up to 8 in space**, spread round your ship in a pattern so their ranges overlap where you are. Each covers its rated range (3, 12, 48 AU and so on) for its lifetime, and the system scanner lists them. Probes can't be moved once launched in this client, so to scan somewhere else, fly there and launch more.
- Select **three or more probes**, press Analyze and choose **Cosmic Signature**. Any signature inside all of your selected probes' ranges shows as a result. More probes give better accuracy, and harder complexes are harder to pin down.
- **Warp To** a result. Within 150 km of the truth takes you to the complex, which opens there with its acceleration gates. A rougher result only gets you close, so analyse again with more probes.
- **Tip:** start with big probes (Spook 48AU, Fathom 12AU) to see which signatures are about, warp towards a rough result, then launch Snoop 3AU probes there. Six or more give a fix close enough to warp straight in.

### DED loot

- Overseers now drop **Overseer's Personal Effects** of a tier to match the complex's rating.
- They may also drop a **deadspace module** of their faction, sized to the complex: Coreli, Gistii, Corpii, Pithi and Centii in 1/10 to 3/10; Corelum, Gistum, Corpum, Pithum and Centum in 4/10 to 6/10; Core, Gist, Corpus, Pith and Centus in 7/10 and up. Most are C-Type, fewer B-Type, rarely A-Type, and X-Type is possible in 10/10s.
- Now and then they drop **pirate faction modules**: Shadow Serpentis, Domination, Dark Blood, Dread Guristas and True Sansha.
- The last room's overseer drops more and better. A complex with no overseer in its last room pays out when that room is cleared.

## Deadspace complexes: DED 1/10 to 10/10, with acceleration gates

*2026-11-12 16:29 UTC*

### Dungeons

- Sites can now have **several rooms** (deadspace pockets) joined by **acceleration gates**. Right-click a gate and choose **Activate Gate** to be flung into the next room. **Locked gates** open once that room's defenders are dead.
- Rooms have **waves**: some ships wait for you when you arrive, and **reinforcements** come once a room is cleared. Overseers lead the last rooms, and clearing the last room completes the complex.
- Rats in complexes are the factions' proper **deadspace ships** (Coreli, Gistii, Corpii, Pithi, Centii and their bigger kin) and named overseers, scaled to the complex's rating.

### 45 DED complexes

- **Serpentis, Angel Cartel, Blood Raiders, Guristas and Sansha's Nation** each have their 1/10 to 8/10 and 10/10 complexes, such as the Drug Outlet, Red Light District, Mul-Zatah Monastery, The Maze and Centus Assembly T.P. Co.: 165 rooms in all. There were no 9/10s in this era.
- Layouts, ship groups and overseers follow the published guides for these sites. Gate distances are estimates. Where the game's data lacks an overseer, a strong deadspace ship stands in under its name.
- Complexes are data files (`dungeons/`), so more can be added without rebuilding the server.

### Dashboard

- A new **Deadspace complexes** page lists every complex. Pick an online pilot, optionally a system, and press **Spawn for pilot**. A pilot in space in that system warps straight in. Otherwise the complex waits in their **System Scanner** as a result already pinned down: no probes needed, just **Warp To**.

### Coming next

- Finding complexes with **scan probes**, then agent missions built on the same rooms and gates.

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
