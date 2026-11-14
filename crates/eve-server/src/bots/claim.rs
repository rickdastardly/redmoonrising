//! Claiming null-sec for a player's alliance on its word. A player says
//! "start claiming sov for our alliance" in alliance chat (`talk`) and the
//! alliance's bot corporations go about it as players would, telling the
//! alliance chat as they go:
//!
//! - Towers: a large control tower of their race is anchored at a moon in
//!   the alliance's home region (`sov::player_claimants`), a few at a time,
//!   spreading out from the space already held. Once anchored it is fuelled
//!   and onlined; online, its claim counts towards sovereignty (the claim
//!   delay, `sovClaimDelaySeconds`, scaled by the bots' clock).
//! - Outposts: in a system where a tower of theirs is online and there is no
//!   station, an outpost construction platform of their race is anchored by
//!   a planet; anchored, the bots bring in its materials and build the
//!   outpost (`outpost::build_for_bots`), then hand it to the alliance's
//!   executor corporation, the alliance's holder.
//! - Timers are the structures' own (anchoringDelay, onliningDelay), run at
//!   the starbase speedup and the bots' speed; the client shows them.
//! - It costs the bots: each tower and outpost is paid for from the richest
//!   bot corporation in the alliance; they say so when they cannot pay.
//!
//! "stop claiming sov" ends new work (what stands stays up and fuelled);
//! "sov status" has them say how it stands. Orders outlive a restart
//! (`rmrSovCampaigns`).

use std::collections::{HashMap, HashSet, VecDeque};
use std::time::{Duration, Instant};

use tokio_postgres::Client;
use tracing::{error, info, warn};

use super::behaviour::Atlas;
use super::control::SovOrder;
use super::officers::Office;
use super::politics::Politics;
use super::sov;
use super::talk::alliance_report;
use crate::combat::Rng;
use crate::pos::state;
use crate::svc::rows::{int, text};

/// How often the campaigns are looked at, in real time.
const STEP_EVERY: Duration = Duration::from_secs(15);
/// Towers anchoring or onlining at once, per alliance.
const AT_ONCE: usize = 3;
/// One outpost for every this many systems with an online tower.
const SYSTEMS_PER_OUTPOST: usize = 2;
const SECOND_FT: i64 = 10_000_000;
/// Outpost construction platforms by race (Caldari, Minmatar, Amarr, Gallente).
const PLATFORMS: [(i64, i32); 4] = [(1, 19758), (2, 10258), (4, 10260), (8, 10257)];
const GROUP_TOWER: i32 = 365;
const GROUP_PLATFORM: i32 = 307;
const GROUP_PLANET: i32 = 7;
/// How far off its planet's surface a platform is anchored, in metres.
const OFF_PLANET: f64 = 150_000.0;
/// How often the alliance hears that its bots cannot pay.
const BROKE_EVERY: Duration = Duration::from_secs(30 * 60);

/// The alliances whose bots are claiming space, and what they last said.
#[derive(Default)]
pub struct Campaigns {
    loaded: bool,
    next: Option<Instant>,
    /// Alliance -> the character who gave the order.
    orders: HashMap<i32, i64>,
    /// Towers the bots onlined, until they say each is online.
    onlining: HashSet<i64>,
    /// Outposts built and not yet handed over: (station, alliance, name).
    handover: Vec<(i64, i32, String)>,
    broke: HashMap<i32, Instant>,
    /// Alliances told they hold all the space they can for now.
    full: HashSet<i32>,
    /// Alliances told their bots have no corporation to claim with.
    idle: HashSet<i32>,
}

/// A tower or platform of the alliance's, as the database has it.
struct Piece {
    id: i64,
    system: i32,
    state: i16,
    stamp: Option<i64>,
    type_id: i32,
    owner: i32,
    moon: Option<i32>,
    group: i32,
}

/// When a step of `delay` (FILETIME ticks) starting now is stamped as begun:
/// at the starbase speedup and the bots' speed.
fn timer_start(now: i64, delay: i64, speed: f64) -> i64 {
    let stamp = crate::pos::timer_start(now, delay);
    let left = stamp + delay - now;
    stamp - (left - (left as f64 / speed.max(1.0)) as i64)
}

fn left_text(stamp: i64, delay: i64, now: i64) -> String {
    crate::pos::duration_text((stamp + delay - now) / SECOND_FT)
}

async fn load(c: &mut Campaigns, db: &Client) {
    match db.query(r#"SELECT "allianceID", "orderedBy" FROM "rmrSovCampaigns""#, &[]).await {
        Ok(rows) => {
            for r in rows {
                if let (Some(a), Some(by)) = (int(&r, 0), int(&r, 1)) {
                    c.orders.insert(a as i32, by);
                }
            }
            if !c.orders.is_empty() {
                info!("bots: {} alliance(s) have their bots claiming sovereignty", c.orders.len());
            }
            c.loaded = true;
        }
        Err(e) => error!("bots: reading sovereignty orders: {}", crate::logging::chain(&e)),
    }
}

/// A player's order, heard in alliance chat.
pub async fn order(c: &mut Campaigns, db: &Client, p: &mut Politics, alliance: i32, by: i64, order: SovOrder) {
    if !c.loaded {
        load(c, db).await;
    }
    let who = character_name(db, by).await;
    let first = who.split_whitespace().next().unwrap_or("mate").to_string();
    let say = |text: String| alliance_report(alliance as i64, None, None, text);
    match order {
        SovOrder::Start => {
            let claimant = match sov::player_claimants(db, p).await {
                Ok(list) => list.into_iter().find(|x| x.0 == alliance),
                Err(e) => {
                    error!("bots: finding where alliance {alliance} claims: {}", crate::logging::chain(&e));
                    return;
                }
            };
            let Some((_, region, executor, _, pilots)) = claimant else {
                say(format!("Sorry {first}, none of our corporations has pilots to spare for it just now."));
                return;
            };
            let saved = db
                .execute(
                    r#"INSERT INTO "rmrSovCampaigns" ("allianceID", "orderedBy", "since") VALUES ($1, $2, $3)
                       ON CONFLICT ("allianceID") DO UPDATE SET "orderedBy" = EXCLUDED."orderedBy""#,
                    &[&alliance, &(by as i32), &crate::bulkdata::filetime_now()],
                )
                .await;
            if let Err(e) = saved {
                error!("bots: saving alliance {alliance}'s sovereignty order: {}", crate::logging::chain(&e));
            }
            let again = c.orders.insert(alliance, by).is_some();
            c.full.remove(&alliance);
            c.idle.remove(&alliance);
            c.broke.remove(&alliance);
            c.next = None;
            let (region_name, holder) = (sov::region_name(db, region).await, holder(db, alliance).await);
            let holder_name = holder.map_or_else(|| "the alliance".to_string(), |(_, n)| n);
            info!("bots: {who} ordered alliance {alliance}'s bots to claim sovereignty in {region_name}");
            let ceo = super::officers::holder(db, executor, Office::Ceo).await;
            let name_of = |h: Option<(i32, String, String)>| h.map_or_else(|| "one of ours".to_string(), |(_, n, _)| n);
            let text = if again {
                format!("Already on it, {first}: still claiming {region_name}.")
            } else {
                let towers = name_of(super::officers::holder(db, executor, Office::Starbase).await);
                let stations = name_of(super::officers::holder(db, executor, Office::Stations).await);
                format!(
                    "o7 {first}, on it. We'll take {region_name} for the alliance. {towers}, our Starbase Manager, will get control towers up at the moons, fuelled and online; {stations}, our Station Manager, will build outposts and hand them to {holder_name}. {pilots} of us are in."
                )
            };
            alliance_report(alliance as i64, Some(executor as i64), ceo.map(|(id, ..)| id as i64), text);
        }
        SovOrder::Stop => {
            if let Err(e) = db.execute(r#"DELETE FROM "rmrSovCampaigns" WHERE "allianceID" = $1"#, &[&alliance]).await {
                error!("bots: ending alliance {alliance}'s sovereignty order: {}", crate::logging::chain(&e));
            }
            if c.orders.remove(&alliance).is_some() {
                info!("bots: {who} told alliance {alliance}'s bots to stop claiming sovereignty");
                say(format!("Copy {first}, standing down: no new towers or outposts. What's up stays up, and we'll keep it fuelled."));
            } else {
                say(format!("We're not claiming anything just now, {first}. Say \"start claiming sov\" when you want us to."));
            }
        }
        SovOrder::Status => match status(db, alliance, c.orders.contains_key(&alliance)).await {
            Ok(text) => say(text),
            Err(e) => error!("bots: alliance {alliance}'s sovereignty status: {}", crate::logging::chain(&e)),
        },
    }
}

/// How the alliance's claim stands, in a line.
async fn status(db: &Client, alliance: i32, ordered: bool) -> Result<String, tokio_postgres::Error> {
    let pieces = pieces(db, alliance).await?;
    let towers: Vec<&Piece> = pieces.iter().filter(|x| x.group == GROUP_TOWER).collect();
    let count = |s: &[i16]| towers.iter().filter(|t| s.contains(&t.state)).count();
    let sov = db
        .query_one(r#"SELECT count(*) FROM "mapSolarSystems" WHERE "allianceID" = $1"#, &[&alliance])
        .await?
        .get::<_, i64>(0);
    let outposts = db
        .query_one(
            r#"SELECT count(*) FROM "staStations" st JOIN "corporation" co ON co."corporationID" = st."corporationID"
                WHERE co."allianceID" = $1 AND st."stationID" BETWEEN $2 AND $3"#,
            &[&alliance, &crate::svc::outpost::OUTPOST_ID_MIN, &crate::svc::outpost::OUTPOST_ID_MAX],
        )
        .await?
        .get::<_, i64>(0);
    let platforms = pieces.iter().filter(|x| x.group == GROUP_PLATFORM).count();
    let mut text = format!(
        "Sov report: {} tower(s) online, {} onlining, {} anchoring; {sov} system(s) with our sovereignty on the map; {outposts} outpost(s), {platforms} being built.",
        count(&[state::ONLINE, state::REINFORCED, state::OPERATING]),
        count(&[state::ONLINING]),
        count(&[state::ANCHORED]),
    );
    text.push_str(if ordered { " Still claiming." } else { " We're not claiming more until you say so." });
    Ok(text)
}

/// Every few seconds: each ordered alliance's campaign moves on.
#[allow(clippy::too_many_arguments)]
pub async fn step(c: &mut Campaigns, db: &mut Client, world: &crate::world::World, atlas: &Atlas, p: &mut Politics, rng: &mut Rng, claim_delay_secs: i64, speed: f64) {
    let now = Instant::now();
    if c.next.is_some_and(|n| now < n) {
        return;
    }
    c.next = Some(now + STEP_EVERY);
    if !c.loaded {
        load(c, db).await;
    }
    if c.orders.is_empty() && c.handover.is_empty() {
        return;
    }
    let claimants = match sov::player_claimants(db, p).await {
        Ok(list) => list,
        Err(e) => {
            error!("bots: sovereignty campaigns: {}", crate::logging::chain(&e));
            return;
        }
    };
    let mut alliances: Vec<i32> = c.orders.keys().copied().chain(c.handover.iter().map(|h| h.1)).collect();
    alliances.sort_unstable();
    alliances.dedup();
    for alliance in alliances {
        let ordered = c.orders.contains_key(&alliance);
        let Some(&(_, region, executor, race, pilots)) = claimants.iter().find(|x| x.0 == alliance) else {
            if ordered && c.idle.insert(alliance) {
                warn!("bots: alliance {alliance} ordered its bots to claim sovereignty but has no bot corporation with pilots");
            }
            continue;
        };
        c.idle.remove(&alliance);
        let campaign = Campaign { alliance, region, executor, race, pilots, ordered, speed, claim_delay_secs };
        if let Err(e) = campaign.step(c, db, world, atlas, p, rng).await {
            error!("bots: alliance {alliance}'s sovereignty campaign: {}", crate::logging::chain(&e));
        }
    }
}

struct Campaign {
    alliance: i32,
    region: i32,
    /// The bot corporation that anchors (the alliance's biggest), and its race.
    executor: i32,
    race: i64,
    pilots: u32,
    ordered: bool,
    speed: f64,
    claim_delay_secs: i64,
}

impl Campaign {
    /// Says `text` in alliance chat as the officer whose work it is.
    async fn say(&self, db: &Client, office: Office, text: String) {
        let speaker = super::officers::holder(db, self.executor, office).await.map(|(id, ..)| id as i64);
        alliance_report(self.alliance as i64, Some(self.executor as i64), speaker, text);
    }

    #[allow(clippy::too_many_arguments)]
    async fn step(&self, c: &mut Campaigns, db: &mut Client, world: &crate::world::World, atlas: &Atlas, p: &mut Politics, rng: &mut Rng) -> Result<(), tokio_postgres::Error> {
        let now = crate::bulkdata::filetime_now();
        let bot_corps: Vec<i32> = p.corps.iter().filter(|(_, x)| x.alliance == Some(self.alliance)).map(|(id, _)| *id).collect();
        let (holder, holder_name) = holder(db, self.alliance).await.unwrap_or((self.executor, "the alliance".into()));

        // Outposts built: handed over.
        let mut kept = Vec::new();
        for (station, alliance, name) in std::mem::take(&mut c.handover) {
            if alliance != self.alliance {
                kept.push((station, alliance, name));
                continue;
            }
            if crate::svc::outpost::transfer(db, world, station, holder as i64).await? {
                info!("bots: outpost {name} ({station}) handed to corporation {holder}");
                self.say(db, Office::Ceo, format!("{name} now belongs to {holder_name}. She's all yours. o7")).await;
            }
        }
        c.handover = kept;

        let pieces = pieces(db, self.alliance).await?;
        let types: Vec<i32> = pieces.iter().map(|x| x.type_id).collect();
        let delays = crate::pos::delays(&*db, &types).await?;
        let delay = |x: &Piece, k: usize| delays.get(&(x.type_id as i64)).map_or(0, |d| d[k]);
        let done = |x: &Piece, k: usize| x.stamp.is_none_or(|s| s + delay(x, k) <= now);

        // Towers anchored: fuelled and onlined. Online ones counted.
        for t in pieces.iter().filter(|x| x.group == GROUP_TOWER) {
            let ours = bot_corps.contains(&t.owner);
            if t.state == state::ANCHORED && ours && done(t, 0) {
                if !self.ordered {
                    continue;
                }
                sov::refuel_tower(db, Some(t.id as i32)).await?;
                let stamp = timer_start(now, delay(t, 1), self.speed);
                let set = db
                    .execute(
                        r#"UPDATE "posStructures" SET "state" = $2, "stateTimestamp" = $3, "lastFuel" = $4, "claimSince" = NULL
                            WHERE "itemID" = $1 AND "state" = $5"#,
                        &[&(t.id as i32), &state::ONLINING, &stamp, &now, &state::ANCHORED],
                    )
                    .await?;
                if set == 0 {
                    continue;
                }
                crate::svc::starbase::show_state(world, t.system as i64, t.id, state::ONLINING, Some(stamp));
                c.onlining.insert(t.id);
                let (tower, moon) = (type_name(db, t.type_id).await, moon_name(db, t.moon).await);
                info!("bots: alliance {}'s {tower} at {moon} is fuelled and onlining", self.alliance);
                self.say(db, Office::Starbase, format!("{tower} at {moon} has anchored. Fuel and strontium are in, putting it online: {} to go.", left_text(stamp, delay(t, 1), now))).await;
            } else if matches!(t.state, state::ONLINE | state::OPERATING) && c.onlining.remove(&t.id) {
                self.went_online(db, t).await?;
            }
        }

        if !self.ordered {
            return Ok(());
        }

        // More towers, a few at a time, out from the space held.
        let mine: HashSet<i32> = pieces.iter().filter(|x| x.group == GROUP_TOWER).map(|x| x.system).collect();
        let busy = pieces.iter().filter(|x| x.group == GROUP_TOWER && matches!(x.state, state::ANCHORED | state::ONLINING)).count();
        let size = atlas.systems.values().filter(|s| s.region == self.region && s.security <= 0.0).count();
        let want = ((self.pilots / sov::PILOTS_PER_SYSTEM) as usize).max(sov::MIN_SYSTEMS).min(((size as f64 * sov::MAX_SHARE) as usize).max(1));
        if mine.len() < want && busy < AT_ONCE {
            let taken = taken_systems(db, self.alliance).await?;
            let mut order = self.expansion(atlas, &mine, &taken);
            let mut anchored = 0;
            while anchored < (AT_ONCE - busy).min(want - mine.len()) {
                let Some(system) = order.pop_front() else { break };
                if self.anchor_tower(c, db, world, rng, &bot_corps, system).await? {
                    anchored += 1;
                } else if c.broke.contains_key(&self.alliance) {
                    break;
                }
            }
            if anchored == 0 && busy == 0 && order.is_empty() && c.full.insert(self.alliance) {
                self.say(db, Office::Director, format!(
                    "There's no free moon left for us in {} that nobody else holds. We hold {} system(s) there.",
                    sov::region_name(db, self.region).await,
                    mine.len()
                )).await;
            }
        } else if mine.len() >= want && busy == 0 && c.full.insert(self.alliance) {
            self.say(db, Office::Director, format!(
                "That's as much of {} as {} of us can hold: {} system(s). We'll take more as the alliance grows.",
                sov::region_name(db, self.region).await,
                self.pilots,
                mine.len()
            )).await;
        } else if mine.len() < want {
            c.full.remove(&self.alliance);
        }

        // Platforms anchored: the outpost built.
        for x in pieces.iter().filter(|x| x.group == GROUP_PLATFORM && x.state == state::ANCHORED && bot_corps.contains(&x.owner)) {
            if !done(x, 0) {
                continue;
            }
            match crate::svc::outpost::build_for_bots(db, world, x.system as i64, x.id, &holder_name).await? {
                Ok((station, name)) => {
                    info!("bots: alliance {} built outpost {name} ({station})", self.alliance);
                    self.politics_news(p, format!("{} raised an outpost: {name}", sov::alliance_name(db, self.alliance).await));
                    self.say(db, Office::Stations, format!("{name} is up and online! Handing it over to {holder_name}.")).await;
                    c.handover.push((station, self.alliance, name));
                    c.next = Some(Instant::now() + Duration::from_secs(5));
                }
                Err(why) => {
                    warn!("bots: alliance {}'s platform {} could not build: {why}", self.alliance, x.id);
                    self.say(db, Office::Stations, format!("The outpost platform in {} won't build: {why}.", sov::system_name(db, x.system).await)).await;
                }
            }
        }

        // A new platform where a tower is online and there is no station.
        let building = pieces.iter().any(|x| x.group == GROUP_PLATFORM);
        let online: Vec<i32> = {
            let mut v: Vec<i32> = pieces.iter().filter(|x| x.group == GROUP_TOWER && matches!(x.state, state::ONLINE | state::OPERATING | state::REINFORCED)).map(|x| x.system).collect();
            v.sort_unstable();
            v.dedup();
            v
        };
        if !building && !online.is_empty() {
            let rows = db
                .query(
                    r#"SELECT DISTINCT st."solarSystemID", st."stationID" BETWEEN $2 AND $3 FROM "staStations" st WHERE st."solarSystemID" = ANY($1)"#,
                    &[&online, &crate::svc::outpost::OUTPOST_ID_MIN, &crate::svc::outpost::OUTPOST_ID_MAX],
                )
                .await?;
            let with_station: HashSet<i32> = rows.iter().filter_map(|r| int(r, 0).map(|v| v as i32)).collect();
            let outposts = rows.iter().filter(|r| r.try_get::<_, bool>(1).unwrap_or(false)).count();
            let wanted = online.len().div_ceil(SYSTEMS_PER_OUTPOST);
            if outposts < wanted {
                let held: HashSet<i32> = online.iter().copied().collect();
                let best = online
                    .iter()
                    .copied()
                    .filter(|s| !with_station.contains(s))
                    .max_by_key(|s| (atlas.systems.get(s).map_or(0, |x| x.neighbours.iter().filter(|n| held.contains(n)).count()), -*s));
                if let Some(system) = best {
                    self.anchor_platform(c, db, world, &bot_corps, system).await?;
                }
            }
        }
        Ok(())
    }

    fn politics_news(&self, p: &mut Politics, text: String) {
        p.tell(vec![(crate::bulkdata::filetime_now(), text)]);
    }

    /// A tower of theirs came online: its claim starts (sooner, at the bots'
    /// speed), and the alliance hears when the system will be theirs.
    async fn went_online(&self, db: &Client, t: &Piece) -> Result<(), tokio_postgres::Error> {
        let now = crate::bulkdata::filetime_now();
        let delay = self.claim_delay_secs as f64;
        let since = now - ((delay - delay / self.speed.max(1.0)) as i64) * SECOND_FT;
        if since < now {
            db.execute(
                r#"UPDATE "posStructures" SET "claimSince" = $2 WHERE "itemID" = $1 AND ("claimSince" IS NULL OR "claimSince" > $2)"#,
                &[&(t.id as i32), &since],
            )
            .await?;
        }
        let left = (delay / self.speed.max(1.0)) as i64;
        let (tower, moon) = (type_name(db, t.type_id).await, moon_name(db, t.moon).await);
        let when = if left <= 0 { "straight away".to_string() } else { format!("in {}", crate::pos::duration_text(left)) };
        self.say(db, Office::Starbase, format!("{tower} at {moon} is online. Our claim on {} stands {when}.", sov::system_name(db, t.system).await)).await;
        Ok(())
    }

    /// Systems of the home region to claim next: nearest the space held
    /// first (from the region's first station system when none is), skipping
    /// any another alliance claims.
    fn expansion(&self, atlas: &Atlas, mine: &HashSet<i32>, taken: &HashSet<i32>) -> VecDeque<i32> {
        let null = |s: &i32| atlas.systems.get(s).is_some_and(|x| x.region == self.region && x.security <= 0.0);
        let mut starts: Vec<i32> = mine.iter().copied().filter(null).collect();
        if starts.is_empty() {
            let mut all: Vec<i32> = atlas.systems.keys().copied().filter(null).collect();
            all.sort_unstable();
            let home = all.iter().copied().find(|s| atlas.systems.get(s).is_some_and(|x| !x.stations.is_empty()) && !taken.contains(s)).or_else(|| all.iter().copied().find(|s| !taken.contains(s)));
            starts.extend(home);
        }
        starts.sort_unstable();
        let mut seen: HashSet<i32> = starts.iter().copied().collect();
        let mut queue: VecDeque<i32> = starts.into_iter().collect();
        let mut out = VecDeque::new();
        while let Some(s) = queue.pop_front() {
            if !mine.contains(&s) && !taken.contains(&s) {
                out.push_back(s);
            }
            let mut next: Vec<i32> = atlas.systems.get(&s).map(|x| x.neighbours.clone()).unwrap_or_default();
            next.sort_unstable();
            for n in next {
                if null(&n) && seen.insert(n) {
                    queue.push_back(n);
                }
            }
        }
        out
    }

    /// Pays for and anchors a tower at a free moon of `system`. False if it
    /// could not (no free moon, or no money: then the alliance hears so).
    async fn anchor_tower(&self, c: &mut Campaigns, db: &mut Client, world: &crate::world::World, rng: &mut Rng, bot_corps: &[i32], system: i32) -> Result<bool, tokio_postgres::Error> {
        let Some(place) = sov::spot(db, system, rng).await? else { return Ok(false) };
        let Some(payer) = self.pay(c, db, bot_corps, sov::TOWER_COST, "a control tower").await? else { return Ok(false) };
        let tower_type = sov::TOWERS.iter().find(|(r, _)| *r == self.race).map_or(sov::TOWERS[2].1, |(_, t)| *t);
        let now = crate::bulkdata::filetime_now();
        let delay = crate::pos::delays(&*db, &[tower_type]).await?.get(&(tower_type as i64)).map_or(0, |d| d[0]);
        let stamp = timer_start(now, delay, self.speed);
        if !sov::place_tower(db, self.executor, tower_type, &place, state::ANCHORED, stamp, None).await? {
            refund(db, payer, sov::TOWER_COST).await?;
            return Ok(false);
        }
        sov::show_towers(db, world, system).await;
        let (tower, moon) = (type_name(db, tower_type).await, moon_name(db, Some(place.moon)).await);
        info!("bots: alliance {} anchoring a {tower} at {moon}", self.alliance);
        self.say(db, Office::Starbase, format!("Anchoring a {tower} at {moon} ({}): anchored in {}.", sov::region_name(db, self.region).await, left_text(stamp, delay, now))).await;
        Ok(true)
    }

    /// Pays for and anchors an outpost platform by a planet of `system`.
    async fn anchor_platform(&self, c: &mut Campaigns, db: &mut Client, world: &crate::world::World, bot_corps: &[i32], system: i32) -> Result<(), tokio_postgres::Error> {
        let Some(planet) = db
            .query_opt(
                r#"SELECT "x", "y", "z", COALESCE("radius", 5000000) FROM "mapDenormalize"
                    WHERE "solarSystemID" = $1 AND "groupID" = $2 ORDER BY "itemID" LIMIT 1"#,
                &[&system, &GROUP_PLANET],
            )
            .await?
        else {
            return Ok(());
        };
        if self.pay(c, db, bot_corps, sov::OUTPOST_COST, "an outpost").await?.is_none() {
            return Ok(());
        }
        let platform = PLATFORMS.iter().find(|(r, _)| *r == self.race).map_or(PLATFORMS[2].1, |(_, t)| *t);
        let now = crate::bulkdata::filetime_now();
        let delay = crate::pos::delays(&*db, &[platform]).await?.get(&(platform as i64)).map_or(0, |d| d[0]);
        let stamp = timer_start(now, delay, self.speed);
        let f = |k: usize| crate::svc::rows::float(&planet, k).unwrap_or(0.0);
        let reach = f(3) + OFF_PLANET;
        let dir = crate::ballpark::Vec3::new(-0.6, 0.15, 0.78).unit();
        let pos = (f(0) + dir.x * reach, f(1) + dir.y * reach, f(2) + dir.z * reach);
        let tx = db.transaction().await?;
        tx.execute(
            r#"WITH t AS (INSERT INTO entity ("itemID","itemName","typeID","ownerID","locationID","flag","contraband","singleton","quantity","x","y","z")
                          VALUES (nextval('dyn_id_seq'), '', $1, $2, $3, 0, 0, 1, 1, $4, $5, $6) RETURNING "itemID")
               INSERT INTO "posStructures" ("itemID","state","stateTimestamp","claim","allowCorp","allowAlliance")
               SELECT "itemID", $7, $8, 0, 1, 1 FROM t"#,
            &[&platform, &self.executor, &system, &pos.0, &pos.1, &pos.2, &state::ANCHORED, &stamp],
        )
        .await?;
        tx.commit().await?;
        sov::show_towers(db, world, system).await;
        let name = type_name(db, platform).await;
        let place = sov::system_name(db, system).await;
        info!("bots: alliance {} anchoring a {name} in {place}", self.alliance);
        self.say(db, Office::Stations, format!("Anchoring an {name} in {place}. Once it's anchored ({}) we'll haul the materials in and build.", left_text(stamp, delay, now))).await;
        Ok(())
    }

    /// Takes `cost` from the richest bot corporation of the alliance that
    /// can pay it; None (and the alliance told, now and then) if none can.
    async fn pay(&self, c: &mut Campaigns, db: &Client, bot_corps: &[i32], cost: f64, what: &str) -> Result<Option<i32>, tokio_postgres::Error> {
        let paid = db
            .query_opt(
                r#"UPDATE "corporation" SET "balance" = "balance" - $2
                    WHERE "corporationID" = (SELECT "corporationID" FROM "corporation"
                                               WHERE "corporationID" = ANY($1) AND "balance" >= $2
                                               ORDER BY "balance" DESC LIMIT 1)
                  RETURNING "corporationID""#,
                &[&bot_corps, &cost],
            )
            .await?
            .and_then(|r| int(&r, 0))
            .map(|v| v as i32);
        if paid.is_some() {
            c.broke.remove(&self.alliance);
        } else if c.broke.get(&self.alliance).is_none_or(|at| at.elapsed() >= BROKE_EVERY) {
            c.broke.insert(self.alliance, Instant::now());
            self.say(db, Office::Accountant, format!("We're short of ISK for {what} ({:.0}M ISK). We'll carry on when our wallets can stand it.", cost / 1.0e6)).await;
        }
        Ok(paid)
    }
}

async fn refund(db: &Client, corp: i32, cost: f64) -> Result<(), tokio_postgres::Error> {
    db.execute(r#"UPDATE "corporation" SET "balance" = "balance" + $2 WHERE "corporationID" = $1"#, &[&corp, &cost]).await?;
    Ok(())
}

/// The alliance's towers and platforms.
async fn pieces(db: &Client, alliance: i32) -> Result<Vec<Piece>, tokio_postgres::Error> {
    let rows = db
        .query(
            r#"SELECT p."itemID", e."locationID", COALESCE(p."state", 0), p."stateTimestamp", e."typeID", e."ownerID", p."moonID", t."groupID"
                 FROM "posStructures" p JOIN entity e ON e."itemID" = p."itemID"
                 JOIN "invTypes" t ON t."typeID" = e."typeID"
                 JOIN "corporation" co ON co."corporationID" = e."ownerID"
                WHERE co."allianceID" = $1 AND t."groupID" IN ($2, $3) AND p."controlTowerID" IS NULL"#,
            &[&alliance, &GROUP_TOWER, &GROUP_PLATFORM],
        )
        .await?;
    Ok(rows
        .iter()
        .map(|r| Piece {
            id: int(r, 0).unwrap_or(0),
            system: int(r, 1).unwrap_or(0) as i32,
            state: int(r, 2).unwrap_or(0) as i16,
            stamp: int(r, 3),
            type_id: int(r, 4).unwrap_or(0) as i32,
            owner: int(r, 5).unwrap_or(0) as i32,
            moon: int(r, 6).map(|v| v as i32),
            group: int(r, 7).unwrap_or(0) as i32,
        })
        .collect())
}

/// Systems someone else claims: another alliance's tower there, or its
/// sovereignty on the map.
async fn taken_systems(db: &Client, alliance: i32) -> Result<HashSet<i32>, tokio_postgres::Error> {
    let rows = db
        .query(
            r#"SELECT e."locationID" FROM "posStructures" p JOIN entity e ON e."itemID" = p."itemID"
                 LEFT JOIN "corporation" co ON co."corporationID" = e."ownerID"
                WHERE p."claim" = 1 AND COALESCE(co."allianceID", 0) <> $1
               UNION
               SELECT "solarSystemID" FROM "mapSolarSystems" WHERE "allianceID" IS NOT NULL AND "allianceID" <> $1"#,
            &[&alliance],
        )
        .await?;
    Ok(rows.iter().filter_map(|r| int(r, 0).map(|v| v as i32)).collect())
}

/// The alliance's executor corporation, its holder, and its name.
async fn holder(db: &Client, alliance: i32) -> Option<(i32, String)> {
    let sql = r#"SELECT al."executorCorpID", COALESCE(co."corporationName", '') FROM alliances al
                   JOIN "corporation" co ON co."corporationID" = al."executorCorpID" WHERE al."allianceID" = $1"#;
    let r = db.query_opt(sql, &[&alliance]).await.ok().flatten()?;
    Some((int(&r, 0)? as i32, text(&r, 1).unwrap_or_default()))
}

async fn character_name(db: &Client, id: i64) -> String {
    let sql = r#"SELECT "itemName" FROM entity WHERE "itemID" = $1"#;
    db.query_opt(sql, &[&(id as i32)]).await.ok().flatten().and_then(|r| text(&r, 0)).unwrap_or_default()
}

async fn type_name(db: &Client, id: i32) -> String {
    let sql = r#"SELECT "typeName" FROM "invTypes" WHERE "typeID" = $1"#;
    db.query_opt(sql, &[&id]).await.ok().flatten().and_then(|r| text(&r, 0)).unwrap_or_else(|| "control tower".into())
}

async fn moon_name(db: &Client, id: Option<i32>) -> String {
    let Some(id) = id else { return "its moon".into() };
    let sql = r#"SELECT "itemName" FROM "mapDenormalize" WHERE "itemID" = $1"#;
    db.query_opt(sql, &[&id]).await.ok().flatten().and_then(|r| text(&r, 0)).unwrap_or_else(|| "a moon".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bot_timers_run_at_the_bots_speed() {
        let (now, delay) = (1_000_000_000_000i64, 1_800 * SECOND_FT);
        // At 1x a 30-minute step ends 30 minutes from now; at 4x, 7.5.
        assert_eq!(timer_start(now, delay, 1.0) + delay - now, delay);
        assert_eq!(timer_start(now, delay, 4.0) + delay - now, delay / 4);
        assert_eq!(left_text(timer_start(now, delay, 1.0), delay, now), "30m 00s");
    }
}
