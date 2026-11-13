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

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use tokio_postgres::Client;
use tracing::{info, warn};

use super::behaviour::{self, Activity, Atlas, Bot};
use super::capitals::{self, Assigned, Class};
use crate::ballpark::{BattleOrder, Vec3};
use super::politics::Politics;
use super::presence::{Identity, Presence, Rally};
use super::sov::{self, News, Siege};
use crate::combat::Rng;
use crate::world::World;

/// The most ships one side brings to a battle, allies included.
pub const SIDE_MAX: u32 = 10_000;
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
const REINFORCE: Duration = Duration::from_secs(5);
/// The most ships one side warps onto the grid in a wave: hundreds landing
/// in the same second stall a watching client for half a minute (it loads
/// every new ship at once), and its ship seems to jump about as it catches
/// up. A wave every few seconds fills the grid smoothly, with no limit on
/// how many come in the end.
const WAVE_MOST: usize = 25;
/// Capitals jump through a cyno in waves of up to this many.
const JUMP_WAVE_MOST: usize = 50;
/// Fought where players see it: the subcapital fleets fight this long
/// before the first side lights its cyno and its capitals jump in ...
const CYNO_FIRST: Duration = Duration::from_secs(60);
/// ... and the other side lights its own this long after they land.
const CYNO_ANSWER: Duration = Duration::from_secs(40);
/// Should the first side never get its capitals in, the other lights its
/// cyno anyway this much later.
const CYNO_LATE: Duration = Duration::from_secs(90);
/// Allies at war with the other side who may join each side.
const ALLIES_MAX: usize = 3;
const ALLY_CHANCE: f64 = 0.5;
/// How often, fought where players see it, the capitals act: sieged
/// dreadnoughts fire on the tower, logistics repair, titans look to fire.
const ORDERS_EVERY: Duration = Duration::from_secs(5);
/// How often a fight players see says, in the server log, what its capitals
/// are doing.
const ORDERS_LOG_EVERY: Duration = Duration::from_secs(30);
/// A sieged dreadnought's volley at an enemy ship, where there is no tower
/// to bombard: capitals first.
const VOLLEY: f64 = 12_000.0;
/// Where an open-space battle forms up: a beacon this far from the player
/// who watches, the fleets round it.
const FIELD_OFF: f64 = 45_000.0;
const FIELD_DEFEND_ORBIT: f64 = 12_000.0;
const FIELD_ATTACK_ORBIT: f64 = 28_000.0;
/// How far a sieged dreadnought reaches with a volley at a ship.
const VOLLEY_RANGE: f64 = 150_000.0;
/// Titan, dreadnought, freighter, carrier and mothership groups.
const CAPITAL_GROUPS: [i64; 5] = [30, 485, 513, 547, 659];
/// Celestial Beacon II: marks an open-space battlefield.
const TYPE_BATTLE_BEACON: i64 = 19706;
/// A sieged dreadnought's damage to a tower in that time.
const BOMBARD_VOLLEY: f64 = 9_000.0;
/// A logistics ship's repair in that time.
const REPAIR_CYCLE: f64 = 700.0;
/// A titan's doomsday recharges this long.
const DOOMSDAY_EVERY: Duration = Duration::from_secs(600);
/// A titan fires its first doomsday this long into the fight.
const DOOMSDAY_FIRST: Duration = Duration::from_secs(90);
/// Fought unseen, a doomsday takes this share of the enemy's subcapitals,
/// and at most this many.
const DOOMSDAY_SHARE: f64 = 0.05;
const DOOMSDAY_MOST: usize = 60;
/// With no cyno ship on the grid this long, any ship of the side lights one.
const CYNO_WAIT: Duration = Duration::from_secs(45);
/// Cynosural Field I, the beacon capitals jump to.
const TYPE_CYNO_FIELD: i64 = 21094;
/// How often the battle report is brought up to date.
const REPORT_EVERY: Duration = Duration::from_secs(60);

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

    /// How many ships one side brings to a battle asked for by hand: from
    /// two hundred to five hundred, capitals on top of their share.
    fn showcase_fleet(self, rng: &mut Rng) -> u32 {
        let (lo, hi) = match self {
            Stakes::Skirmish => (200, 280),
            Stakes::Battle => (250, 400),
            Stakes::Decisive => (350, 500),
        };
        lo + rng.below((hi - lo + 1) as usize) as u32
    }

    fn share(self, rng: &mut Rng) -> f64 {
        let (lo, hi) = match self {
            Stakes::Skirmish => (0.05, 0.15),
            Stakes::Battle => (0.2, 0.45),
            Stakes::Decisive => (0.6, 0.95),
        };
        lo + rng.unit() * (hi - lo)
    }

    /// "skirmish", "battle" or "decisive".
    pub fn parse(word: &str) -> Option<Stakes> {
        match word.trim().to_ascii_lowercase().as_str() {
            "skirmish" => Some(Stakes::Skirmish),
            "battle" => Some(Stakes::Battle),
            "decisive" => Some(Stakes::Decisive),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Stakes::Skirmish => "skirmish",
            Stakes::Battle => "battle",
            Stakes::Decisive => "decisive battle",
        }
    }
}

/// What a seen fight's capitals did lately, for the server log.
#[derive(Default)]
struct Tally {
    bombard: u32,
    volleys: u32,
    repairs: u32,
    doomsdays: u32,
    sieged: u32,
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
    /// Pilots flying capitals, logistics and cyno ships for this battle.
    pub caps: Vec<Assigned>,
    /// Capitals waiting for a cynosural field to jump to.
    waiting: Vec<u32>,
    /// Its cynosural field, once lit: the field and where it is.
    cyno: Option<(i64, Vec3)>,
    /// Since when its capitals have waited for a cyno.
    waiting_since: Option<Instant>,
    /// When its first capitals jumped in.
    landed: Option<Instant>,
    /// Dreadnoughts in siege mode.
    sieged: HashSet<u32>,
    /// When each titan last fired its doomsday.
    doomsday: HashMap<u32, Instant>,
    /// Capitals (and logistics) lost, by class.
    pub caps_lost: HashMap<Class, u32>,
}

impl Side {
    /// Ships it has left, on the grid and to come.
    pub fn left(&self) -> u32 {
        (self.on_grid.len() + self.reserve.len() + self.waiting.len()) as u32
    }

    fn new(alliances: Vec<i32>, reserve: Vec<u32>, brought: u32, quality: f64) -> Side {
        Side {
            alliances,
            ally_names: Vec::new(),
            on_grid: Vec::new(),
            reserve,
            brought,
            lost: 0,
            owed: 0.0,
            quality,
            withdrew: false,
            caps: Vec::new(),
            waiting: Vec::new(),
            cyno: None,
            waiting_since: None,
            landed: None,
            sieged: HashSet::new(),
            doomsday: HashMap::new(),
            caps_lost: HashMap::new(),
        }
    }

    /// What a pilot flies for this battle, if it is special.
    fn assigned(&self, i: u32) -> Option<Assigned> {
        self.caps.iter().find(|a| a.bot == i).copied()
    }

    /// Still in the fight: on the grid, to come, or waiting for a cyno.
    fn has(&self, i: u32) -> bool {
        self.on_grid.contains(&i) || self.reserve.contains(&i) || self.waiting.contains(&i)
    }

    /// The side's strength fought unseen: capitals count for many ships.
    fn strength(&self) -> f64 {
        let extra: f64 = self.caps.iter().filter(|a| self.has(a.bot)).map(|a| a.class.weight() - 1.0).sum();
        self.left() as f64 + extra
    }

    /// How much its logistics cut its losses fought unseen.
    fn repairs(&self) -> f64 {
        let logi = self.caps.iter().filter(|a| a.class == Class::Logistics && self.has(a.bot)).count() as f64;
        (logi / self.left().max(1) as f64 * 3.0).min(0.35)
    }

    /// What its capitals were: "2 titans, 14 dreadnoughts".
    pub fn capitals(&self) -> String {
        let counts: Vec<(Class, usize)> = capitals::CLASSES.iter().map(|c| (*c, self.caps.iter().filter(|a| a.class == *c).count())).collect();
        capitals::describe(&counts)
    }

    /// Capitals lost, the same way.
    pub fn capitals_lost(&self) -> String {
        let counts: Vec<(Class, usize)> = capitals::CLASSES.iter().map(|c| (*c, self.caps_lost.get(c).copied().unwrap_or(0) as usize)).collect();
        capitals::describe(&counts)
    }

    /// A pilot of this side lost its ship: counted, and a capital pilot
    /// goes back to its own hull (the killmail has the capital).
    fn lost_ship(&mut self, i: u32, bots: &mut [Bot]) {
        self.lost += 1;
        if let Some(a) = self.assigned(i) {
            *self.caps_lost.entry(a.class).or_insert(0) += 1;
            capitals::restore(bots, &a);
        }
    }
}

/// A battle being fought.
pub struct Fight {
    pub stakes: Stakes,
    /// Asked for by hand: every ship it brings may be on the grid.
    pub showcase: bool,
    /// The side whose capitals cyno in first: 0 the attackers, 1 the
    /// defenders.
    first_cyno: usize,
    pub attack: Side,
    pub defend: Side,
    /// Whether it is being fought ship against ship, where players see it.
    pub seen: bool,
    /// The beacon an open-space battle forms up round, while seen.
    beacon: Option<i64>,
    began: Instant,
    last: Instant,
    last_wave: Instant,
    /// When the last defender fell, and the bombardment began.
    cleared: Option<Instant>,
    rallies: Option<(Rally, Rally)>,
    /// Its battle report on the dashboard's killboard.
    report_id: Option<i64>,
    last_report: Instant,
    /// When the capitals last acted, and last said so in the log.
    last_order: Instant,
    last_order_log: Instant,
    /// What the capitals did since the last log line.
    tally: Tally,
    /// When it was first fought where players see it.
    seen_since: Option<Instant>,
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
                if s.field {
                    info!("bots: the open-space battle in {} did not happen: one side had nobody to bring", s.names.system);
                    continue;
                }
                report.news.push(sov::conclude(db, p, &s, won, (0, 0), rng, claim_delay_secs, speed).await?);
                sov::show_towers(db, world, s.system).await;
                continue;
            };
            for side in [&mut f.attack, &mut f.defend] {
                for a in side.alliances.iter().skip(1) {
                    side.ally_names.push(sov::alliance_name(db, *a).await);
                }
                // The capitals, logistics and cyno ships it brings, and who flies them.
                let plan = capitals::plan(side.brought, f.stakes, side.quality, s.showcase.is_some(), rng);
                let yard = presence.yard.clone();
                side.caps = capitals::assign(&side.reserve, bots, &plan, |hull| yard.as_ref().map_or(4, |y| y.race_of(hull)));
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
            for (side, who) in [(&f.attack, &s.names.attacker), (&f.defend, &s.names.defender)] {
                let caps = side.capitals();
                if !caps.is_empty() {
                    report.news.push((now, format!("{who} bring {caps} to {}", s.names.system)));
                }
            }
            info!("bots: {} in {}: {} ({}) against {} ({})", f.stakes.label(), s.names.system, f.attack.brought, f.attack.capitals(), f.defend.brought, f.defend.capitals());
            match report_start(db, &s, &f).await {
                Ok(id) => f.report_id = Some(id),
                Err(e) => tracing::error!("bots: starting a battle report: {}", crate::logging::chain(&e)),
            }
            s.fight = Some(f);
        }
        let seen = seen_in(world, presence, s.system);
        let verdict = {
            let f = s.fight.as_mut().expect("a fight under way");
            if seen && !f.seen {
                f.seen = muster(&s.system, s.defender, s.field, world, f, bots, presence, rng, clock, &mut report);
            } else if !seen && f.seen {
                stand_down(f, s.system, world, bots, atlas, presence, rng, &mut report);
            }
            if f.seen {
                seen_systems.insert(s.system);
                fight_seen(f, &s.system, world, bots, presence, clock, &mut report)
            } else {
                fight_unseen(f, s.system, atlas, bots, p, rng, &mut report)
            }
        };
        if verdict.is_none() {
            let f = s.fight.as_mut().expect("a fight under way");
            if f.last_report.elapsed() >= REPORT_EVERY {
                f.last_report = Instant::now();
                if let Some(id) = f.report_id {
                    if let Err(e) = report_update(db, id, f, None).await {
                        tracing::error!("bots: updating battle report {id}: {}", crate::logging::chain(&e));
                    }
                }
            }
        }
        let Some(won) = verdict else {
            kept.push(s);
            continue;
        };
        let mut f = s.fight.take().expect("a fight under way");
        let lost = (f.attack.lost, f.defend.lost);
        take_down_beacon(&mut f, s.system, world);
        // An open-space battle holds no space: it ends with the battle.
        let mut news = if s.field {
            let (winner, loser) = if won { (&s.names.attacker, &s.names.defender) } else { (&s.names.defender, &s.names.attacker) };
            (crate::bulkdata::filetime_now(), format!("{winner} beat {loser} in open space in {}", s.names.system))
        } else {
            sov::conclude(db, p, &s, won, lost, rng, claim_delay_secs, speed).await?
        };
        news.1 = format!("{} ({} ships lost: {} of {}'s side, {} of {}'s)", news.1, lost.0 + lost.1, lost.0, s.names.attacker, lost.1, s.names.defender);
        if let Some(id) = f.report_id {
            if let Err(e) = report_update(db, id, &f, Some((&news.1, won))).await {
                tracing::error!("bots: closing battle report {id}: {}", crate::logging::chain(&e));
            }
        }
        report.news.push(news);
        // The cynos go out, and the capitals' pilots go back to their own ships.
        for side in [&mut f.attack, &mut f.defend] {
            put_out_cyno(side, s.system, world);
            for a in side.caps.clone() {
                capitals::restore(bots, &a);
                report.touched.push(a.bot);
            }
        }
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

/// Every pilot flying something special in a battle under way goes back to
/// the ship it had before (at shutdown: battles are not saved). Returns them.
pub fn restore_pilots(sieges: &mut [Siege], bots: &mut [Bot]) -> Vec<u32> {
    let mut out = Vec::new();
    for s in sieges.iter_mut() {
        let Some(f) = s.fight.as_mut() else { continue };
        for side in [&mut f.attack, &mut f.defend] {
            for a in std::mem::take(&mut side.caps) {
                capitals::restore(bots, &a);
                out.push(a.bot);
            }
        }
    }
    out
}

fn capitalise(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}

/// Both fleets for a siege: each side's alliance, allies who join, and how
/// many ships they commit. None if a side has nobody to send.
fn assemble(s: &Siege, bots: &[Bot], ids: &[Identity], presence: &Presence, p: &Politics, rng: &mut Rng, report: &mut Report) -> Option<Fight> {
    let _ = report;
    let stakes = s.showcase.unwrap_or_else(|| Stakes::roll(rng));
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
        // Asked for by hand, a side brings two to five hundred ships.
        let brought = if s.showcase.is_some() {
            stakes.showcase_fleet(rng).min(fit.len() as u32)
        } else {
            ((fit.len() as f64 * share) as u32).clamp(FLEET_MIN.min(fit.len() as u32), (super::battle_size() as u32).clamp(FLEET_MIN, SIDE_MAX))
        };
        // The nearest come; the reserve is kept nearest last, to call up in turn.
        fit.sort_by_key(|i| std::cmp::Reverse(nearness(&bots[*i as usize], s.system)));
        let reserve = fit.split_off(fit.len() - brought as usize);
        Some(Side::new(alliances, reserve, brought, wealth(main)))
    };
    let attack = side(s.attacker, &attack_allies, false, rng)?;
    let defend = side(s.defender, &defend_allies, true, rng)?;
    let now = Instant::now();
    Some(Fight {
        stakes,
        showcase: s.showcase.is_some(),
        first_cyno: rng.below(2),
        attack,
        defend,
        seen: false,
        began: now,
        last: now,
        last_wave: now - REINFORCE,
        cleared: None,
        rallies: None,
        report_id: None,
        beacon: None,
        last_report: now,
        last_order: now,
        last_order_log: now,
        tally: Tally::default(),
        seen_since: None,
    })
}

/// The fight comes into view: rally points round the defender's tower. False
/// if there is no tower left to fight round (it is fought unseen).
#[allow(clippy::too_many_arguments)]
#[allow(clippy::too_many_arguments)]
fn muster(system: &i32, defender: i32, open: bool, world: &World, f: &mut Fight, bots: &mut [Bot], presence: &mut Presence, rng: &mut Rng, clock: u32, report: &mut Report) -> bool {
    let dir = rng.in_sphere(1.0);
    let dir = if dir.len() > 1e-6 { dir.unit() } else { crate::ballpark::Vec3::new(1.0, 0.0, 0.0) };
    let place = {
        let w = world.lock();
        let found = w.parks.get(&(*system as i64)).and_then(|park| {
            park.structures.values().find(|t| t.is_tower() && t.alliance_id == Some(defender as i64)).map(|t| (t.id, t.pos, t.field_radius.max(t.radius)))
        });
        found
    };
    let (defend, attack) = match place {
        Some((tower, at, field)) => (
            Rally { anchor: tower, point: at + dir * (field + DEFENDERS_OUT), orbit: field + DEFENDERS_OUT, target: None },
            Rally { anchor: tower, point: at + dir * (field + ATTACKERS_IN), orbit: field + ATTACKERS_ORBIT, target: None },
        ),
        None if open => {
            let Some((beacon, at)) = battlefield(*system, world, dir) else {
                tracing::warn!("bots: an open-space battle in {system} has nobody in space to fight beside; fought unseen until someone undocks");
                return false;
            };
            f.beacon = Some(beacon);
            info!("bots: open-space battle in {system}: the fleets form up round beacon {beacon}, {:.0} km from the pilot watching", FIELD_OFF / 1000.0);
            (
                Rally { anchor: beacon, point: at + dir * FIELD_DEFEND_ORBIT, orbit: FIELD_DEFEND_ORBIT, target: None },
                Rally { anchor: beacon, point: at - dir * FIELD_ATTACK_ORBIT, orbit: FIELD_ATTACK_ORBIT, target: None },
            )
        }
        None => return false,
    };
    f.rallies = Some((attack, defend));
    f.last_wave = Instant::now() - REINFORCE;
    f.seen_since.get_or_insert_with(Instant::now);
    reinforce(f, *system, world, bots, presence, clock, report);
    true
}

/// Puts a beacon in space `FIELD_OFF` from a player in space in `system`, for
/// an open-space battle to form up round. Its id and where it is.
fn battlefield(system: i32, world: &World, dir: Vec3) -> Option<(i64, Vec3)> {
    let mut w = world.lock();
    let park = w.parks.get_mut(&(system as i64))?;
    let watcher = park.ships.values().find(|s| s.client_id != 0 && !s.bot && s.npc.is_none() && s.drone.is_none())?.pos;
    let id = crate::npc::next_id();
    let pos = watcher + dir * FIELD_OFF;
    park.sites.insert(
        id,
        crate::sites::Site {
            id,
            mission_id: 0,
            char_id: 0,
            name: "Battlefield".into(),
            pos,
            rats: Vec::new(),
            wave: 0,
            waves: 0,
            security: -1.0,
            type_id: TYPE_BATTLE_BEACON,
            dungeon: None,
            gate: None,
            part_of: Some(id),
            size: 0.0,
        },
    );
    let entries = crate::svc::space::add_object_entries_for_nearby(park, id);
    for (client, entry) in entries {
        w.notify(&[client], "DoDestinyUpdate", eve_marshal::Value::Tuple(vec![eve_marshal::Value::List(vec![entry]), eve_marshal::Value::Int(0)]));
    }
    Some((id, pos))
}

/// The open-space battle's beacon goes.
fn take_down_beacon(f: &mut Fight, system: i32, world: &World) {
    let Some(id) = f.beacon.take() else { return };
    let mut w = world.lock();
    let Some(park) = w.parks.get_mut(&(system as i64)) else { return };
    let gone = crate::sites::close_site(park, id);
    crate::svc::space::broadcast(&w, system as i64, None, gone);
}

/// Both sides' reserves warp in, as far as the grid has room. A side's
/// capitals wait for a cynosural field and jump to it; with a titan on the
/// grid, the rest of its fleet comes through the titan's bridge.
#[allow(clippy::too_many_arguments)]
fn reinforce(f: &mut Fight, system: i32, world: &World, bots: &mut [Bot], presence: &mut Presence, clock: u32, report: &mut Report) {
    if f.last_wave.elapsed() < REINFORCE {
        return;
    }
    f.last_wave = Instant::now();
    let Some((attack, defend)) = f.rallies else { return };
    // Asked for by hand, every ship it brings may come; otherwise as many as
    // the shown-per-system setting leaves room for.
    let room = if f.showcase { usize::MAX } else { (presence.cap().saturating_sub(ROOM_FOR_OTHERS) / 2).max(1) };
    // The cyno sequence: the subcapitals fight first; one side lights a
    // cyno and its capitals jump in; the other answers with its own.
    let fighting = f.seen_since.map_or(Duration::ZERO, |t| t.elapsed());
    let landed = [f.attack.landed, f.defend.landed];
    let first = f.first_cyno;
    let may_cyno = |me: usize| {
        if me == first {
            fighting >= CYNO_FIRST
        } else {
            landed[first].is_some_and(|t| t.elapsed() >= CYNO_ANSWER) || fighting >= CYNO_FIRST + CYNO_ANSWER + CYNO_LATE
        }
    };
    for (me, (side, rally)) in [(&mut f.attack, attack), (&mut f.defend, defend)].into_iter().enumerate() {
        if side.withdrew {
            continue;
        }
        let bridge = titan_on_grid(side, system, world, presence);
        let mut bridged = 0;
        let mut arrived = 0;
        while side.on_grid.len() < room && arrived < WAVE_MOST {
            let Some(i) = side.reserve.pop() else { break };
            if bots[i as usize].pod.is_some() || bots[i as usize].activity == Activity::Sieging {
                continue;
            }
            if side.assigned(i).is_some_and(|a| a.class.jumps()) {
                side.waiting.push(i);
                continue;
            }
            enlist(bots, presence, i, system, rally, clock);
            if let Some((titan, at)) = bridge {
                presence.jump_drive_in(i, at + scatter(i, 8_000.0), titan);
                bridged += 1;
            }
            side.on_grid.push(i);
            report.touched.push(i);
            arrived += 1;
        }
        if let (Some((titan, _)), true) = (bridge, bridged > 0) {
            info!("bots: titan {titan} bridged {bridged} ship(s) of alliance {} into {system}", side.alliances[0]);
            let w = world.lock();
            let tick = w.parks.get(&(system as i64)).map_or(0, |p| p.tick);
            crate::svc::space::broadcast(&w, system as i64, None, vec![crate::svc::space::special_fx(tick, titan, None, "effects.JumpPortal")]);
        }
        // Capitals jump to the side's cyno, once it is lit, in their turn.
        if side.waiting.is_empty() || !may_cyno(me) {
            continue;
        }
        if side.cyno.is_none() {
            light_cyno(side, system, world, presence);
            if side.cyno.is_some() {
                let order = if me == first { "first" } else { "in answer" };
                info!("bots: alliance {} lit its cyno {order} in {system}, {:.0}s into the fight: {} capital(s) jump in", side.alliances[0], fighting.as_secs_f64(), side.waiting.len());
            }
        }
        let Some((beacon, at)) = side.cyno else { continue };
        let mut jumped: Vec<&'static str> = Vec::new();
        while side.on_grid.len() < room && jumped.len() < JUMP_WAVE_MOST {
            let Some(i) = side.waiting.pop() else { break };
            if bots[i as usize].pod.is_some() || bots[i as usize].activity == Activity::Sieging {
                continue;
            }
            enlist(bots, presence, i, system, rally, clock);
            presence.jump_drive_in(i, at + scatter(i, 10_000.0), beacon);
            side.on_grid.push(i);
            report.touched.push(i);
            jumped.push(side.assigned(i).map_or("ship", |a| a.class.name(1)));
        }
        if !jumped.is_empty() {
            side.landed.get_or_insert_with(Instant::now);
            let mut counts: HashMap<&str, usize> = HashMap::new();
            for c in &jumped {
                *counts.entry(c).or_default() += 1;
            }
            let mut said: Vec<String> = counts.iter().map(|(c, n)| format!("{n} {c}")).collect();
            said.sort();
            info!("bots: {} jumped to cynosural field {beacon} in {system} (alliance {}; {} still waiting)", said.join(", "), side.alliances[0], side.waiting.len());
        }
    }
}

/// A point within `spread` of nowhere in particular, the same for the same bot.
fn scatter(i: u32, spread: f64) -> Vec3 {
    let mut rng = Rng::seeded(u64::from(i).wrapping_mul(0x9E37_79B9_7F4A_7C15));
    let v = rng.in_sphere(1.0);
    if v.len() > 1e-6 { v.unit() * (spread * (0.3 + 0.7 * rng.unit())) } else { Vec3::new(spread, 0.0, 0.0) }
}

/// The side's titan on the grid, out of warp: its ship and where it is.
fn titan_on_grid(side: &Side, system: i32, world: &World, presence: &Presence) -> Option<(i64, Vec3)> {
    let w = world.lock();
    let park = w.parks.get(&(system as i64))?;
    let titan = side
        .caps
        .iter()
        .filter(|a| a.class == Class::Titan && side.on_grid.contains(&a.bot))
        .filter_map(|a| presence.shown_at(a.bot))
        .filter_map(|(_, ship)| park.ships.get(&ship))
        .find(|s| !s.warping())
        .map(|s| (s.id, s.pos));
    titan
}

/// A cyno ship of the side on the grid lights a cynosural field: a beacon
/// the capitals jump to. With none there for a while, any of its ships does.
fn light_cyno(side: &mut Side, system: i32, world: &World, presence: &Presence) {
    let since = *side.waiting_since.get_or_insert_with(Instant::now);
    let mut w = world.lock();
    let Some(park) = w.parks.get_mut(&(system as i64)) else { return };
    let cyno_pilots: Vec<u32> = side.caps.iter().filter(|a| a.class == Class::Cyno).map(|a| a.bot).collect();
    let anyone = since.elapsed() >= CYNO_WAIT;
    let lighter = side
        .on_grid
        .iter()
        .filter(|i| anyone || cyno_pilots.contains(i))
        .filter_map(|i| presence.shown_at(*i))
        .filter_map(|(_, ship)| park.ships.get(&ship))
        .find(|s| !s.warping())
        .map(|s| (s.id, s.pos));
    let Some((ship, at)) = lighter else {
        // Said once a minute while the capitals wait for want of a cyno.
        let waited = since.elapsed().as_secs();
        if waited >= CYNO_WAIT.as_secs() && waited % 60 < 2 {
            warn!(
                "bots: alliance {}'s {} capital(s) have waited {waited}s in {system}: no ship of theirs on the grid to light a cyno ({} on grid, {} cyno pilot(s))",
                side.alliances[0],
                side.waiting.len(),
                side.on_grid.len(),
                cyno_pilots.len()
            );
        }
        return;
    };
    let id = crate::npc::next_id();
    let pos = at + Vec3::new(0.0, 600.0, 0.0);
    park.sites.insert(
        id,
        crate::sites::Site {
            id,
            mission_id: 0,
            char_id: 0,
            name: "Cynosural Field I".into(),
            pos,
            rats: Vec::new(),
            wave: 0,
            waves: 0,
            security: -1.0,
            type_id: TYPE_CYNO_FIELD,
            dungeon: None,
            gate: None,
            part_of: Some(id),
            size: 0.0,
        },
    );
    let entries = crate::svc::space::add_object_entries_for_nearby(park, id);
    for (client, entry) in entries {
        w.notify(&[client], "DoDestinyUpdate", eve_marshal::Value::Tuple(vec![eve_marshal::Value::List(vec![entry]), eve_marshal::Value::Int(0)]));
    }
    let by = if cyno_pilots.iter().any(|i| presence.shown_at(*i).is_some_and(|(_, s)| s == ship)) { "cyno ship" } else { "ship (no cyno ship came)" };
    info!("bots: {by} {ship} of alliance {} lit cynosural field {id} in {system} after {:.0}s; {} capital(s) waiting", side.alliances[0], since.elapsed().as_secs_f64(), side.waiting.len());
    side.cyno = Some((id, pos));
}

/// The side's cynosural field goes out.
fn put_out_cyno(side: &mut Side, system: i32, world: &World) {
    let Some((id, _)) = side.cyno.take() else { return };
    side.waiting_since = None;
    let mut w = world.lock();
    let Some(park) = w.parks.get_mut(&(system as i64)) else { return };
    let gone = crate::sites::close_site(park, id);
    crate::svc::space::broadcast(&w, system as i64, None, gone);
}

/// The fight goes out of view: the ships on the grid go back to the
/// reserves, to fight on unseen.
#[allow(clippy::too_many_arguments)]
fn stand_down(f: &mut Fight, system: i32, world: &World, bots: &mut [Bot], atlas: &Atlas, presence: &mut Presence, rng: &mut Rng, report: &mut Report) {
    take_down_beacon(f, system, world);
    for side in [&mut f.attack, &mut f.defend] {
        put_out_cyno(side, system, world);
        side.sieged.clear();
        let waiting = std::mem::take(&mut side.waiting);
        side.reserve.extend(waiting);
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
#[allow(clippy::too_many_arguments)]
fn fight_seen(f: &mut Fight, system: &i32, world: &World, bots: &mut [Bot], presence: &mut Presence, clock: u32, report: &mut Report) -> Option<bool> {
    for side in [&mut f.attack, &mut f.defend] {
        let mut lost = Vec::new();
        side.on_grid.retain(|i| {
            let b = &bots[*i as usize];
            if b.pod.is_some() {
                lost.push(*i);
                return false;
            }
            b.activity == Activity::Sieging && b.system == *system
        });
        for i in lost {
            side.sieged.remove(&i);
            side.lost_ship(i, bots);
        }
    }
    rout(f);
    reinforce(f, *system, world, bots, presence, clock, report);
    capital_orders(f, *system, world, presence);
    verdict(f, true)
}

/// Every few seconds, where players see it: dreadnoughts go into siege and
/// the attackers' bombard the defenders' tower, logistics repair the most
/// hurt ship of their side, and titans fire their doomsday on the enemy.
fn capital_orders(f: &mut Fight, system: i32, world: &World, presence: &Presence) {
    if f.last_order.elapsed() < ORDERS_EVERY {
        return;
    }
    f.last_order = Instant::now();
    let seen_for = f.seen_since.map_or(Duration::ZERO, |t| t.elapsed());
    let hostile_to_attack: Vec<i64> = f.defend.alliances.iter().map(|a| i64::from(*a)).collect();
    let hostile_to_defend: Vec<i64> = f.attack.alliances.iter().map(|a| i64::from(*a)).collect();
    let mut w = world.lock();
    let Some(park) = w.parks.get_mut(&(system as i64)) else { return };
    let tower = park
        .structures
        .values()
        .find(|t| t.is_tower() && t.alliance_id.is_some_and(|a| hostile_to_attack.contains(&a)))
        .map(|t| t.id);
    let now_ms = crate::combat::now_ms();
    let tick = park.tick;
    let mut tally = Tally::default();
    let mut orders = Vec::new();
    let mut fx = Vec::new();
    for (side, hostile, attacking) in [(&mut f.attack, &hostile_to_attack, true), (&mut f.defend, &hostile_to_defend, false)] {
        // The enemy's ships on the grid: (ship, where, a capital?).
        let enemies: Vec<(i64, Vec3, bool)> = park
            .ships
            .values()
            .filter(|s| s.fight.is_some() && !s.in_warp() && s.pilot.alliance_id.is_some_and(|a| hostile.contains(&a)))
            .map(|s| (s.id, s.pos, CAPITAL_GROUPS.contains(&s.pilot.group_id)))
            .collect();
        // Its ships on the grid: (bot, ship, how hurt).
        let ships: Vec<(u32, i64, f64)> = side
            .on_grid
            .iter()
            .filter_map(|i| presence.shown_at(*i).map(|(_, ship)| (*i, ship)))
            .filter_map(|(i, ship)| {
                let s = park.ships.get(&ship)?;
                let v = &s.fight.as_ref()?.vitals;
                let hurt = (1.0 - v.shield.fraction(now_ms)) + if v.armor_max > 0.0 { v.armor_damage / v.armor_max } else { 0.0 };
                Some((i, ship, hurt))
            })
            .collect();
        for a in side.caps.clone() {
            let Some(&(_, ship, _)) = ships.iter().find(|(i, ..)| *i == a.bot) else { continue };
            if park.ships.get(&ship).map_or(true, |s| s.warping()) {
                continue;
            }
            match a.class {
                Class::Dreadnought => {
                    if side.sieged.insert(a.bot) {
                        fx.push(crate::svc::space::special_fx_active(tick, ship, None, "effects.SiegeMode", true));
                        tally.sieged += 1;
                        info!("bots: dreadnought {ship} of alliance {} went into siege mode in {system}", side.alliances[0]);
                    }
                    if let (true, Some(tower)) = (attacking, tower) {
                        orders.push(BattleOrder::Bombard { ship, tower, damage: BOMBARD_VOLLEY });
                        tally.bombard += 1;
                    } else if let Some(at) = park.ships.get(&ship).map(|s| s.pos) {
                        // No tower to bombard: the nearest enemy capital, else
                        // the nearest enemy ship.
                        let target = enemies
                            .iter()
                            .filter(|(_, pos, _)| (*pos - at).len() <= VOLLEY_RANGE)
                            .min_by(|x, y| (!x.2).cmp(&!y.2).then((x.1 - at).len().total_cmp(&(y.1 - at).len())));
                        if let Some(&(target, ..)) = target {
                            orders.push(BattleOrder::Volley { ship, target, damage: VOLLEY });
                            tally.volleys += 1;
                        }
                    }
                }
                Class::Logistics => {
                    let target = ships.iter().filter(|(i, s, _)| *i != a.bot && *s != ship).max_by(|x, y| x.2.total_cmp(&y.2));
                    if let Some(&(_, to, hurt)) = target {
                        if hurt > 0.05 {
                            orders.push(BattleOrder::Repair { from: ship, to, amount: REPAIR_CYCLE, shield: Class::shield_logistics(a.race) });
                            tally.repairs += 1;
                        }
                    }
                }
                Class::Titan => {
                    let ready = match side.doomsday.get(&a.bot) {
                        Some(t) => t.elapsed() >= DOOMSDAY_EVERY,
                        None => seen_for >= DOOMSDAY_FIRST,
                    };
                    if ready {
                        side.doomsday.insert(a.bot, Instant::now());
                        orders.push(BattleOrder::Doomsday { titan: ship, hostile: hostile.clone() });
                        tally.doomsdays += 1;
                    }
                }
                Class::Carrier | Class::Cyno => {}
            }
        }
    }
    park.battle_orders.extend(orders);
    let tower_state = tower.and_then(|t| park.structures.get(&t)).map(|t| {
        let v = &t.vitals;
        let part = |damage: f64, max: f64| if max > 0.0 { 100.0 * (1.0 - damage / max) } else { 100.0 };
        format!(
            "tower {} at shield {:.0}%, armour {:.0}%, structure {:.0}%",
            t.id,
            100.0 * v.shield.fraction(now_ms),
            part(v.armor_damage, v.armor_max),
            part(v.hull_damage, v.hull_max)
        )
    });
    if !fx.is_empty() {
        crate::svc::space::broadcast(&w, system as i64, None, fx);
    }
    drop(w);
    f.tally.sieged += tally.sieged;
    f.tally.bombard += tally.bombard;
    f.tally.volleys += tally.volleys;
    f.tally.repairs += tally.repairs;
    f.tally.doomsdays += tally.doomsdays;
    if f.last_order_log.elapsed() >= ORDERS_LOG_EVERY {
        f.last_order_log = Instant::now();
        let t = std::mem::take(&mut f.tally);
        let on_grid = |side: &Side, class: Class| side.caps.iter().filter(|a| a.class == class && side.on_grid.contains(&a.bot)).count();
        let dreads_attacking = on_grid(&f.attack, Class::Dreadnought);
        info!(
            "bots: battle in {system}: on grid {} v {} (titans {}/{}, dreads {}/{}, carriers {}/{}, logistics {}/{}); in {}s: {} bombard volley(s), {} dreadnought volley(s) at ships, {} repair cycle(s), {} doomsday(s), {} into siege; {}",
            f.attack.on_grid.len(),
            f.defend.on_grid.len(),
            on_grid(&f.attack, Class::Titan),
            on_grid(&f.defend, Class::Titan),
            dreads_attacking,
            on_grid(&f.defend, Class::Dreadnought),
            on_grid(&f.attack, Class::Carrier),
            on_grid(&f.defend, Class::Carrier),
            on_grid(&f.attack, Class::Logistics),
            on_grid(&f.defend, Class::Logistics),
            ORDERS_LOG_EVERY.as_secs(),
            t.bombard,
            t.volleys,
            t.repairs,
            t.doomsdays,
            t.sieged,
            tower_state.unwrap_or_else(|| if f.beacon.is_some() { "open space: the dreadnoughts fire on the enemy's ships".into() } else if dreads_attacking > 0 { "no enemy tower on the grid: the dreadnoughts fire on the enemy's ships".into() } else { "no tower in view".into() })
        );
    }
}

/// A second of a fight nobody sees: each side loses ships in proportion to
/// the other's strength (capitals count for many), less what its logistics
/// save; capitals are hard to kill; titans' doomsdays thin the enemy. Every
/// ship lost is a killmail.
fn fight_unseen(f: &mut Fight, system: i32, atlas: &Atlas, bots: &mut [Bot], p: &mut Politics, rng: &mut Rng, report: &mut Report) -> Option<bool> {
    let dt = f.last.elapsed().as_secs_f64().min(5.0);
    f.last = Instant::now();
    let (a, d) = (f.attack.strength(), f.defend.strength());
    let noise = |rng: &mut Rng| 0.7 + rng.unit() * 0.6;
    if !f.attack.withdrew && !f.defend.withdrew {
        f.attack.owed += ATTRITION * d * f.defend.quality / f.attack.quality.max(0.1) * dt * noise(rng) * (1.0 - f.attack.repairs());
        f.defend.owed += ATTRITION * a * f.attack.quality / f.defend.quality.max(0.1) * dt * noise(rng) * (1.0 - f.defend.repairs());
    }
    for attacking in [true, false] {
        let (victims, killers) = if attacking { (&mut f.attack, &mut f.defend) } else { (&mut f.defend, &mut f.attack) };
        let whole = victims.owed.floor() as u32;
        victims.owed -= whole as f64;
        for _ in 0..whole {
            let killer = pick_killer(killers, rng);
            if !lose_one(victims, killer, system, atlas, bots, p, rng, report, true) {
                break;
            }
        }
        // The killers' titans fire their doomsdays.
        if killers.withdrew || victims.withdrew {
            continue;
        }
        let titans: Vec<u32> = killers.caps.iter().filter(|c| c.class == Class::Titan && killers.has(c.bot)).map(|c| c.bot).collect();
        for t in titans {
            let ready = match killers.doomsday.get(&t) {
                Some(at) => at.elapsed() >= DOOMSDAY_EVERY,
                None => f.began.elapsed() >= DOOMSDAY_FIRST,
            };
            if !ready {
                continue;
            }
            killers.doomsday.insert(t, Instant::now());
            let subcaps = victims.reserve.iter().filter(|i| victims.assigned(**i).map_or(true, |a| !a.class.jumps())).count();
            let n = ((subcaps as f64 * DOOMSDAY_SHARE).ceil() as usize).min(DOOMSDAY_MOST);
            for _ in 0..n {
                if !lose_one(victims, Some(t), system, atlas, bots, p, rng, report, false) {
                    break;
                }
            }
        }
    }
    rout(f);
    verdict(f, false)
}

/// Who gets the kill: one of the side's ships, a capital now and then.
fn pick_killer(side: &Side, rng: &mut Rng) -> Option<u32> {
    let in_fight: Vec<u32> = side.reserve.iter().chain(&side.on_grid).copied().collect();
    if in_fight.is_empty() {
        return None;
    }
    Some(in_fight[rng.below(in_fight.len())])
}

/// One ship of `side` is lost, fought unseen: a capital only now and then
/// (`spare_capitals`), never a capital to a doomsday. False if it has none
/// left to lose.
#[allow(clippy::too_many_arguments)]
fn lose_one(side: &mut Side, killer: Option<u32>, system: i32, atlas: &Atlas, bots: &mut [Bot], p: &mut Politics, rng: &mut Rng, report: &mut Report, spare_capitals: bool) -> bool {
    let mut pool: Vec<u32> = side.reserve.iter().chain(&side.waiting).copied().collect();
    if !spare_capitals {
        pool.retain(|i| side.assigned(*i).map_or(true, |a| !a.class.jumps()));
    }
    if pool.is_empty() {
        return false;
    }
    let mut v = pool[rng.below(pool.len())];
    if let Some(a) = side.assigned(v) {
        if rng.unit() < a.class.hard_to_kill() {
            v = pool[rng.below(pool.len())];
        }
    }
    side.reserve.retain(|i| *i != v);
    side.waiting.retain(|i| *i != v);
    if let Some(k) = killer {
        p.unseen_kills.push((v, k, system, bots[v as usize].hull));
    }
    p.lost += 1;
    side.lost_ship(v, bots);
    let bot = &mut bots[v as usize];
    if bot.activity != Activity::Sieging {
        let _ = behaviour::reclone(bot, atlas, rng);
        report.touched.push(v);
    }
    true
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

/// Opens the battle's report on the dashboard's killboard.
async fn report_start(db: &Client, s: &Siege, f: &Fight) -> Result<i64, tokio_postgres::Error> {
    let ids = |side: &Side| side.alliances.clone();
    let row = db
        .query_one(
            r#"INSERT INTO "rmrBattleReports"
                   ("solarSystemID", "systemName", "regionName", "stakes",
                    "attackerID", "attackerName", "attackerAllies", "defenderID", "defenderName", "defenderAllies",
                    "attackAlliances", "defendAlliances", "startedAt", "attackBrought", "defendBrought",
                    "attackCapitals", "defendCapitals")
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17)
               RETURNING "battleID""#,
            &[
                &s.system,
                &s.names.system,
                &s.names.region,
                &f.stakes.label(),
                &s.attacker,
                &s.names.attacker,
                &f.attack.ally_names.join(", "),
                &s.defender,
                &s.names.defender,
                &f.defend.ally_names.join(", "),
                &ids(&f.attack),
                &ids(&f.defend),
                &crate::bulkdata::filetime_now(),
                &(f.attack.brought as i32),
                &(f.defend.brought as i32),
                &f.attack.capitals(),
                &f.defend.capitals(),
            ],
        )
        .await?;
    Ok(row.get(0))
}

/// Brings the report up to date; `ended`: how it ended, and whether the
/// attackers won.
async fn report_update(db: &Client, id: i64, f: &Fight, ended: Option<(&str, bool)>) -> Result<(), tokio_postgres::Error> {
    let (outcome, won, at) = match ended {
        Some((text, won)) => (text.to_string(), Some(won), Some(crate::bulkdata::filetime_now())),
        None => (String::new(), None, None),
    };
    db.execute(
        r#"UPDATE "rmrBattleReports"
              SET "attackLost" = $2, "defendLost" = $3, "attackCapitalsLost" = $4, "defendCapitalsLost" = $5,
                  "outcome" = CASE WHEN $6 = '' THEN "outcome" ELSE $6 END,
                  "attackersWon" = COALESCE($7, "attackersWon"), "endedAt" = COALESCE($8, "endedAt")
            WHERE "battleID" = $1"#,
        &[
            &id,
            &(f.attack.lost as i32),
            &(f.defend.lost as i32),
            &f.attack.capitals_lost(),
            &f.defend.capitals_lost(),
            &outcome,
            &won,
            &at,
        ],
    )
    .await?;
    Ok(())
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
        let siege = Siege { war: 1, attacker: ATTACKER, defender: DEFENDER, attacker_exec: 1, attacker_race: 1, attackers: 300, defenders: 300, system: HERE, names, starts: 0, starts_at: 0, fight: None, showcase: None, field: false };
        let politics = Politics::new(&atlas);
        let mut report = Report::default();
        let mut fight = assemble(&siege, &bots, &ids, &presence, &politics, &mut rng, &mut report).expect("both sides have armed ships");
        assert_eq!((fight.attack.brought, fight.defend.brought), (6, 6), "small alliances bring everyone");
        assert!(muster(&HERE, DEFENDER, false, &world, &mut fight, &mut bots, &mut presence, &mut rng, 0, &mut report), "fought round the tower");
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
            verdict = fight_seen(&mut fight, &HERE, &world, &mut bots, &mut presence, 0, &mut report);
            if verdict.is_some() {
                break;
            }
        }
        let (a, d) = fight.standing();
        assert!(fight.attack.lost + fight.defend.lost >= 2, "ships were destroyed: standing {a} and {d}");
        assert!(verdict.is_some() || a == 0 || d == 0, "the battle was decided: {a} and {d}");
    }
}
