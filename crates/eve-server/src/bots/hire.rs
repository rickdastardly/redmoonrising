//! Bots for hire: a player in space invites a bot ship in the same system
//! to their gang, and if it is free and armed, does not hold anything
//! against them, and they can pay its hourly rate, it joins and flies as
//! their wingman. It follows them from system to system (through the gates
//! with them), docks when they dock, and fights whoever attacks them, and
//! what they lock (rats included). The rate is paid from their wallet into
//! the bot's at the start of each hour; when they cannot pay, kick it, leave
//! the gang or log off, it goes home.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use tokio_postgres::Client;
use tracing::info;

use super::behaviour::{self, Activity, Atlas, Bot};
use super::presence::{Identity, Presence};
use super::relations::{self, Stance};
use super::ships::{self, Shipyard};
use crate::combat::{Lock, Rng};
use crate::world::World;

const HOUR: Duration = Duration::from_secs(3_600);
/// ISK an hour, by hull class: frigate to battleship.
const RATES: [f64; 5] = [500_000.0, 1_000_000.0, 2_500_000.0, 5_000_000.0, 10_000_000.0];
const REF_PLAYER_DONATION: i32 = 10;
/// How close the wingman keeps.
pub const ESCORT_RANGE: f64 = 2_500.0;
/// A wingman goes for targets no further off than this.
const ASSIST_RANGE: f64 = 150_000.0;

/// A wingman's contract.
pub struct Hire {
    pub player: i64,
    pub rate: f64,
    paid_until: Instant,
    /// Flying in its alliance's fleet for the player, unpaid: formed by
    /// their word in alliance chat, or to answer an attack in the
    /// alliance's space.
    pub fleet: Option<i32>,
    /// When a fleet answering an attack goes home (none: when told).
    until: Option<Instant>,
}

/// How many of an alliance's bots form up on a player who calls a fleet,
/// and how many come when one of its players is attacked in its space.
pub const FLEET_SIZE: usize = 10;
pub const DEFENCE_SIZE: usize = 6;
/// How long a defence fleet stays after the last attack.
pub const CALM: Duration = Duration::from_secs(3 * 60);

/// Alliances whose players told their bots not to guard their space.
static UNGUARDED: std::sync::Mutex<Vec<i32>> = std::sync::Mutex::new(Vec::new());

/// Whether an alliance's bots guard its space (they do unless told not to).
pub fn guarding(alliance: i32) -> bool {
    !UNGUARDED.lock().map(|u| u.contains(&alliance)).unwrap_or(false)
}

pub fn guard(alliance: i32, on: bool) {
    if let Ok(mut u) = UNGUARDED.lock() {
        u.retain(|a| *a != alliance);
        if !on {
            u.push(alliance);
        }
    }
}

/// Up to `size` of an alliance's free, armed bots form up on `player`: the
/// nearest first, those further off following them in. Returns those who
/// came.
#[allow(clippy::too_many_arguments)]
pub fn form_fleet(hires: &mut Hires, world: &World, bots: &mut [Bot], ids: &[Identity], presence: &mut Presence, alliance: i32, player: i64, size: usize, until: Option<Instant>) -> Vec<u32> {
    let Some(here) = world.lock().presence_of(player).map(|p| p.solar_system_id as i32) else { return Vec::new() };
    let mut fit: Vec<u32> = (0..bots.len() as u32)
        .filter(|i| {
            let (b, id) = (&bots[*i as usize], &ids[*i as usize]);
            id.alliance == Some(alliance)
                && b.online
                && b.pod.is_none()
                && presence.armed(id.hull)
                && !matches!(b.activity, Activity::Sieging | Activity::Escorting)
                && !hires.by_bot.contains_key(i)
        })
        .collect();
    fit.sort_by_key(|i| super::battle::nearness(&bots[*i as usize], here));
    fit.truncate(size);
    for &i in &fit {
        let b = &mut bots[i as usize];
        b.activity = Activity::Escorting;
        b.route.clear();
        b.dock_at = 0;
        hires.by_bot.insert(i, Hire { player, rate: 0.0, paid_until: Instant::now() + HOUR, fleet: Some(alliance), until });
        presence.escort(i, player);
    }
    if !fit.is_empty() {
        info!("bots: {} of alliance {alliance}'s pilots fly in a fleet with {player}", fit.len());
    }
    fit
}

/// The fleet flying with `player` goes home at the next look. Returns how
/// many were in it.
pub fn disband(hires: &mut Hires, alliance: i32, player: i64) -> usize {
    let now = Instant::now();
    let mut n = 0;
    for h in hires.by_bot.values_mut().filter(|h| h.fleet == Some(alliance) && h.player == player) {
        h.until = Some(now);
        n += 1;
    }
    n
}

/// A fleet already flying with `player` stays while they are attacked.
/// False if there is none.
pub fn keep_defending(hires: &mut Hires, alliance: i32, player: i64) -> bool {
    let mut any = false;
    let calm = Instant::now() + CALM;
    for h in hires.by_bot.values_mut().filter(|h| h.fleet == Some(alliance) && h.player == player) {
        any = true;
        if let Some(u) = h.until.as_mut() {
            *u = (*u).max(calm);
        }
    }
    any
}

/// Players under attack by someone outside their alliance (rats aside),
/// for the alliances `guarded` says: (alliance, player, system).
pub fn attacked(w: &crate::world::WorldState, guarded: impl Fn(i32) -> bool) -> Vec<(i32, i64, i32)> {
    let mut out = Vec::new();
    for (system, park) in &w.parks {
        for s in park.ships.values().filter(|s| s.is_player()) {
            let Some(alliance) = s.pilot.alliance_id.map(|a| a as i32).filter(|a| guarded(*a)) else { continue };
            let Some((_, ship)) = s.fight.as_ref().and_then(|f| f.last_attacker) else { continue };
            let Some(a) = park.ships.get(&ship) else { continue };
            if a.pilot.char_id == 0 || a.pilot.alliance_id.map(|x| x as i32) == Some(alliance) || (a.pos - s.pos).len() > ASSIST_RANGE {
                continue;
            }
            out.push((alliance, s.pilot.char_id, *system as i32));
        }
    }
    out
}

#[derive(Default)]
pub struct Hires {
    pub by_bot: HashMap<u32, Hire>,
}

pub fn rate(yard: Option<&Shipyard>, hull: i64) -> f64 {
    let class = yard.map_or(0, |y| ships::weight_class(y.group_of(hull as i32)));
    RATES[class as usize]
}

/// Why a bot turned an offer down, in its own words.
fn refusal(why: &str) -> String {
    format!("Thanks for the offer, but no: {why}")
}

/// Takes up the gang invitations sent to bots, keeps contracts paid, and
/// ends those that are over. Returns the bots whose state changed, with
/// the seconds to their next step (for those sent home).
#[allow(clippy::too_many_arguments)]
pub async fn run(
    hires: &mut Hires,
    world: &World,
    db: &Client,
    db_url: &str,
    atlas: &Atlas,
    bots: &mut [Bot],
    ids: &[Identity],
    presence: &mut Presence,
    yard: Option<&Shipyard>,
    by_char: &HashMap<i64, u32>,
    rng: &mut Rng,
) -> Vec<(u32, u32)> {
    let mut touched = Vec::new();
    let requests = std::mem::take(&mut world.lock().hire_requests);
    for (player, bot_char) in requests {
        let Some(&i) = by_char.get(&bot_char) else { continue };
        let (b, id) = (&bots[i as usize], &ids[i as usize]);
        let here = world.lock().presence_of(player).filter(|p| !p.docked).map(|p| p.solar_system_id as i32);
        let terms = crate::bots::wingmen();
        let fee = rate(yard, id.hull) * terms.price_factor();
        let pilot = crate::ballpark::Pilot {
            type_id: id.hull,
            group_id: 0,
            name: id.name.clone(),
            char_id: b.id as i64,
            corp_id: id.corp as i64,
            alliance_id: id.alliance.map(i64::from),
            security: id.security as f64,
            bounty: 0.0,
        };
        let why = if terms == crate::bots::Wingmen::Off {
            Some("I'm not taking on wingman work right now.".to_string())
        } else if here.is_none() || here != Some(b.system) || b.station != 0 || !b.online {
            Some("I'm not where you are.".to_string())
        } else if b.pod.is_some() || !presence.armed(id.hull) {
            Some("I've nothing to fight with right now.".to_string())
        } else if matches!(b.activity, Activity::Sieging | Activity::Escorting) {
            Some("I'm already spoken for.".to_string())
        } else if hires.by_bot.values().any(|h| h.player == player) && hires.by_bot.values().filter(|h| h.player == player).count() >= 5 {
            Some("you have enough wingmen already.".to_string())
        } else if relations::stance(&pilot, player) == Stance::Hostile {
            Some("I don't fly with people who shoot my friends.".to_string())
        } else {
            None
        };
        let paid = match &why {
            Some(_) => false,
            None => charge(db, db_url, world, player, b.id, fee).await,
        };
        let gang = world.lock().gangs.of(player);
        match (why, paid, gang) {
            (None, true, Some(gang_id)) => {
                crate::svc::gang::bot_joins(&mut world.lock(), gang_id, bot_char);
                let b = &mut bots[i as usize];
                b.activity = Activity::Escorting;
                b.route.clear();
                b.dock_at = 0;
                hires.by_bot.insert(i, Hire { player, rate: fee, paid_until: Instant::now() + HOUR, fleet: None, until: None });
                presence.escort(i, player);
                touched.push((i, 60));
                info!("bots: {} hired by {player} at {fee} ISK an hour", ids[i as usize].name);
                let body = if fee <= 0.0 {
                    "Glad to fly with you, no charge. I'll stay on your wing, follow you through gates, and shoot whatever shoots you or whatever you lock. Kick me from the gang when you're done.".to_string()
                } else {
                    format!(
                        "Glad to fly with you. My rate is {} ISK an hour, paid at the start of each hour; I've taken the first. I'll stay on your wing, follow you through gates, and shoot whatever shoots you or whatever you lock. Kick me from the gang when you're done.",
                        whole(fee)
                    )
                };
                letter(db, world, b_id(bots, i), player, "On your wing", &body).await;
            }
            (why, _, _) => {
                let why = why.unwrap_or_else(|| format!("you can't cover my rate of {} ISK an hour.", whole(fee)));
                crate::svc::gang::bot_declines(&mut world.lock(), bot_char);
                letter(db, world, b_id(bots, i), player, "Your offer", &refusal(&why)).await;
            }
        }
    }

    // Contracts: still in the gang with the player, the player still about,
    // and paid up.
    let mut ended = Vec::new();
    for (&i, hire) in hires.by_bot.iter_mut() {
        let bot_char = bots[i as usize].id as i64;
        // An alliance fleet: unpaid, out of any gang; over when they log
        // off, are told, or (answering an attack) all is calm.
        if hire.fleet.is_some() {
            let online = world.lock().is_online(hire.player);
            if !online || hire.until.is_some_and(|u| Instant::now() >= u) {
                ended.push((i, None));
            }
            continue;
        }
        let (together, online) = {
            let w = world.lock();
            let gang = w.gangs.of(bot_char);
            (gang.is_some() && gang == w.gangs.of(hire.player), w.is_online(hire.player))
        };
        if !together || !online {
            ended.push((i, None));
            continue;
        }
        if Instant::now() >= hire.paid_until {
            if charge(db, db_url, world, hire.player, bot_char as i32, hire.rate).await {
                hire.paid_until += HOUR;
            } else {
                ended.push((i, Some("Your wallet's run dry, so I'm off. Fly safe.")));
            }
        }
    }
    for (i, why) in ended {
        let Some(hire) = hires.by_bot.remove(&i) else { continue };
        let bot_char = bots[i as usize].id as i64;
        crate::svc::gang::bot_leaves(&mut world.lock(), bot_char);
        presence.unescort(i);
        let b = &mut bots[i as usize];
        if b.activity == Activity::Escorting {
            let wait = if b.station != 0 {
                b.activity = Activity::Docked;
                60
            } else {
                behaviour::release(b, atlas, rng)
            };
            touched.push((i, wait));
        }
        if let Some(why) = why {
            letter(db, world, bot_char as i32, hire.player, "Contract over", why).await;
        }
        info!("bots: {}'s contract with {} is over", ids[i as usize].name, hire.player);
    }

    // Where the players are, their wingmen are: in space beside them, or
    // docked in the same station.
    for (&i, hire) in &hires.by_bot {
        let Some(p) = world.lock().presence_of(hire.player) else { continue };
        let b = &mut bots[i as usize];
        let (system, station) = (p.solar_system_id as i32, if p.docked { p.station_id.unwrap_or(0) as i32 } else { 0 });
        if b.system != system || b.station != station {
            b.system = system;
            b.station = station;
            b.pod = None;
            touched.push((i, 60));
        }
    }
    touched
}

fn b_id(bots: &[Bot], i: u32) -> i32 {
    bots[i as usize].id
}

fn whole(amount: f64) -> String {
    let digits = (amount.round() as i64).to_string();
    let mut out = String::new();
    for (n, c) in digits.chars().enumerate() {
        if n > 0 && (digits.len() - n) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// An hour's pay, from the player to the bot. Whether it went through.
async fn charge(db: &Client, db_url: &str, world: &World, player: i64, bot: i32, amount: f64) -> bool {
    if amount <= 0.0 {
        return true;
    }
    use crate::svc::wallet::{move_cash, Payment};
    let payment = Payment { payer: player, payee: Some(bot as i64), amount, ref_type: REF_PLAYER_DONATION, arg_id1: "", reason: "Wingman's hourly rate" };
    let ok = move_cash(db_url, payment).await.is_ok();
    if ok {
        relations::tell_balance(db, world, player as i32).await;
    }
    ok
}

async fn letter(db: &Client, world: &World, from: i32, to: i64, subject: &str, body: &str) {
    if let Err(e) = crate::svc::chat::deliver(db, world, from as i64, &[to], subject, body).await {
        tracing::error!("bots: a wingman's mail to {to}: {}", crate::logging::chain(&e));
    }
}

/// In space, each wingman goes for whoever last shot its player, else for
/// what its player has locked. Called with the world locked.
pub fn assist(w: &mut crate::world::WorldState, hires: &Hires, shown: impl Fn(u32) -> Option<(i32, i64)>) {
    for (&i, hire) in &hires.by_bot {
        let Some((system, ship)) = shown(i) else { continue };
        let Some(park) = w.parks.get_mut(&(system as i64)) else { continue };
        let Some(player) = park.ships.values().find(|s| s.is_player() && s.pilot.char_id == hire.player) else { continue };
        let pos = player.pos;
        let fight = player.fight.as_ref();
        let attacker = fight.and_then(|f| f.last_attacker).map(|(_, s)| s);
        let locked: Vec<i64> = fight.map(|f| f.locks.iter().filter(|(_, l)| matches!(l, Lock::Locked)).map(|(t, _)| *t).collect()).unwrap_or_default();
        let target = attacker
            .into_iter()
            .chain(locked)
            .find(|t| *t != ship && park.ships.get(t).is_some_and(|s| (s.pos - pos).len() <= ASSIST_RANGE && !(s.bot && s.pilot.char_id == 0)));
        let Some(target) = target else { continue };
        if let Some(brain) = park.ships.get_mut(&ship).and_then(|s| s.npc.as_mut()).filter(|b| !b.engaged()) {
            brain.provoked(target);
        }
    }
}
