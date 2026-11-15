//! After a battle players saw, haulers come and loot the field, as they did
//! on Tranquility: a few of the nearest haulers fly in, warp to the wrecks
//! and work through the cans one by one, and sell what they find.

use std::time::{Duration, Instant};

use tokio_postgres::Client;
use tracing::{error, info};

use super::behaviour::{self, Activity, Atlas, Bot, Role};
use super::presence::{Identity, Meet, Presence};
use crate::ballpark::Vec3;
use crate::combat::Rng;
use crate::world::World;

/// How many haulers come to a battlefield.
const HAULERS: usize = 6;
/// How long after the battle they set off, and the longest they stay.
const START_AFTER: Duration = Duration::from_secs(45);
const STAY_MOST: Duration = Duration::from_secs(25 * 60);
/// Each hauler on the field empties a can this often.
const LOOT_EVERY: Duration = Duration::from_secs(12);
/// Cans this far from the middle of the field are part of it.
const FIELD: f64 = 150_000.0;
/// A hauler is at work within this of the middle of the field.
const AT_WORK: f64 = 40_000.0;
/// What the haulers get for what they find, of its base price.
const SELL_SHARE: f64 = 0.6;

struct Op {
    system: i32,
    point: Vec3,
    haulers: Vec<u32>,
    started: Option<Instant>,
    due: Instant,
    last: Instant,
    looted: usize,
    value: f64,
}

#[derive(Default)]
pub struct Salvage {
    ops: Vec<Op>,
}

/// A battle in `system` is over: haulers will come for what it left.
pub fn after_battle(s: &mut Salvage, system: i32) {
    if s.ops.iter().any(|o| o.system == system) {
        return;
    }
    let now = Instant::now();
    s.ops.push(Op { system, point: Vec3::ZERO, haulers: Vec::new(), started: None, due: now + START_AFTER, last: now, looted: 0, value: 0.0 });
}

/// The cans in a system's field: (can, where).
fn cans(world: &World, system: i32, around: Option<Vec3>) -> Vec<(i64, Vec3)> {
    let w = world.lock();
    let Some(park) = w.parks.get(&(system as i64)) else { return Vec::new() };
    park.cans.values().filter(|c| around.is_none_or(|p| (c.pos - p).len() <= FIELD)).map(|c| (c.id, c.pos)).collect()
}

/// Sends haulers, loots, and sends them home. Returns the bots whose state
/// changed, and those sent home.
#[allow(clippy::too_many_arguments)]
pub async fn run(s: &mut Salvage, db: &mut Client, world: &World, atlas: &Atlas, bots: &mut [Bot], ids: &[Identity], presence: &mut Presence, rng: &mut Rng) -> (Vec<u32>, Vec<u32>) {
    let (mut touched, mut released) = (Vec::new(), Vec::new());
    let now = Instant::now();
    let mut kept = Vec::new();
    for mut op in std::mem::take(&mut s.ops) {
        if now < op.due {
            kept.push(op);
            continue;
        }
        if op.started.is_none() {
            let field = cans(world, op.system, None);
            if field.is_empty() {
                continue;
            }
            op.point = field.iter().fold(Vec3::ZERO, |a, c| a + c.1) * (1.0 / field.len() as f64);
            let anchor = nearest(&field, op.point);
            let mut free: Vec<u32> = (0..bots.len() as u32)
                .filter(|i| {
                    let b = &bots[*i as usize];
                    b.role == Role::Hauler && b.online && b.pod.is_none() && !matches!(b.activity, Activity::Sieging | Activity::Escorting | Activity::Travelling)
                })
                .collect();
            free.sort_by_key(|i| super::battle::nearness(&bots[*i as usize], op.system));
            free.truncate(HAULERS);
            if free.is_empty() {
                continue;
            }
            let mut w = world.lock();
            for &i in &free {
                let b = &mut bots[i as usize];
                b.system = op.system;
                b.station = 0;
                b.route.clear();
                b.dock_at = 0;
                b.activity = Activity::Escorting;
                presence.salvage(&mut w, i, b, Meet { system: op.system, anchor, point: op.point });
                touched.push(i);
            }
            drop(w);
            info!("bots: {} hauler(s) come to loot the battlefield in {} ({} can(s))", free.len(), op.system, field.len());
            op.haulers = free;
            op.started = Some(now);
            op.last = now;
            kept.push(op);
            continue;
        }
        if now.duration_since(op.last) >= LOOT_EVERY {
            op.last = now;
            let field = cans(world, op.system, Some(op.point));
            // Those at work: in the system, on the field.
            let working: Vec<u32> = {
                let w = world.lock();
                op.haulers
                    .iter()
                    .copied()
                    .filter(|i| {
                        presence.shown_at(*i).is_some_and(|(sys, ship)| {
                            sys == op.system && w.parks.get(&(sys as i64)).and_then(|p| p.ships.get(&ship)).is_some_and(|s| !s.warping() && (s.pos - op.point).len() <= AT_WORK)
                        })
                    })
                    .collect()
            };
            let mut taken = field.clone();
            taken.sort_by(|a, b| (a.1 - op.point).len().total_cmp(&(b.1 - op.point).len()));
            for (&hauler, (can, _)) in working.iter().zip(taken.iter()) {
                let value = match db
                    .query_one(
                        r#"SELECT COALESCE(sum(e."quantity"::float8 * COALESCE(t."basePrice", 0)::float8), 0)::float8
                             FROM entity e JOIN "invTypes" t ON t."typeID" = e."typeID" WHERE e."locationID" = $1"#,
                        &[&(*can as i32)],
                    )
                    .await
                {
                    Ok(r) => r.get::<_, f64>(0),
                    Err(e) => {
                        error!("bots: valuing can {can}: {}", crate::logging::chain(&e));
                        0.0
                    }
                };
                crate::loot::scrap(db, *can).await;
                crate::loot::forget_cans(world, &[*can as i32]);
                let earned = value * SELL_SHARE;
                bots[hauler as usize].isk += earned;
                touched.push(hauler);
                op.looted += 1;
                op.value += earned;
            }
            // The next can for each to go round.
            let left = cans(world, op.system, Some(op.point));
            if !left.is_empty() {
                let anchor = nearest(&left, op.point);
                let mut w = world.lock();
                for &i in &op.haulers {
                    presence.salvage(&mut w, i, &bots[i as usize], Meet { system: op.system, anchor, point: op.point });
                }
            }
            let done = left.is_empty() || op.started.is_some_and(|t| now.duration_since(t) >= STAY_MOST);
            if done {
                for &i in &op.haulers {
                    presence.end_salvage(i);
                    let b = &mut bots[i as usize];
                    if b.activity == Activity::Escorting {
                        behaviour::release(b, atlas, rng);
                        released.push(i);
                    }
                    touched.push(i);
                }
                info!("bots: haulers looted the battlefield in {}: {} can(s), sold for {:.1}M ISK", op.system, op.looted, op.value / 1.0e6);
                continue;
            }
        }
        kept.push(op);
    }
    s.ops = kept;
    let _ = ids;
    (touched, released)
}

fn nearest(field: &[(i64, Vec3)], point: Vec3) -> i64 {
    field.iter().min_by(|a, b| (a.1 - point).len().total_cmp(&(b.1 - point).len())).map_or(0, |c| c.0)
}
