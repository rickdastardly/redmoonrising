//! Mission sites: a beacon where an agent's encounter waits, and the rats
//! sent to it. A site opens when its pilot first heads there and lives in
//! the system's park; when the last of its rats dies the mission's
//! objective is done (`Job::SiteCleared`).

use eve_marshal::Value;

use crate::ballpark::{Park, Vec3};
use crate::destiny::Ball;
use crate::svc::space::{self, entry};

/// Celestial Agent Site Beacon.
pub const TYPE_BEACON: i64 = 25354;
const BEACON_RADIUS: f64 = 1_000.0;
const GATE_RADIUS: f64 = 1_500.0;
const OWNER_SYSTEM: i64 = 1;

#[derive(Debug, Clone)]
pub struct Site {
    pub id: i64,
    pub mission_id: i64,
    pub char_id: i64,
    pub name: String,
    pub pos: Vec3,
    /// The rats still alive.
    pub rats: Vec<i64>,
    /// The current wave, counting from one.
    pub wave: usize,
    pub waves: usize,
    pub security: f64,
    /// What it shows as: the site beacon, or a dungeon's acceleration gate.
    pub type_id: i64,
    /// A dungeon at this site (`dungeons`): its rooms, gates and waves.
    pub dungeon: Option<Box<crate::dungeons::Run>>,
    /// For an acceleration gate: its dungeon's site and the room it leads to.
    pub gate: Option<(i64, usize)>,
    /// For a dungeon's scenery (a bunker, a wall, a cloud): its dungeon's site.
    pub part_of: Option<i64>,
    /// Its size, when not a beacon's or a gate's.
    pub size: f64,
}

pub enum Progress {
    Reinforcements(Vec<i64>),
    Cleared { mission_id: i64, char_id: i64 },
}

impl Site {
    pub fn ball(&self) -> Ball {
        Ball::celestial(self.id, self.radius(), self.pos.tuple(), &self.name, false)
    }

    pub fn slim(&self) -> Value {
        space::slim(vec![
            ("itemID", Value::Int(self.id)),
            ("typeID", Value::Int(self.type_id)),
            ("ownerID", Value::Int(OWNER_SYSTEM)),
            ("name", Value::text(&self.name)),
        ])
    }

    pub fn radius(&self) -> f64 {
        if self.size > 0.0 {
            self.size
        } else if self.gate.is_some() {
            GATE_RADIUS
        } else {
            BEACON_RADIUS
        }
    }
}

/// Opens a site with rats of a strength for `security` around it and returns
/// the beacon and rat entries for pilots close enough to see them.
pub fn open(park: &mut Park, mut site: Site, security: f64, now: i64) -> Vec<(i64, Value)> {
    site.wave = 1;
    site.security = security;
    site.rats = crate::npc::spawn_group(park, site.id, site.pos, security, now);
    let mut nearby = Vec::new();
    for id in &site.rats {
        nearby.extend(space::add_ball_entries_for_nearby(park, *id));
    }
    tracing::info!("mission {} site {} opened wave 1/{} with {} rat(s)", site.mission_id, site.id, site.waves, site.rats.len());
    let site_id = site.id;
    park.sites.insert(site_id, site);
    nearby.extend(space::add_object_entries_for_nearby(park, site_id));
    nearby
}

/// Closes a mission's site, and sends off any rats left there.
pub fn close(park: &mut Park, mission_id: i64) -> Vec<Value> {
    let Some(id) = park.sites.values().find(|s| s.mission_id == mission_id && s.gate.is_none() && s.part_of.is_none()).map(|s| s.id) else { return Vec::new() };
    close_site(park, id)
}

/// Closes a site: its beacon, a dungeon's gates and scenery, and the rats left there.
pub fn close_site(park: &mut Park, id: i64) -> Vec<Value> {
    let Some(site) = park.sites.remove(&id) else { return Vec::new() };
    let mut gone = vec![Value::Int(id)];
    let mut rats = site.rats;
    if let Some(run) = &site.dungeon {
        rats.extend(run.alive.keys().copied());
    }
    for rat in rats {
        if park.ships.remove(&rat).is_some() {
            gone.push(Value::Int(rat));
        }
    }
    let gates: Vec<i64> = park
        .sites
        .values()
        .filter(|s| s.gate.is_some_and(|(of, _)| of == id) || s.part_of == Some(id))
        .map(|s| s.id)
        .collect();
    for gate in gates {
        park.sites.remove(&gate);
        gone.push(Value::Int(gate));
    }
    vec![entry(park.tick, "RemoveBalls", vec![Value::List(gone)])]
}

/// A cleared wave brings reinforcements until the encounter's last wave.
pub fn rat_killed(park: &mut Park, site_id: i64, rat: i64, now: i64) -> Option<Progress> {
    if park.sites.get(&site_id).is_some_and(|s| s.dungeon.is_some()) {
        return crate::dungeons::rat_killed(park, site_id, rat, now);
    }
    let site = park.sites.get_mut(&site_id)?;
    site.rats.retain(|r| *r != rat);
    if !site.rats.is_empty() {
        return Some(Progress::Reinforcements(Vec::new()));
    }
    if site.wave >= site.waves {
        return Some(Progress::Cleared { mission_id: site.mission_id, char_id: site.char_id });
    }
    site.wave += 1;
    let (anchor, pos, security, wave, waves, mission_id, char_id) =
        (site.id, site.pos, site.security, site.wave, site.waves, site.mission_id, site.char_id);
    let rats = crate::npc::spawn_group(park, anchor, pos, security, now);
    let site = park.sites.get_mut(&site_id).expect("site still exists");
    site.rats = rats.clone();
    tracing::info!("mission {mission_id} site {site_id} wave {wave}/{waves} brought {} rats", rats.len());
    if rats.is_empty() {
        Some(Progress::Cleared { mission_id, char_id })
    } else {
        Some(Progress::Reinforcements(rats))
    }
}
