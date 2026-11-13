//! Sieges: battles between whole fleets, from a hundred ships a side to
//! well over a thousand.
//!
//! When a siege comes due, each side brings a fleet: its own alliance's
//! armed pilots and, often, allies at war with the other side. How many it
//! commits depends on the stakes (`Stakes`): a border skirmish, a battle, or
//! a war-deciding fight with nearly everything they have.
//!
//! A battle is one fight with two ways to be seen:
//! - Where a player is in the system (in space, or docked with the system's
//!   grid loaded), it is fought ship against ship. The front line is as big
//!   as the system can show (the dashboard's bots-shown-per-system); as ships
//!   die, reinforcements from the rest of the fleet warp in, until one side
//!   is spent or routs. Defenders form up outside their tower's force field;
//!   attackers who clear the field bombard the tower for `BOMBARD`.
//! - Elsewhere, it is fought out second by second with the full numbers:
//!   each side loses ships in proportion to the other's (Lanchester's square
//!   law), and every ship lost is a real killmail.
//!
//! It switches between the two as players come and go. A side reduced to a
//! fifth of what it brought, and outnumbered two to one, withdraws. A fight
//! still going after `MAX_FIGHT` goes to whoever has more ships left.

use std::collections::HashSet;
use std::time::{Duration, Instant};

use tokio_postgres::Client;
use tracing::info;

use super::behaviour::{self, Activity, Atlas, Bot};
use super::politics::Politics;
use super::presence::{Identity, Presence, Rally};
use super::sov::{self, News, Siege};
use crate::combat::Rng;
use crate::world::World;

/// The most ships one side brings to a battle, allies included.
pub const SIDE_MAX: u32 = 3_000;
const FLEET_MIN: u32 = 6;
/// Room left in the system for bots that are not in either fleet.
const ROOM_FOR_OTHERS: usize = 10;
/// Where the fleets form up, beyond the tower's force field.
const DEFENDERS_OUT: f64 = 15_000.0;
const ATTACKERS_IN: f64 = 55_000.0;
const ATTACKERS_ORBIT: f64 = 35_000.0;
const MAX_FIGHT: Duration = Duration::from_secs(45 * 60);
const BOMBARD: Duration = Duration::from_secs(60);
/// Fought unseen, a side loses this share of the other's numbers a second:
/// even fleets lose about half their ships in ten minutes.
const ATTRITION: f64 = 1.0 / 900.0;
/// A side that falls to this share of what it brought, outnumbered two to
/// one, withdraws.
const ROUT: f64 = 0.2;
/// Reinforcements warp in this often, in waves.
const REINFORCE: Duration = Duration::from_secs(8);
/// Allies at war with the other side who may join each side.
const ALLIES_MAX: usize = 3;
const ALLY_CHANCE: f64 = 0.5;

/// How much of themselves the sides commit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stakes {
    /// A border skirmish: a tenth of the fleet or so.
    Skirmish,
    /// A real fight for a system: a third.
    Battle,
    /// Everything they can field.
    Decisive,
}

impl Stakes {
    fn roll(rng: &mut Rng) -> Stakes {
        let r = rng.unit();
        if r < 0.45 {
            Stakes::Skirmish
        } else if r < 0.82 {
            Stakes::Battle
        } else {
            Stakes::Decisive
        }
    }

    fn share(self, rng: &mut Rng) -> f64 {
        let (lo, hi) = match self {
            Stakes::Skirmish => (0.05, 0.15),
            Stakes::Battle => (0.2, 0.45),
            Stakes::Decisive => (0.6, 0.95),
        };
        lo + rng.unit() * (hi - lo)
    }

    pub fn label(self) -> &'static str {
        match self {
            Stakes::Skirmish => "skirmish",
            Stakes::Battle => "battle",
            Stakes::Decisive => "decisive battle",
        }
    }
}

/// One side of a battle.
pub struct Side {
    /// The alliances fighting on it: the first is the siege's own.
    pub alliances: Vec<i32>,
    pub ally_names: Vec<String>,
    /// Its ships on the grid now, as indexes into the bots.
    pub on_grid: Vec<u32>,
    /// Its ships still to come (or fighting unseen), nearest last.
    reserve: Vec<u32>,
    pub brought: u32,
    /// Ships lost so far.
    pub lost: u32,
    /// Unseen losses owed, a fraction of a ship.
    owed: f64,
    /// How well it fights (a rich alliance's ships are better fitted).
    quality: f64,
    pub withdrew: bool,
}

impl Side {
    /// Ships it has left, on the grid and to come.
    pub fn left(&self) -> u32 {
        (self.on_grid.len() + self.reserve.len()) as u32
    }
}

/// A battle being fought.
pub struct Fight {
    pub stakes: Stakes,
    pub attack: Side,
    pub defend: Side,
    /// Whether it is being fought ship against ship, where players see it.
    pub seen: bool,
    began: Instant,
    last: Instant,
    last_wave: Instant,
    /// When the last defender fell, and the bombardment began.
    cleared: Option<Instant>,
    rallies: Option<(Rally, Rally)>,
}

impl Fight {
    /// Ships each side has left.
    pub fn standing(&self) -> (u32, u32) {
        (self.attack.left(), self.defend.left())
    }

    /// Every bot either side has on the grid.
    fn on_grid(&self) -> impl Iterator<Item = u32> + '_ {
        self.attack.on_grid.iter().chain(&self.defend.on_grid).copied()
    }
}

/// What a pass over the sieges (or the defences) did.
#[derive(Default)]
pub struct Report {
    pub news: Vec<News>,
    /// Bots called up or sent home, whose state changed.
    pub touched: Vec<u32>,
    /// Bots sent home, whose ships should set off.
    pub released: Vec<u32>,
    /// Players who fired on a bot side's tower: (side, character).
    pub aggressors: Vec<(i32, i64)>,
}

impl Report {
    pub fn merge(&mut self, other: Report) {
        self.news.extend(other.news);
        self.touched.extend(other.touched);
        self.released.extend(other.released);
        self.aggressors.extend(other.aggressors);
    }
}

/// Whether a battle in `system` can be seen: a player in space there, or
/// one docked there with the system's grid loaded.
fn seen_in(world: &World, presence: &Presence, system: i32) -> bool {
    if presence.watched().contains(&system) {
        return true;
    }
    let w = world.lock();
    let docked = w.parks.contains_key(&(system as i64)) && w.local_players(system as i64).values().any(|p| p.docked);
    docked
}

/// Starts the sieges that are due, and fights the ones under way.
#[allow(clippy::too_many_arguments)]
pub async fn run(
    sieges: &mut Vec<Siege>,
    world: &World,
    db: &mut Client,
    atlas: &Atlas,
    bots: &mut [Bot],
    ids: &[Identity],
    presence: &mut Presence,
    p: &mut Politics,
    rng: &mut Rng,
    clock: u32,
    claim_delay_secs: i64,
    speed: f64,
) -> Result<Report, tokio_postgres::Error> {
    let mut report = Report::default();
    let mut kept = Vec::new();
    let mut seen_systems = HashSet::new();
    for mut s in std::mem::take(sieges) {
        if s.fight.is_none() {
            if clock < s.starts {
                kept.push(s);
                continue;
            }
            let Some(mut f) = assemble(&s, bots, ids, presence, p, rng, &mut report) else {
                // One side has nobody who can fight: the other walks in.
                let won = s.attackers >= s.defenders;
                report.news.push(sov::conclude(db, p, &s, won, (0, 0), rng, claim_delay_secs, speed).await?);
                sov::show_towers(db, world, s.system).await;
                continue;
            };
            for side in [&mut f.attack, &mut f.defend] {
                for a in side.alliances.iter().skip(1) {
                    side.ally_names.push(sov::alliance_name(db, *a).await);
                }
            }
            let allies = |side: &Side| if side.ally_names.is_empty() { String::new() } else { format!(" with {}", side.ally_names.join(", ")) };
            let now = crate::bulkdata::filetime_now();
            report.news.push((
                now,
                format!(
                    "{} in {}: {} ships of {}{} against {} of {}{}",
                    capitalise(f.stakes.label()),
                    s.names.system,
                    f.attack.brought,
                    s.names.attacker,
                    allies(&f.attack),
                    f.defend.brought,
                    s.names.defender,
                    allies(&f.defend)
                ),
            ));
            info!("bots: {} in {}: {} against {}", f.stakes.label(), s.names.system, f.attack.brought, f.defend.brought);
            s.fight = Some(f);
        }
        let seen = seen_in(world, presence, s.system);
        let verdict = {
            let f = s.fight.as_mut().expect("a fight under way");
            if seen && !f.seen {
                f.seen = muster(&s.system, s.defender, world, f, bots, presence, rng, clock, &mut report);
            } else if !seen && f.seen {
                stand_down(f, bots, atlas, presence, rng, &mut report);
            }
            if f.seen {
                seen_systems.insert(s.system);
                fight_seen(f, &s.system, bots, presence, clock, &mut report)
            } else {
                fight_unseen(f, s.system, atlas, bots, p, rng, &mut report)
            }
        };
        let Some(won) = verdict else {
            kept.push(s);
            continue;
        };
        let f = s.fight.take().expect("a fight under way");
        let lost = (f.attack.lost, f.defend.lost);
        let mut news = sov::conclude(db, p, &s, won, lost, rng, claim_delay_secs, speed).await?;
        news.1 = format!("{} ({} ships lost: {} of {}'s side, {} of {}'s)", news.1, lost.0 + lost.1, lost.0, s.names.attacker, lost.1, s.names.defender);
        report.news.push(news);
        sov::show_towers(db, world, s.system).await;
        for i in f.on_grid().collect::<Vec<_>>() {
            presence.dismiss(i);
            let bot = &mut bots[i as usize];
            if bot.activity == Activity::Sieging {
                behaviour::release(bot, atlas, rng);
                report.released.push(i);
            }
            report.touched.push(i);
        }
    }
    presence.set_battles(seen_systems);
    *sieges = kept;
    Ok(report)
}

fn capitalise(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}

/// Both fleets for a siege: each side's alliance, allies who join, and how
/// many ships they commit. None if a side has nobody to send.
fn assemble(s: &Siege, bots: &[Bot], ids: &[Identity], presence: &Presence, p: &Politics, rng: &mut Rng, report: &mut Report) -> Option<Fight> {
    let _ = report;
    let stakes = Stakes::roll(rng);
    let wealth = |side: i32| 1.0 + (p.treasury.get(&side).copied().unwrap_or(0.0) / 5_000_000_000.0).clamp(0.0, 1.0);
    // Allies: others at war with the enemy, some of whom come.
    let allies_of = |me: i32, enemy: i32, rng: &mut Rng| -> Vec<i32> {
        let mut out: Vec<i32> = p
            .wars
            .iter()
            .filter_map(|w| if w.a == enemy { Some(w.b) } else if w.b == enemy { Some(w.a) } else { None })
            .filter(|a| *a != me && p.alliances.contains_key(a))
            .collect();
        out.sort_unstable();
        out.dedup();
        out.retain(|_| rng.unit() < ALLY_CHANCE);
        out.truncate(ALLIES_MAX.min(super::sov_warfare().max_allies()));
        out
    };
    let attack_allies = allies_of(s.attacker, s.defender, rng);
    let defend_allies = allies_of(s.defender, s.attacker, rng);
    let side = |main: i32, allies: &[i32], home: bool, rng: &mut Rng| -> Option<Side> {
        let alliances: Vec<i32> = std::iter::once(main).chain(allies.iter().copied()).collect();
        let members: HashSet<i32> = alliances.iter().copied().collect();
        let mut fit: Vec<u32> = (0..bots.len() as u32)
            .filter(|i| {
                let (b, id) = (&bots[*i as usize], &ids[*i as usize]);
                id.alliance.is_some_and(|a| members.contains(&a)) && b.pod.is_none() && b.activity != Activity::Sieging && presence.armed(id.hull)
            })
            .collect();
        if fit.is_empty() {
            return None;
        }
        // The defenders turn out for their own space a little more.
        let share = (stakes.share(rng) + if home { 0.1 } else { 0.0 }).min(1.0);
        let brought = ((fit.len() as f64 * share) as u32).clamp(FLEET_MIN.min(fit.len() as u32), (super::battle_size() as u32).clamp(FLEET_MIN, SIDE_MAX));
        // The nearest come; the reserve is kept nearest last, to call up in turn.
        fit.sort_by_key(|i| std::cmp::Reverse(nearness(&bots[*i as usize], s.system)));
        let reserve = fit.split_off(fit.len() - brought as usize);
        Some(Side { alliances, ally_names: Vec::new(), on_grid: Vec::new(), reserve, brought, lost: 0, owed: 0.0, quality: wealth(main), withdrew: false })
    };
    let attack = side(s.attacker, &attack_allies, false, rng)?;
    let defend = side(s.defender, &defend_allies, true, rng)?;
    let now = Instant::now();
    Some(Fight { stakes, attack, defend, seen: false, began: now, last: now, last_wave: now - REINFORCE, cleared: None, rallies: None })
}

/// The fight comes into view: rally points round the defender's tower. False
/// if there is no tower left to fight round (it is fought unseen).
#[allow(clippy::too_many_arguments)]
fn muster(system: &i32, defender: i32, world: &World, f: &mut Fight, bots: &mut [Bot], presence: &mut Presence, rng: &mut Rng, clock: u32, report: &mut Report) -> bool {
    let place = {
        let w = world.lock();
        let found = w.parks.get(&(*system as i64)).and_then(|park| {
            park.structures.values().find(|t| t.is_tower() && t.alliance_id == Some(defender as i64)).map(|t| (t.id, t.pos, t.field_radius.max(t.radius)))
        });
        found
    };
    let Some((tower, at, field)) = place else { return false };
    let dir = rng.in_sphere(1.0);
    let dir = if dir.len() > 1e-6 { dir.unit() } else { crate::ballpark::Vec3::new(1.0, 0.0, 0.0) };
    let defend = Rally { anchor: tower, point: at + dir * (field + DEFENDERS_OUT), orbit: field + DEFENDERS_OUT, target: None };
    let attack = Rally { anchor: tower, point: at + dir * (field + ATTACKERS_IN), orbit: field + ATTACKERS_ORBIT, target: None };
    f.rallies = Some((attack, defend));
    f.last_wave = Instant::now() - REINFORCE;
    reinforce(f, *system, bots, presence, clock, report);
    true
}

/// Both sides' reserves warp in, as far as the grid has room.
fn reinforce(f: &mut Fight, system: i32, bots: &mut [Bot], presence: &mut Presence, clock: u32, report: &mut Report) {
    if f.last_wave.elapsed() < REINFORCE {
        return;
    }
    f.last_wave = Instant::now();
    let Some((attack, defend)) = f.rallies else { return };
    let room = (presence.cap().saturating_sub(ROOM_FOR_OTHERS) / 2).max(1);
    for (side, rally) in [(&mut f.attack, attack), (&mut f.defend, defend)] {
        if side.withdrew {
            continue;
        }
        while side.on_grid.len() < room {
            let Some(i) = side.reserve.pop() else { break };
            if bots[i as usize].pod.is_some() || bots[i as usize].activity == Activity::Sieging {
                continue;
            }
            enlist(bots, presence, i, system, rally, clock);
            side.on_grid.push(i);
            report.touched.push(i);
        }
    }
}

/// The fight goes out of view: the ships on the grid go back to the
/// reserves, to fight on unseen.
fn stand_down(f: &mut Fight, bots: &mut [Bot], atlas: &Atlas, presence: &mut Presence, rng: &mut Rng, report: &mut Report) {
    for side in [&mut f.attack, &mut f.defend] {
        for i in std::mem::take(&mut side.on_grid) {
            presence.dismiss(i);
            let bot = &mut bots[i as usize];
            if bot.activity == Activity::Sieging {
                behaviour::release(bot, atlas, rng);
                report.released.push(i);
            }
            report.touched.push(i);
            if bot.pod.is_none() {
                side.reserve.push(i);
            }
        }
    }
    f.seen = false;
    f.rallies = None;
}

/// A second of a fight players can see: losses counted, reinforcements
/// called, and the verdict once there is one (true: the attackers won).
fn fight_seen(f: &mut Fight, system: &i32, bots: &mut [Bot], presence: &mut Presence, clock: u32, report: &mut Report) -> Option<bool> {
    for side in [&mut f.attack, &mut f.defend] {
        let mut lost = 0;
        side.on_grid.retain(|i| {
            let b = &bots[*i as usize];
            if b.pod.is_some() {
                lost += 1;
                return false;
            }
            b.activity == Activity::Sieging && b.system == *system
        });
        side.lost += lost;
    }
    rout(f);
    reinforce(f, *system, bots, presence, clock, report);
    verdict(f, true)
}

/// A second of a fight nobody sees: each side loses ships in proportion to
/// the other's, every one a killmail.
fn fight_unseen(f: &mut Fight, system: i32, atlas: &Atlas, bots: &mut [Bot], p: &mut Politics, rng: &mut Rng, report: &mut Report) -> Option<bool> {
    let dt = f.last.elapsed().as_secs_f64().min(5.0);
    f.last = Instant::now();
    let (a, d) = (f.attack.left() as f64, f.defend.left() as f64);
    let noise = |rng: &mut Rng| 0.7 + rng.unit() * 0.6;
    if !f.attack.withdrew && !f.defend.withdrew {
        f.attack.owed += ATTRITION * d * f.defend.quality / f.attack.quality.max(0.1) * dt * noise(rng);
        f.defend.owed += ATTRITION * a * f.attack.quality / f.defend.quality.max(0.1) * dt * noise(rng);
    }
    for (victims, killers) in [(0usize, 1usize), (1, 0)] {
        let whole = {
            let side = if victims == 0 { &mut f.attack } else { &mut f.defend };
            let whole = side.owed.floor() as u32;
            side.owed -= whole as f64;
            whole
        };
        for _ in 0..whole {
            let (victim_side, killer_side) = if victims == 0 { (&mut f.attack, &f.defend) } else { (&mut f.defend, &f.attack) };
            if victim_side.reserve.is_empty() {
                break;
            }
            let v = victim_side.reserve.swap_remove(rng.below(victim_side.reserve.len()));
            victim_side.lost += 1;
            let killer = if killer_side.reserve.is_empty() { killer_side.on_grid.first().copied() } else { Some(killer_side.reserve[rng.below(killer_side.reserve.len())]) };
            if let Some(k) = killer {
                p.unseen_kills.push((v, k, system, bots[v as usize].hull));
            }
            p.lost += 1;
            let bot = &mut bots[v as usize];
            if bot.activity != Activity::Sieging {
                let _ = behaviour::reclone(bot, atlas, rng);
                report.touched.push(v);
            }
        }
        let _ = killers;
    }
    rout(f);
    verdict(f, false)
}

/// A side worn down to a fifth and outnumbered two to one withdraws.
fn rout(f: &mut Fight) {
    let (a, d) = f.standing();
    if !f.attack.withdrew && a as f64 <= f.attack.brought as f64 * ROUT && d >= 2 * a.max(1) {
        f.attack.withdrew = true;
    }
    if !f.defend.withdrew && d as f64 <= f.defend.brought as f64 * ROUT && a >= 2 * d.max(1) {
        f.defend.withdrew = true;
    }
}

/// Who has won, if anyone yet: true for the attackers.
fn verdict(f: &mut Fight, seen: bool) -> Option<bool> {
    let (a, d) = f.standing();
    if f.attack.withdrew || a == 0 {
        return Some(false);
    }
    if f.defend.withdrew || d == 0 {
        if !seen {
            return Some(true);
        }
        // The field is clear: the tower is bombarded before it falls.
        let since = *f.cleared.get_or_insert_with(Instant::now);
        return (since.elapsed() >= BOMBARD).then_some(true);
    }
    if f.began.elapsed() >= MAX_FIGHT {
        return Some(a > d);
    }
    None
}

/// How near a bot is to answer a call to `system`: 0 in space there, 1
/// docked there, 2 playing elsewhere, 3 not playing.
pub fn nearness(b: &Bot, system: i32) -> u8 {
    match (b.online, b.system == system, b.station == 0) {
        (true, true, true) => 0,
        (true, true, false) => 1,
        (true, false, _) => 2,
        (false, _, _) => 3,
    }
}

/// Up to `n` bots for a fleet to `system`: armed, in their ships, not in
/// another fleet, no further off than `furthest` (`nearness`), and on the
/// side `member` says; the nearest first.
#[allow(clippy::too_many_arguments)]
pub fn pick(bots: &[Bot], ids: &[Identity], presence: &Presence, n: usize, system: i32, furthest: u8, member: impl Fn(&Identity) -> bool) -> Vec<u32> {
    let mut fit: Vec<u32> = (0..bots.len() as u32)
        .filter(|i| {
            let (b, id) = (&bots[*i as usize], &ids[*i as usize]);
            member(id) && b.pod.is_none() && b.activity != Activity::Sieging && presence.armed(id.hull) && nearness(b, system) <= furthest
        })
        .collect();
    fit.sort_by_key(|i| nearness(&bots[*i as usize], system));
    fit.truncate(n);
    fit
}

/// Calls a bot up to a fleet in `system`: it comes out of its station if
/// it is docked there.
pub fn enlist(bots: &mut [Bot], presence: &mut Presence, i: u32, system: i32, rally: Rally, clock: u32) {
    let b = &mut bots[i as usize];
    let docked_here = (b.online && b.system == system && b.station != 0).then_some(b.station);
    behaviour::muster(b, system, clock);
    presence.muster(i, rally, docked_here);
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::ballpark::{Celestial, Park, Pilot, Ship, ShipStats, Vec3};
    use crate::combat::attr;
    use crate::npc::NpcType;
    use crate::pos::Structure;

    const HERE: i32 = 30_000_001;
    const ATTACKER: i32 = 99_000_001;
    const DEFENDER: i32 = 99_000_002;
    const TOWER: i64 = 1_000_000_001;

    fn tower() -> Structure {
        Structure {
            id: TOWER,
            type_id: 12235,
            group_id: 365,
            name: String::new(),
            owner_id: 98_000_002,
            corp_id: 98_000_002,
            alliance_id: Some(DEFENDER as i64),
            pos: Vec3::ZERO,
            radius: 3_000.0,
            state: crate::pos::state::ONLINE,
            stamp: None,
            tower: None,
            delays: [0; 3],
            vitals: crate::pos::vitals(None, 0),
            signature: 0.0,
            last_attacker: None,
            reinforce_asked: false,
            field_radius: 30_000.0,
            disruption_radius: 0.0,
            harmonic: -1,
            allow_corp: true,
            allow_alliance: true,
            field: None,
            sentry: Default::default(),
            guard: None,
        }
    }

    /// A system with a tower, a gate near it, and a player watching.
    fn world() -> World {
        let world = World::default();
        let gate = Celestial { id: 50_000_001, type_id: 0, group_id: 10, name: String::new(), pos: Vec3::new(60_000.0, 0.0, 0.0), radius: 1_000.0, undock: None, jump: Some((30_000_002, 50_000_002)) };
        let mut park = Park::new(HERE as i64, vec![gate], 0);
        park.structures.insert(TOWER, tower());
        let pilot = Pilot { type_id: 587, group_id: 25, name: "Player".into(), char_id: 90, corp_id: 1, alliance_id: None, security: 0.0, bounty: 0.0 };
        park.ships.insert(900, Ship::new(900, 77, ShipStats::new(30.0, 1e6, 1.0, 300.0, 1.0), pilot, Vec3::new(0.0, 1.0e7, 0.0), Vec3::ZERO));
        world.lock().parks.insert(HERE as i64, park);
        world
    }

    fn frigate() -> NpcType {
        NpcType::with(
            587,
            25,
            30.0,
            1.0e6,
            &[
                (attr::MAX_VELOCITY, 350.0),
                (attr::AGILITY, 3.0),
                (attr::HP, 300.0),
                (attr::ARMOR_HP, 300.0),
                (attr::SHIELD_CAPACITY, 300.0),
                (attr::SCAN_RESOLUTION, 600.0),
                (attr::SIGNATURE_RADIUS, 35.0),
            ],
        )
    }

    #[test]
    fn a_watched_siege_is_fought_ship_against_ship() {
        crate::svc::war::in_force_for_test(ATTACKER as i64, DEFENDER as i64);
        let world = world();
        let capsule = NpcType::with(super::super::presence::TYPE_CAPSULE, 29, 25.0, 32_000.0, &[(attr::MAX_VELOCITY, 1_000.0), (attr::AGILITY, 1.0), (attr::HP, 200.0)]);
        let mut presence = Presence::new(HashMap::from([(587, frigate()), (super::super::presence::TYPE_CAPSULE, capsule)]), 120);
        let (mut bots, mut ids) = (Vec::new(), Vec::new());
        for n in 0..12u32 {
            let side = if n < 6 { ATTACKER } else { DEFENDER };
            bots.push(Bot {
                id: 140_600_000 + n as i32, role: behaviour::Role::Ratter, home: 60_000_001, system: 30_000_009, station: 60_000_001, online: true,
                share: 0.3, session_end: u32::MAX, activity: Activity::Docked, route: Vec::new(), dock_at: 0, hold: Vec::new(), isk: 0.0, isk_saved: 0.0, earned: 0.0, pod: None, turf: 0, hull: 587, tier: 0, hull_lost: false, prime: 19,
            });
            ids.push(Identity { name: format!("Pilot {n}"), corp: side - 1_000_000, security: 0.0, ship: 140_700_000 + n as i64, hull: 587, alliance: Some(side) });
        }
        let by_char: HashMap<i64, u32> = bots.iter().enumerate().map(|(i, b)| (b.id as i64, i as u32)).collect();
        static GRAPH: std::sync::OnceLock<crate::gates::Graph> = std::sync::OnceLock::new();
        let atlas = Atlas { systems: HashMap::new(), station_system: HashMap::from([(60_000_001, 30_000_009)]), hubs: Vec::new(), belts_near: HashMap::new(), graph: GRAPH.get_or_init(HashMap::new) };
        let mut rng = Rng::seeded(3);

        // Somebody is watching, so the fleets are called up.
        presence.sync(&world, &atlas, &mut bots, &ids, &by_char, Vec::new(), &mut rng);
        let names = sov::SiegeNames { attacker: "A".into(), defender: "D".into(), system: "Here".into(), region: "R".into(), security: -0.3 };
        let siege = Siege { war: 1, attacker: ATTACKER, defender: DEFENDER, attacker_exec: 1, attacker_race: 1, attackers: 300, defenders: 300, system: HERE, names, starts: 0, starts_at: 0, fight: None };
        let politics = Politics::new(&atlas);
        let mut report = Report::default();
        let mut fight = assemble(&siege, &bots, &ids, &presence, &politics, &mut rng, &mut report).expect("both sides have armed ships");
        assert_eq!((fight.attack.brought, fight.defend.brought), (6, 6), "small alliances bring everyone");
        assert!(muster(&HERE, DEFENDER, &world, &mut fight, &mut bots, &mut presence, &mut rng, 0, &mut report), "fought round the tower");
        assert_eq!((fight.attack.on_grid.len(), fight.defend.on_grid.len()), (6, 6));
        presence.sync(&world, &atlas, &mut bots, &ids, &by_char, Vec::new(), &mut rng);
        assert_eq!(presence.shown(), 12, "both fleets are in space");

        // They fight: ships are lost on both sides' account.
        let mut now = crate::combat::now_ms();
        let mut verdict = None;
        for _ in 0..600 {
            now += 1_000;
            let mut w = world.lock();
            let park = w.parks.get_mut(&(HERE as i64)).unwrap();
            let tick = park.tick + 1;
            park.advance(tick);
            crate::fight::step(park, now, &mut crate::heartbeat::Out::default());
            drop(w);
            presence.sync(&world, &atlas, &mut bots, &ids, &by_char, Vec::new(), &mut rng);
            fight.cleared = fight.cleared.map(|_| Instant::now() - BOMBARD);
            verdict = fight_seen(&mut fight, &HERE, &mut bots, &mut presence, 0, &mut report);
            if verdict.is_some() {
                break;
            }
        }
        let (a, d) = fight.standing();
        assert!(fight.attack.lost + fight.defend.lost >= 2, "ships were destroyed: standing {a} and {d}");
        assert!(verdict.is_some() || a == 0 || d == 0, "the battle was decided: {a} and {d}");
    }
}
