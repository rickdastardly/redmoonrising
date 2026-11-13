//! Dungeons: deadspace complexes (and, later, mission sites) of several
//! rooms joined by acceleration gates, each room with waves of rats.
//!
//! What a dungeon holds comes from data, not code: `dungeons/*.json` beside
//! rmr.yml, made from the text in `dungeons/src` by `dungeons/build.py`, and
//! read at start (or from the dashboard's Dungeons page, without a restart).
//! A dungeon has rooms; a room has waves, each of ship groups: so many of
//! some named NPC types (or, where the game lacks them, deadspace ships of a
//! class), an overseer perhaps. A room's first waves come when a pilot
//! enters it; a wave marked `cleared` comes once everything before it in the
//! room is dead.
//!
//! In space a dungeon is a site (`sites`): its beacon at the first room's
//! warp-in, and an acceleration gate in every room but the last. Rooms are
//! pockets far apart, each out of sight of the others. A gate (keeper
//! ActivateAccelerationGate, from the client's Activate Gate) sends a ship
//! within reach on to the next room by warp, waking that room's rats; a
//! locked gate opens only once its room's defenders are all dead. The last
//! room cleared completes the dungeon, and its mission if it has one.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock, RwLock};

use eve_marshal::Value;
use serde::Deserialize;
use serde_json::{json, Value as Json};
use tracing::{info, warn};

use crate::ballpark::{Park, Vec3};
use crate::combat::Rng;
use crate::loot::Stash;
use crate::npc::{self, DungeonRats};
use crate::services::{Ctx, Outcome};
use crate::sites::{Progress, Site, TYPE_BEACON};
use crate::svc::space;

/// Acceleration Gate (group Warp Gate, which the client's Activate Gate needs).
pub const TYPE_ACCELERATION_GATE: i64 = 17831;
/// How far apart a dungeon's rooms are: each its own grid, out of sight.
const ROOM_SPACING: f64 = 2.0e10;
/// How close a ship must be to a gate to use it: 2,500 m, surface to surface.
const GATE_REACH: f64 = 2_500.0;
/// The most ships one group spawns, whatever its data says.
const GROUP_CAP: u32 = 60;

// ---- the data -------------------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Dungeon {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub kind: String,
    pub faction: String,
    /// DED rating, 1 to 10; a mission's level.
    #[serde(default)]
    pub rating: u8,
    /// What a mission asks of its pilot.
    #[serde(default)]
    pub objective: String,
    pub rooms: Vec<Room>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Room {
    pub name: String,
    /// Whether the gate on to the next room opens only once this room's rats are dead.
    #[serde(default)]
    pub gate_locked: bool,
    /// How far the gate on is from the warp-in, in metres.
    #[serde(default)]
    pub gate_distance: f64,
    /// Where the gate on is, from the warp-in (CCP's layout), when known.
    #[serde(default)]
    pub gate_pos: Option<[f64; 3]>,
    /// The room's structures and scenery (CCP's layout), from the warp-in.
    #[serde(default)]
    pub objects: Vec<RoomObject>,
    pub waves: Vec<Wave>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoomObject {
    pub type_id: i64,
    pub name: String,
    pub pos: [f64; 3],
    #[serde(default)]
    pub radius: f64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Wave {
    /// "arrive": when a pilot enters the room; "cleared": once the room is
    /// clear; "killed": when one of the ships named in `on` dies.
    #[serde(default)]
    pub trigger: String,
    /// For "killed": the ships whose death brings the wave.
    #[serde(default)]
    pub on: Vec<String>,
    pub ships: Vec<Ships>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Ships {
    pub count: u32,
    /// frigate, destroyer, cruiser, battlecruiser or battleship: for a ship
    /// whose named types the game lacks.
    #[serde(default)]
    pub class: String,
    /// NPC types, taken in turn.
    #[serde(default)]
    pub names: Vec<String>,
    #[serde(default)]
    pub boss: bool,
    /// An overseer the game lacks: the name its stand-in goes by.
    #[serde(default)]
    pub label: Option<String>,
    /// A sentry gun or tower rather than a ship.
    #[serde(default)]
    pub fixed: bool,
}

impl Dungeon {
    /// Every NPC type the dungeon names.
    pub fn ship_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.rooms.iter().flat_map(|r| &r.waves).flat_map(|w| &w.ships).flat_map(|s| s.names.iter().cloned()).collect();
        names.sort();
        names.dedup();
        names
    }

    /// Every NPC type the dungeon names, with the class (0 frigate to 4
    /// battleship) its groups give it: for factions with no deadspace ships.
    pub fn name_classes(&self) -> Vec<(String, usize)> {
        let mut out: Vec<(String, usize)> = self
            .rooms
            .iter()
            .flat_map(|r| &r.waves)
            .flat_map(|w| &w.ships)
            .flat_map(|s| s.names.iter().map(move |n| (n.clone(), class_index(&s.class))))
            .collect();
        out.sort();
        out.dedup_by(|a, b| a.0 == b.0);
        out
    }

    pub fn ships(&self) -> u32 {
        self.rooms.iter().flat_map(|r| &r.waves).flat_map(|w| &w.ships).map(|s| s.count.min(GROUP_CAP)).sum()
    }
}

fn class_index(class: &str) -> usize {
    match class {
        "destroyer" => 1,
        "cruiser" => 2,
        "battlecruiser" => 3,
        "battleship" => 4,
        _ => 0,
    }
}

static CATALOG: RwLock<Vec<Arc<Dungeon>>> = RwLock::new(Vec::new());

fn data_dir() -> std::path::PathBuf {
    match crate::config::file_path().parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir.join("dungeons"),
        _ => std::path::PathBuf::from("dungeons"),
    }
}

/// Reads every dungeons/*.json; returns how many dungeons there are.
pub fn load_catalog() -> usize {
    let dir = data_dir();
    let mut all: Vec<Arc<Dungeon>> = Vec::new();
    match std::fs::read_dir(&dir) {
        Ok(entries) => {
            let mut files: Vec<std::path::PathBuf> =
                entries.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "json")).collect();
            files.sort();
            for file in files {
                match std::fs::read_to_string(&file).map_err(|e| e.to_string()).and_then(|t| serde_json::from_str::<Vec<Dungeon>>(&t).map_err(|e| e.to_string())) {
                    Ok(list) => all.extend(list.into_iter().filter(|d| !d.rooms.is_empty()).map(Arc::new)),
                    Err(e) => warn!("dungeons: {} could not be read: {e}", file.display()),
                }
            }
        }
        Err(e) => warn!("dungeons: no {} ({e}); there are no dungeons", dir.display()),
    }
    all.sort_by(|a, b| a.faction.cmp(&b.faction).then(a.rating.cmp(&b.rating)).then(a.name.cmp(&b.name)));
    let count = all.len();
    if let Ok(mut catalog) = CATALOG.write() {
        *catalog = all;
    }
    info!("dungeons: {count} loaded from {}", dir.display());
    count
}

/// The DED complexes of a faction rated `lo` to `hi`, for signatures.
pub fn pool(faction: &str, lo: u8, hi: u8) -> Vec<Arc<Dungeon>> {
    CATALOG
        .read()
        .map(|c| c.iter().filter(|d| d.faction == faction && d.kind == "ded" && (lo..=hi).contains(&d.rating)).cloned().collect())
        .unwrap_or_default()
}

/// The agent missions of a level.
pub fn missions(level: u8) -> Vec<Arc<Dungeon>> {
    CATALOG
        .read()
        .map(|c| c.iter().filter(|d| d.kind == "mission" && d.rating == level).cloned().collect())
        .unwrap_or_default()
}

pub fn find(id: &str) -> Option<Arc<Dungeon>> {
    CATALOG.read().ok()?.iter().find(|d| d.id == id).cloned()
}

/// The dungeons, for the dashboard.
pub fn catalog_json() -> Json {
    let list: Vec<Json> = CATALOG
        .read()
        .map(|c| {
            c.iter()
                .map(|d| json!({ "id": d.id, "name": d.name, "kind": d.kind, "faction": d.faction, "rating": d.rating, "objective": d.objective, "rooms": d.rooms.len(), "ships": d.ships() }))
                .collect()
        })
        .unwrap_or_default();
    Json::Array(list)
}

/// Every dungeon in full, for the dashboard's mission and complex guides:
/// rooms, gates, waves and what triggers them, ships, and each room's
/// structures counted by name.
pub fn guides_json() -> Json {
    let list: Vec<Json> = CATALOG
        .read()
        .map(|c| {
            c.iter()
                .map(|d| {
                    let rooms: Vec<Json> = d
                        .rooms
                        .iter()
                        .map(|r| {
                            let mut structures: Vec<(String, usize)> = Vec::new();
                            for o in &r.objects {
                                match structures.iter_mut().find(|(n, _)| *n == o.name) {
                                    Some((_, k)) => *k += 1,
                                    None => structures.push((o.name.clone(), 1)),
                                }
                            }
                            structures.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
                            let waves: Vec<Json> = r
                                .waves
                                .iter()
                                .map(|w| {
                                    let ships: Vec<Json> = w
                                        .ships
                                        .iter()
                                        .map(|s| json!({ "count": s.count, "class": s.class, "names": s.names, "boss": s.boss, "label": s.label, "fixed": s.fixed }))
                                        .collect();
                                    json!({ "trigger": w.trigger, "on": w.on, "ships": ships })
                                })
                                .collect();
                            json!({
                                "name": r.name,
                                "gateLocked": r.gate_locked,
                                "gateDistance": r.gate_distance,
                                "realGate": r.gate_pos.is_some(),
                                "structures": structures.iter().map(|(n, k)| json!([n, k])).collect::<Vec<_>>(),
                                "waves": waves,
                            })
                        })
                        .collect();
                    json!({ "id": d.id, "name": d.name, "kind": d.kind, "faction": d.faction, "rating": d.rating, "objective": d.objective, "rooms": rooms })
                })
                .collect()
        })
        .unwrap_or_default();
    Json::Array(list)
}

// ---- a dungeon in space ---------------------------------------------------------------------------

/// A dungeon under way, kept on its site.
#[derive(Debug, Clone)]
pub struct Run {
    pub def: Arc<Dungeon>,
    pub rats: Arc<DungeonRats>,
    /// Each room's warp-in.
    pub rooms: Vec<Vec3>,
    pub entered: Vec<bool>,
    /// Which waves of each room have come.
    pub spawned: Vec<Vec<bool>>,
    /// The rats alive, and their rooms.
    pub alive: HashMap<i64, usize>,
    /// What each rat was called for in the data (its trigger name).
    pub names: HashMap<i64, String>,
    /// Overseers still alive, and whether each is in the last room.
    pub bosses: HashMap<i64, bool>,
    pub done: bool,
    /// The completion stash has been dropped.
    pub paid: bool,
}

/// Opens a dungeon with its first room's warp-in at `at`: the beacon, the
/// gates, and the first room's rats. Returns the site and what pilots nearby
/// should be shown.
pub fn open(park: &mut Park, def: Arc<Dungeon>, rats: Arc<DungeonRats>, at: Vec3, char_id: i64, mission_id: i64, now: i64) -> (i64, Vec<(i64, Value)>) {
    let id = npc::next_id();
    let rooms: Vec<Vec3> = (0..def.rooms.len()).map(|k| at + Vec3::new(0.0, 0.0, k as f64 * ROOM_SPACING)).collect();
    let mut gates = Vec::new();
    for k in 0..def.rooms.len().saturating_sub(1) {
        let gate = npc::next_id();
        let distance = if def.rooms[k].gate_distance > 0.0 { def.rooms[k].gate_distance } else { 20_000.0 };
        let offset = match def.rooms[k].gate_pos {
            Some([x, y, z]) if x != 0.0 || y != 0.0 || z != 0.0 => Vec3::new(x, y, z),
            _ => Vec3::new(distance, 0.0, 0.0),
        };
        park.sites.insert(
            gate,
            Site {
                id: gate,
                mission_id,
                char_id,
                name: format!("Acceleration Gate ({})", def.rooms[k + 1].name),
                pos: rooms[k] + offset,
                rats: Vec::new(),
                wave: 0,
                waves: 0,
                security: -1.0,
                type_id: TYPE_ACCELERATION_GATE,
                dungeon: None,
                gate: Some((id, k + 1)),
                part_of: None,
                size: 0.0,
            },
        );
        gates.push(gate);
    }
    // Each room's structures and scenery, where CCP's layout is known.
    for (k, room) in def.rooms.iter().enumerate() {
        for object in &room.objects {
            let scenery = npc::next_id();
            park.sites.insert(
                scenery,
                Site {
                    id: scenery,
                    mission_id: 0,
                    char_id: 0,
                    name: object.name.clone(),
                    pos: rooms[k] + Vec3::new(object.pos[0], object.pos[1], object.pos[2]),
                    rats: Vec::new(),
                    wave: 0,
                    waves: 0,
                    security: -1.0,
                    type_id: object.type_id,
                    dungeon: None,
                    gate: None,
                    part_of: Some(id),
                    size: object.radius.max(1.0),
                },
            );
            gates.push(scenery);
        }
    }
    let run = Run {
        def: Arc::clone(&def),
        rats,
        rooms,
        entered: vec![false; def.rooms.len()],
        spawned: def.rooms.iter().map(|r| vec![false; r.waves.len()]).collect(),
        alive: HashMap::new(),
        names: HashMap::new(),
        bosses: HashMap::new(),
        done: false,
        paid: false,
    };
    park.sites.insert(
        id,
        Site {
            id,
            mission_id,
            char_id,
            name: def.name.clone(),
            pos: at,
            rats: Vec::new(),
            wave: 1,
            waves: 1,
            security: -1.0,
            type_id: TYPE_BEACON,
            dungeon: Some(Box::new(run)),
            gate: None,
            part_of: None,
            size: 0.0,
        },
    );
    let spawned = enter_room(park, id, 0, now);
    let mut nearby = space::add_object_entries_for_nearby(park, id);
    for gate in gates {
        nearby.extend(space::add_object_entries_for_nearby(park, gate));
    }
    for rat in &spawned {
        nearby.extend(space::add_ball_entries_for_nearby(park, *rat));
    }
    info!("dungeon {} ({}) opened at site {id} for {char_id}: {} rooms, {} rats in the first", def.name, def.id, def.rooms.len(), spawned.len());
    (id, nearby)
}

/// A pilot enters a room: its arrival waves come, the first time.
fn enter_room(park: &mut Park, site_id: i64, room: usize, now: i64) -> Vec<i64> {
    let waves: Vec<usize> = {
        let Some(run) = park.sites.get_mut(&site_id).and_then(|s| s.dungeon.as_mut()) else { return Vec::new() };
        if room >= run.entered.len() || run.entered[room] {
            return Vec::new();
        }
        run.entered[room] = true;
        run.def.rooms[room]
            .waves
            .iter()
            .enumerate()
            .filter(|(i, w)| w.trigger != "cleared" && w.trigger != "killed" && !run.spawned[room][*i])
            .map(|(i, _)| i)
            .collect()
    };
    waves.into_iter().flat_map(|w| spawn_wave(park, site_id, room, w, now)).collect()
}

fn spawn_wave(park: &mut Park, site_id: i64, room: usize, wave: usize, now: i64) -> Vec<i64> {
    let (def, rats, centre) = {
        let Some(run) = park.sites.get_mut(&site_id).and_then(|s| s.dungeon.as_mut()) else { return Vec::new() };
        if run.spawned[room][wave] {
            return Vec::new();
        }
        run.spawned[room][wave] = true;
        (Arc::clone(&run.def), Arc::clone(&run.rats), run.rooms[room])
    };
    // Rated 1/10, the weakest deadspace ships of a class; 10/10, the strongest.
    let strength = (f64::from(def.rating.clamp(1, 10)) - 1.0) / 9.0;
    let mut ids = Vec::new();
    let mut names = Vec::new();
    let mut bosses = Vec::new();
    let last_room = room + 1 == def.rooms.len();
    for (g, ships) in def.rooms[room].waves[wave].ships.iter().enumerate() {
        let angle = wave as f64 * 1.3 + g as f64 * 0.7;
        let distance = 15_000.0 + 4_000.0 * g as f64 + 6_000.0 * wave as f64;
        let at = centre + Vec3::new(angle.cos() * distance, 0.0, angle.sin() * distance);
        for n in 0..ships.count.min(GROUP_CAP) as usize {
            let name = (!ships.names.is_empty()).then(|| ships.names[n % ships.names.len()].as_str());
            let offset = park.rng.in_sphere(6_000.0);
            let pos = at + Vec3::new(offset.x, offset.y * 0.3, offset.z);
            let power = if ships.boss { 1.0 } else { strength };
            let Some(id) = npc::spawn_dungeon_ship(park, &rats, name, class_index(&ships.class), power, site_id, pos, now) else { continue };
            if let (true, Some(label)) = (ships.boss, &ships.label) {
                if let Some(s) = park.ships.get_mut(&id) {
                    s.pilot.name = label.clone();
                }
            }
            if ships.boss {
                bosses.push(id);
            }
            if let Some(n) = name.map(str::to_string).or_else(|| ships.label.clone()) {
                names.push((id, n));
            }
            ids.push(id);
        }
    }
    if let Some(run) = park.sites.get_mut(&site_id).and_then(|s| s.dungeon.as_mut()) {
        for id in &ids {
            run.alive.insert(*id, room);
        }
        run.names.extend(names);
        for id in bosses {
            run.bosses.insert(id, last_room);
        }
    }
    info!("dungeon site {site_id}: room {} wave {} brought {} rat(s)", room + 1, wave + 1, ids.len());
    ids
}

/// One of a dungeon's rats died (`sites::rat_killed`): a cleared room brings
/// its next wave, and the last room cleared completes the dungeon.
pub fn rat_killed(park: &mut Park, site_id: i64, rat: i64, now: i64) -> Option<Progress> {
    let (room, clear, next) = {
        let run = park.sites.get_mut(&site_id)?.dungeon.as_mut()?;
        let Some(room) = run.alive.remove(&rat) else { return Some(Progress::Reinforcements(Vec::new())) };
        let name = run.names.remove(&rat);
        let clear = !run.alive.values().any(|r| *r == room);
        let waves = &run.def.rooms[room].waves;
        let spawned = &run.spawned[room];
        // A trigger ship's death brings the first wave waiting on it, once
        // every wave before that one has come (a chain of triggers).
        let triggered = name.as_ref().and_then(|n| {
            waves.iter().enumerate().find(|(i, w)| {
                !spawned[*i] && w.trigger == "killed" && w.on.iter().any(|o| o == n) && spawned[..*i].iter().all(|s| *s)
            })
        });
        let next = match triggered {
            Some((i, _)) => Some(i),
            None if clear => waves
                .iter()
                .enumerate()
                .find(|(i, w)| w.trigger == "cleared" && !spawned[*i])
                // A room cleared with a trigger never met (its ship came
                // under another name) goes on with the next wave anyway.
                .or_else(|| waves.iter().enumerate().find(|(i, _)| !spawned[*i]))
                .map(|(i, _)| i),
            None => None,
        };
        (room, clear, next)
    };
    if let Some(wave) = next {
        let ids = spawn_wave(park, site_id, room, wave, now);
        if !ids.is_empty() {
            return Some(Progress::Reinforcements(ids));
        }
    }
    let finished = {
        let site = park.sites.get_mut(&site_id)?;
        let (mission_id, char_id) = (site.mission_id, site.char_id);
        let run = site.dungeon.as_mut()?;
        let last = run.rooms.len().saturating_sub(1);
        if clear && room == last && !run.done && run.spawned[last].iter().all(|s| *s) {
            run.done = true;
            info!("dungeon {} ({}) at site {site_id} completed", run.def.name, run.def.id);
            crate::exploration::site_done(site_id);
            (mission_id != 0).then_some((mission_id, char_id))
        } else {
            None
        }
    };
    Some(match finished {
        Some((mission_id, char_id)) => Progress::Cleared { mission_id, char_id },
        None => Progress::Reinforcements(Vec::new()),
    })
}

// ---- DED loot ---------------------------------------------------------------------------------------

/// Each pirate faction's deadspace module prefixes (small, medium and large
/// complexes) and its faction modules' prefix.
const LOOT_NAMES: [(&str, [&str; 3], &str); 5] = [
    ("serpentis", ["Coreli", "Corelum", "Core"], "Shadow Serpentis "),
    ("angel", ["Gistii", "Gistum", "Gist"], "Domination "),
    ("blood", ["Corpii", "Corpum", "Corpus"], "Dark Blood "),
    ("guristas", ["Pithi", "Pithum", "Pith"], "Dread Guristas "),
    ("sansha", ["Centii", "Centum", "Centus"], "True Sansha "),
];
const GROUP_OVERSEER_EFFECTS: i32 = 493;
const CATEGORY_MODULE: i32 = 7;

#[derive(Default)]
struct LootTable {
    /// Overseer's Personal Effects, by tier.
    effects: HashMap<u32, i64>,
    /// Deadspace modules by prefix ("Gistum"): (grade, type).
    deadspace: HashMap<String, Vec<(char, i64)>>,
    /// Faction modules by faction.
    faction: HashMap<&'static str, Vec<i64>>,
}

static LOOT: OnceLock<LootTable> = OnceLock::new();

/// Reads the DED loot: overseers' effects, deadspace (A/B/C/X-Type) and
/// pirate faction modules.
pub async fn load_loot(db: &tokio_postgres::Client) -> Result<(), tokio_postgres::Error> {
    let rows = db
        .query(
            r#"SELECT t."typeID", COALESCE(t."typeName", ''), t."groupID", g."categoryID"
                 FROM "invTypes" t JOIN "invGroups" g ON g."groupID" = t."groupID"
                WHERE t."groupID" = $1
                   OR (g."categoryID" = $2 AND (t."typeName" LIKE '% _-Type %'
                       OR t."typeName" LIKE 'Shadow Serpentis %' OR t."typeName" LIKE 'Domination %'
                       OR t."typeName" LIKE 'Dark Blood %' OR t."typeName" LIKE 'Dread Guristas %'
                       OR t."typeName" LIKE 'True Sansha %'))"#,
            &[&GROUP_OVERSEER_EFFECTS, &CATEGORY_MODULE],
        )
        .await?;
    let mut table = LootTable::default();
    for r in &rows {
        let (type_id, name, group): (i32, String, i32) = (r.get(0), r.get(1), r.get(2));
        let type_id = i64::from(type_id);
        if group == GROUP_OVERSEER_EFFECTS {
            let digits: String = name.chars().take_while(|c| c.is_ascii_digit()).collect();
            if let Ok(tier) = digits.parse::<u32>() {
                table.effects.insert(tier, type_id);
            }
            continue;
        }
        let mut words = name.split(' ');
        if let (Some(prefix), Some(grade)) = (words.next(), words.next()) {
            if grade.len() == 6 && grade.ends_with("-Type") {
                let g = grade.chars().next().unwrap_or('C');
                table.deadspace.entry(prefix.to_string()).or_default().push((g, type_id));
                continue;
            }
        }
        if let Some((faction, _, _)) = LOOT_NAMES.iter().find(|(_, _, p)| name.starts_with(p)) {
            table.faction.entry(faction).or_default().push(type_id);
        }
    }
    info!(
        "DED loot: {} overseer effects tiers, {} deadspace module lines, {} faction modules",
        table.effects.len(),
        table.deadspace.len(),
        table.faction.values().map(Vec::len).sum::<usize>()
    );
    let _ = LOOT.set(table);
    Ok(())
}

/// What an overseer of a complex rated `rating` drops: its Personal Effects,
/// perhaps a deadspace module of the complex's size, and now and then one of
/// its faction's modules. The last room's overseer (or, with none, the
/// clearing of the last room) drops more.
fn roll_ded(faction: &str, rating: u8, last_room: bool, rng: &mut Rng) -> Vec<Stash> {
    let Some(table) = LOOT.get() else { return Vec::new() };
    let Some((key, prefixes, _)) = LOOT_NAMES.iter().find(|(k, _, _)| *k == faction) else { return Vec::new() };
    let r = rating.clamp(1, 10);
    let mut out = Vec::new();
    let tier = ((f64::from(r) * 2.2).round() as u32 + u32::from(last_room)).clamp(1, 23);
    if let Some(t) = (1..=tier).rev().find_map(|t| table.effects.get(&t)) {
        out.push(Stash::New { type_id: *t, quantity: 1 });
    }
    let bonus = if last_room { 2.0 } else { 1.0 };
    if rng.unit() < ((0.15 + 0.035 * f64::from(r)) * bonus).min(0.95) {
        let size = match r {
            1..=3 => 0,
            4..=6 => 1,
            _ => 2,
        };
        if let Some(list) = table.deadspace.get(prefixes[size]).filter(|l| !l.is_empty()) {
            let roll = rng.unit();
            let grade = if r >= 10 && roll < 0.3 {
                'X'
            } else if r >= 7 {
                if roll < 0.5 { 'C' } else if roll < 0.85 { 'B' } else { 'A' }
            } else if roll < 0.6 {
                'C'
            } else if roll < 0.9 {
                'B'
            } else {
                'A'
            };
            let graded: Vec<i64> = list.iter().filter(|(g, _)| *g == grade).map(|(_, t)| *t).collect();
            let pick = if graded.is_empty() { list[rng.below(list.len())].1 } else { graded[rng.below(graded.len())] };
            out.push(Stash::New { type_id: pick, quantity: 1 });
        }
    }
    if rng.unit() < ((0.05 + 0.02 * f64::from(r)) * if last_room { 1.5 } else { 1.0 }).min(0.6) {
        if let Some(list) = table.faction.get(key).filter(|l| !l.is_empty()) {
            out.push(Stash::New { type_id: list[rng.below(list.len())], quantity: 1 });
        }
    }
    out
}

/// A rat of a dungeon died to a player (`fight`): what it drops beyond the
/// usual, if it was an overseer, or if its death completed the complex.
pub fn boss_loot(park: &mut Park, site_id: i64, rat: i64) -> Vec<Stash> {
    let (faction, rating, boss, completion) = {
        let Some(run) = park.sites.get_mut(&site_id).and_then(|s| s.dungeon.as_mut()) else { return Vec::new() };
        let boss = run.bosses.remove(&rat);
        let completion = run.done && !run.paid && !run.bosses.values().any(|last| *last);
        if completion {
            run.paid = true;
        }
        (run.def.faction.clone(), run.def.rating, boss, completion)
    };
    let mut out = Vec::new();
    if let Some(last_room) = boss {
        out.extend(roll_ded(&faction, rating, last_room, &mut park.rng));
    }
    // A complex whose last room has no overseer pays out on its clearing.
    if completion && boss != Some(true) {
        out.extend(roll_ded(&faction, rating, true, &mut park.rng));
    }
    if !out.is_empty() {
        info!("dungeon site {site_id}: {} DED item(s) dropped", out.len());
    }
    out
}

/// A ship uses a gate: where it goes, and the rats waiting there.
pub fn activate_gate(park: &mut Park, gate_id: i64, ship_id: i64, now: i64) -> Result<(Vec3, Vec<i64>), String> {
    let gate = park.sites.get(&gate_id).ok_or("That is not an acceleration gate.")?;
    let (site_id, room) = gate.gate.ok_or("That is not an acceleration gate.")?;
    let (gate_pos, gate_radius) = (gate.pos, gate.radius());
    let ship = park.ships.get(&ship_id).ok_or("Your ship is not in space here.")?;
    let gap = (ship.pos - gate_pos).len() - ship.stats.radius - gate_radius;
    if gap > GATE_REACH {
        return Err(format!("You are {:.0} m from the acceleration gate. Get within {:.0} m to activate it.", gap, GATE_REACH));
    }
    let run = park.sites.get(&site_id).and_then(|s| s.dungeon.as_ref()).ok_or("This acceleration gate leads nowhere.")?;
    let from = room.saturating_sub(1);
    if run.def.rooms[from].gate_locked && run.alive.values().any(|r| *r == from) {
        return Err("The acceleration gate is locked. Destroy this area's defenders first.".into());
    }
    let dest = *run.rooms.get(room).ok_or("This acceleration gate leads nowhere.")?;
    let spawned = enter_room(park, site_id, room, now);
    Ok((dest, spawned))
}

/// keeper ActivateAccelerationGate(gateID): the client's Activate Gate.
pub fn activate(ctx: &Ctx<'_>, gate_id: i64) -> Outcome {
    let (Some(system), Some(ship_id)) = (ctx.session.solar_system_id, ctx.session.ship_id) else {
        return Outcome::custom("You must be in space to use an acceleration gate.");
    };
    let now = crate::combat::now_ms();
    let result = {
        let mut w = ctx.world.lock();
        let outcome = match w.parks.get_mut(&system) {
            None => Err("You are not in space.".to_string()),
            Some(park) => activate_gate(park, gate_id, ship_id, now).map(|(dest, spawned)| {
                let nearby: Vec<(i64, Value)> = spawned.iter().flat_map(|id| space::add_ball_entries_for_nearby(park, *id)).collect();
                (dest, nearby)
            }),
        };
        match outcome {
            Ok((dest, nearby)) => {
                space::notify_targeted_updates(&w, nearby);
                Ok(dest)
            }
            Err(e) => Err(e),
        }
    };
    match result {
        Ok(dest) => space::warp_to_point(ctx, dest, 0.0, 0.0, "ActivateAccelerationGate"),
        Err(why) => Outcome::custom(why),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shipped_complexes_read() {
        let text = include_str!("../../../dungeons/ded.json");
        let list: Vec<Dungeon> = serde_json::from_str(text).expect("dungeons/ded.json is valid");
        assert!(list.len() >= 45);
        assert!(list.iter().all(|d| !d.rooms.is_empty() && d.rooms.iter().all(|r| !r.waves.is_empty())));
    }
}
