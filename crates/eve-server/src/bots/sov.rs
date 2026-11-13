//! Null-sec sovereignty for bot alliances: they hold space as players do,
//! with control towers that claim it (svc::starbase::sovereignty decides).
//!
//! - Settling (`establish`, once): each null-sec alliance whose home region
//!   no NPC faction holds anchors a large control tower of its race at a moon
//!   in each of its systems, a block spreading out from its home, one system
//!   for about every 25 pilots. Their claims are dated back past the claim
//!   delay, so the map shows the space held from the start.
//! - Upkeep (`upkeep`, every politics tick): towers are kept in 30 days of
//!   fuel and some strontium. Neighbours at war plan sieges of each other's
//!   border systems, announced in the news `SIEGE_NOTICE` ahead
//!   (`battle` fights them). A siege that succeeds (`conclude`) destroys the
//!   defender's towers there and anchors the attacker's, whose claim then
//!   stands its delay (scaled by the bots' clock) before the system changes
//!   hands. An alliance short of systems at home anchors in unclaimed ones.
//!
//! - Player alliances: an alliance that has taken bot corporations in has
//!   those bots claim space for it the same way (`player_claimants`): a home
//!   region of its own (where its towers already stand, or free null-sec),
//!   towers anchored and fuelled by its bot corporations, defended by all its
//!   bots (`defense`), and wars and sieges with its bot neighbours.
//! - Growth: alliances holding more space draw more corporations
//!   (recruiting in `politics`, applications in `relations`, and free bot
//!   corporations joining here), and an alliance with enough settled
//!   systems founds an outpost in its space.
//!
//! Bot towers are real: players can see them, shoot them and reinforce them.

use std::collections::{HashMap, HashSet, VecDeque};

use tokio_postgres::Client;
use tracing::info;

use super::behaviour::Atlas;
use super::politics::{Politics, KIND_NULL_ALLIANCE};
use crate::combat::Rng;
use crate::svc::rows::{float, int, text};

/// One system for this many pilots, at least `MIN_SYSTEMS`.
const PILOTS_PER_SYSTEM: u32 = 25;
const MIN_SYSTEMS: usize = 3;
/// At most this share of a region's systems goes to its alliance.
const MAX_SHARE: f64 = 0.6;
/// Large control towers by race (Caldari, Minmatar, Amarr, Gallente).
const TOWERS: [(i64, i32); 4] = [(1, 16213), (2, 16214), (4, 12235), (8, 12236)];
const GROUP_MOON: i32 = 8;
/// A tower stands this far off its moon's surface (the moon rule is 100 km).
const OFF_MOON: f64 = 30_000.0;
/// Fuel held: 30 days of hourly periods, topped up below a quarter of that.
const FUEL_PERIODS: i64 = 30 * 24;
const STRONTIUM: i64 = 3_000;
const PURPOSE_ONLINE: i32 = 1;
const PURPOSE_REINFORCE: i32 = 4;
/// Each war's chance, per politics tick, of a siege.
const SIEGE_CHANCE: f64 = 0.08;
/// How long before a siege it is announced, in seconds of the bots' clock.
pub const SIEGE_NOTICE: u32 = 15 * 60;
/// An alliance's chance, per tick, to anchor in an unclaimed home system.
const EXPAND_CHANCE: f64 = 0.05;
const SECOND_FT: i64 = 10_000_000;
/// What an alliance pays its executor's wallet: to anchor a tower, and to
/// keep each in fuel (each politics tick, about 5M ISK a day). An alliance
/// that cannot pay lets its towers run dry.
pub const TOWER_COST: f64 = 150_000_000.0;
const UPKEEP_PER_TICK: f64 = 35_000.0;
/// Treasury at which an alliance fields twice the ships its pilots alone would.
const RICH: f64 = 5_000_000_000.0;
/// Settled systems (sovereignty on the map) an alliance holds before it
/// founds an outpost, and its chance each tick to found one where it has none.
const OUTPOST_AFTER: usize = 4;
const OUTPOST_CHANCE: f64 = 0.02;
const OUTPOST_COST: f64 = 1_000_000_000.0;
/// A free bot corporation's chance, each tick, to join a bot alliance; the
/// alliances holding more space are likelier.
const JOIN_CHANCE: f64 = 0.05;

/// What happened, for the news.
pub type News = (i64, String);

/// A siege planned or under way: who, where, and when it begins.
pub struct Siege {
    pub war: i32,
    pub attacker: i32,
    pub defender: i32,
    /// The attacker's executor corporation and its race, for its tower.
    pub attacker_exec: i32,
    pub attacker_race: i64,
    /// Pilots in each alliance, for the odds when nobody watches.
    pub attackers: u32,
    pub defenders: u32,
    pub system: i32,
    pub names: SiegeNames,
    /// When it begins: on the bots' clock, and as FILETIME for the dashboard.
    pub starts: u32,
    pub starts_at: i64,
    /// The fleets, once it is fought where someone watches.
    pub fight: Option<super::battle::Fight>,
    /// Asked for by hand (`/bots battle`): fought at these stakes, with
    /// every kind of capital ship on both sides.
    pub showcase: Option<super::battle::Stakes>,
    /// Fought in open space, round a beacon by a watching pilot, where
    /// nobody holds a tower: it decides no sovereignty.
    pub field: bool,
}

#[derive(Debug, Clone)]
pub struct SiegeNames {
    pub attacker: String,
    pub defender: String,
    pub system: String,
    pub region: String,
    pub security: f64,
}

struct Place {
    system: i32,
    moon: i32,
    pos: (f64, f64, f64),
}

/// The bot alliances' null-sec alliances, with their home region (only those
/// no NPC faction holds), executor corporation, race and pilots.
async fn claimants(db: &Client, p: &Politics) -> Result<Vec<(i32, i32, i32, i64, u32)>, tokio_postgres::Error> {
    let mut out = Vec::new();
    for (id, a) in p.alliances.iter().filter(|(_, a)| a.kind == KIND_NULL_ALLIANCE) {
        let Some(region) = a.region else { continue };
        let sql = r#"SELECT al."executorCorpID", co."raceID" FROM alliances al
                       JOIN "corporation" co ON co."corporationID" = al."executorCorpID"
                       JOIN "mapRegions" r ON r."regionID" = $2
                      WHERE al."allianceID" = $1 AND r."factionID" IS NULL"#;
        let Some(r) = db.query_opt(sql, &[id, &region]).await? else { continue };
        let pilots: u32 = p.corps.values().filter(|c| c.alliance == Some(*id)).map(|c| c.members).sum();
        out.push((*id, region, int(&r, 0).unwrap_or(0) as i32, int(&r, 1).unwrap_or(4), pilots));
    }
    out.sort_unstable();
    Ok(out)
}

/// A moon in the system to put a tower beside, and where.
async fn spot(db: &Client, system: i32, rng: &mut Rng) -> Result<Option<Place>, tokio_postgres::Error> {
    let sql = r#"SELECT m."itemID", m."x", m."y", m."z", COALESCE(m."radius", 1000)
                   FROM "mapDenormalize" m WHERE m."solarSystemID" = $1 AND m."groupID" = $2
                    AND NOT EXISTS (SELECT 1 FROM "posStructures" p WHERE p."moonID" = m."itemID"
                                      AND (p.state <> 0 OR p."stateTimestamp" IS NOT NULL))
                  ORDER BY m."itemID""#;
    let moons = db.query(sql, &[&system, &GROUP_MOON]).await?;
    let Some(m) = moons.get(rng.below(moons.len().max(1))) else { return Ok(None) };
    let dir = rng.in_sphere(1.0);
    let dir = if dir.len() > 1e-6 { dir.unit() } else { crate::ballpark::Vec3::new(1.0, 0.0, 0.0) };
    let reach = float(m, 4).unwrap_or(1_000.0) + OFF_MOON;
    let at = (float(m, 1).unwrap_or(0.0) + dir.x * reach, float(m, 2).unwrap_or(0.0) + dir.y * reach, float(m, 3).unwrap_or(0.0) + dir.z * reach);
    Ok(Some(Place { system, moon: int(m, 0).unwrap_or(0) as i32, pos: at }))
}

/// Anchors and onlines a claiming tower; its claim counts from `claim_since`.
async fn anchor(db: &mut Client, owner: i32, tower_type: i32, place: &Place, claim_since: i64) -> Result<bool, tokio_postgres::Error> {
    let tx = db.transaction().await?;
    if !crate::svc::starbase::reserve_moon(&tx, place.moon).await? {
        return Ok(false);
    }
    let now = crate::bulkdata::filetime_now();
    tx
        .execute(
            r#"WITH t AS (INSERT INTO entity ("itemID","itemName","typeID","ownerID","locationID","flag","contraband","singleton","quantity","x","y","z")
                          VALUES (nextval('dyn_id_seq'), '', $1, $2, $3, 0, 0, 1, 1, $4, $5, $6) RETURNING "itemID")
               INSERT INTO "posStructures" ("itemID","state","stateTimestamp","moonID","lastFuel","claim","claimSince","allowCorp","allowAlliance")
               SELECT "itemID", 4, $7, $8, $7, 1, $9, 1, 1 FROM t"#,
            &[&tower_type, &owner, &place.system, &place.pos.0, &place.pos.1, &place.pos.2, &now, &place.moon, &claim_since],
        )
        .await?;
    tx.commit().await?;
    Ok(true)
}

/// Claims up to `want` systems of `region` outward from its first station
/// system, skipping `claimed` ones. Returns how many it claimed.
#[allow(clippy::too_many_arguments)]
async fn settle(
    db: &mut Client,
    atlas: &Atlas,
    rng: &mut Rng,
    region: i32,
    executor: i32,
    tower_type: i32,
    want: usize,
    claimed: &mut HashSet<i32>,
    since: i64,
) -> Result<usize, tokio_postgres::Error> {
    let in_region: Vec<i32> = atlas.systems.iter().filter(|(_, s)| s.region == region && s.security <= 0.0).map(|(id, _)| *id).collect();
    let mut sorted = in_region.clone();
    sorted.sort_unstable();
    let home = sorted.iter().copied().find(|s| atlas.systems.get(s).is_some_and(|x| !x.stations.is_empty()) && !claimed.contains(s))
        .or_else(|| sorted.iter().copied().find(|s| !claimed.contains(s)));
    let Some(home) = home else { return Ok(0) };
    let mut order: Vec<i32> = Vec::new();
    let mut seen = HashSet::from([home]);
    let mut queue = VecDeque::from([home]);
    while let Some(s) = queue.pop_front() {
        order.push(s);
        for n in atlas.systems.get(&s).map(|x| x.neighbours.clone()).unwrap_or_default() {
            if atlas.systems.get(&n).is_some_and(|x| x.region == region && x.security <= 0.0) && seen.insert(n) {
                queue.push_back(n);
            }
        }
    }
    let mut held = 0;
    for system in order {
        if held >= want {
            break;
        }
        if claimed.contains(&system) {
            continue;
        }
        let Some(place) = spot(db, system, rng).await? else { continue };
        if !anchor(db, executor, tower_type, &place, since).await? {
            continue;
        }
        claimed.insert(system);
        held += 1;
    }
    Ok(held)
}

/// Player alliances with bot corporations in them, as claimants: (alliance,
/// home region, the bot corporation that anchors for it, its race, its bot
/// pilots). The home is where its towers stand, else a free null-sec region
/// no NPC faction or other alliance calls home, kept in `p.player_sov`.
async fn player_claimants(db: &Client, p: &mut Politics) -> Result<Vec<(i32, i32, i32, i64, u32)>, tokio_postgres::Error> {
    let mut by_alliance: HashMap<i32, Vec<(i32, u32)>> = HashMap::new();
    for (id, c) in &p.corps {
        if let Some(a) = c.alliance.filter(|a| !p.alliances.contains_key(a)) {
            if c.members > 0 {
                by_alliance.entry(a).or_default().push((*id, c.members));
            }
        }
    }
    let mut out = Vec::new();
    let mut ids: Vec<i32> = by_alliance.keys().copied().collect();
    ids.sort_unstable();
    p.player_sov.retain(|a, _| by_alliance.contains_key(a));
    for alliance in ids {
        let corps = &by_alliance[&alliance];
        let pilots: u32 = corps.iter().map(|(_, n)| n).sum();
        let &(executor, _) = corps.iter().max_by_key(|(id, n)| (*n, -*id)).expect("an alliance with bot corporations");
        let race = db
            .query_opt(r#"SELECT COALESCE("raceID", 4) FROM "corporation" WHERE "corporationID" = $1"#, &[&executor])
            .await?
            .and_then(|r| int(&r, 0))
            .unwrap_or(4);
        let region = match p.player_sov.get(&alliance) {
            Some(r) => *r,
            None => {
                // Where its towers already stand, if anywhere in null-sec.
                let sql = r#"SELECT s."regionID", count(*) FROM "posStructures" p JOIN entity e ON e."itemID" = p."itemID"
                               JOIN "corporation" co ON co."corporationID" = e."ownerID"
                               JOIN "mapSolarSystems" s ON s."solarSystemID" = e."locationID"
                               JOIN "mapRegions" r ON r."regionID" = s."regionID"
                              WHERE co."allianceID" = $1 AND s."security" <= 0 AND r."factionID" IS NULL
                              GROUP BY 1 ORDER BY 2 DESC LIMIT 1"#;
                let existing = db.query_opt(sql, &[&alliance]).await?.and_then(|r| int(&r, 0)).map(|v| v as i32);
                let taken: HashSet<i32> = p.alliances.values().filter_map(|a| a.region).chain(p.player_sov.values().copied()).collect();
                let free = match existing {
                    Some(r) => Some(r),
                    None => {
                        let sql = r#"SELECT r."regionID" FROM "mapRegions" r
                                      WHERE r."factionID" IS NULL
                                        AND EXISTS (SELECT 1 FROM "mapSolarSystems" s WHERE s."regionID" = r."regionID" AND s."security" <= 0)
                                      ORDER BY r."regionID""#;
                        let all: Vec<i32> = db.query(sql, &[]).await?.iter().filter_map(|r| int(r, 0).map(|v| v as i32)).filter(|r| !taken.contains(r)).collect();
                        all.get(alliance.unsigned_abs() as usize % all.len().max(1)).copied()
                    }
                };
                let Some(r) = free else { continue };
                info!("bots: player alliance {alliance}'s bot corporations make {r} their home");
                p.player_sov.insert(alliance, r);
                r
            }
        };
        out.push((alliance, region, executor, race, pilots));
    }
    Ok(out)
}

/// Settles null-sec: towers for every null-sec bot alliance with none yet.
pub async fn establish(db: &mut Client, atlas: &Atlas, p: &Politics, rng: &mut Rng, claim_delay_secs: i64) -> Result<Vec<News>, tokio_postgres::Error> {
    let have: HashSet<i64> = db
        .query(
            r#"SELECT DISTINCT co."allianceID" FROM "posStructures" p JOIN entity e ON e."itemID" = p."itemID"
                 JOIN "corporation" co ON co."corporationID" = e."ownerID"
                 JOIN "botGroups" g ON g."ownerID" = co."corporationID""#,
            &[],
        )
        .await?
        .iter()
        .filter_map(|r| int(r, 0))
        .collect();
    let now = crate::bulkdata::filetime_now();
    let settled = now - (claim_delay_secs + 60) * SECOND_FT;
    let taken: HashSet<i32> = db
        .query(r#"SELECT "solarSystemID" FROM "mapSolarSystems" WHERE "allianceID" IS NOT NULL"#, &[])
        .await?
        .iter()
        .filter_map(|r| int(r, 0).map(|v| v as i32))
        .collect();
    let mut claimed: HashSet<i32> = taken;
    let mut towers = 0;
    let mut news = Vec::new();
    for (alliance, region, executor, race, pilots) in claimants(db, p).await? {
        if have.contains(&(alliance as i64)) {
            continue;
        }
        let size = atlas.systems.values().filter(|s| s.region == region && s.security <= 0.0).count();
        let want = ((pilots / PILOTS_PER_SYSTEM) as usize).max(MIN_SYSTEMS).min((size as f64 * MAX_SHARE) as usize).max(1);
        let tower_type = TOWERS.iter().find(|(r, _)| *r == race).map_or(TOWERS[2].1, |(_, t)| *t);
        let held = settle(db, atlas, rng, region, executor, tower_type, want, &mut claimed, settled).await?;
        towers += held;
        let name = alliance_name(db, alliance).await;
        news.push((now, format!("{name} claims {held} systems of {}", region_name(db, region).await)));
    }
    if towers > 0 {
        refuel(db).await?;
        info!("bots: {towers} control towers anchored across null-sec");
    }
    Ok(news)
}

/// Keeps every bot tower in fuel: a top-up whenever one runs below a quarter
/// of 30 days' worth.
pub async fn refuel(db: &Client) -> Result<u64, tokio_postgres::Error> {
    let sql = r#"WITH towers AS (
                     SELECT p."itemID", e."typeID", e."ownerID" FROM "posStructures" p
                       JOIN entity e ON e."itemID" = p."itemID"
                       JOIN "botGroups" g ON g."ownerID" = e."ownerID"
                       JOIN "corporation" co ON co."corporationID" = e."ownerID"
                      WHERE p."controlTowerID" IS NULL AND p."state" IN (2, 3, 4, 5) AND co."balance" > 0),
                 needs AS (
                     SELECT t."itemID", t."ownerID", r."resourceTypeID" AS kind,
                            CASE WHEN r."purpose" = $2 THEN $4::bigint ELSE r."quantity"::bigint * $3 END AS target
                       FROM towers t JOIN "invControlTowerResources" r ON r."controlTowerTypeID" = t."typeID"
                      WHERE r."purpose" IN ($1, $2) AND r."minSecurityLevel" IS NULL AND r."factionID" IS NULL),
                 held AS (
                     SELECT e."locationID", e."typeID", sum(e."quantity")::bigint AS q FROM entity e
                      WHERE e."locationID" IN (SELECT "itemID" FROM towers) GROUP BY 1, 2)
               INSERT INTO entity ("itemID","itemName","typeID","ownerID","locationID","flag","contraband","singleton","quantity")
               SELECT nextval('dyn_id_seq'), '', n.kind, n."ownerID", n."itemID", 0, 0, 0, (n.target - COALESCE(h.q, 0))::int
                 FROM needs n LEFT JOIN held h ON h."locationID" = n."itemID" AND h."typeID" = n.kind
                WHERE COALESCE(h.q, 0) * 4 < n.target"#;
    db.execute(sql, &[&PURPOSE_ONLINE, &PURPOSE_REINFORCE, &FUEL_PERIODS, &STRONTIUM]).await
}

/// Every politics tick: fuel, sieges planned between warring neighbours
/// (added to `sieges`), and expansion into empty home systems. Returns the
/// news.
#[allow(clippy::too_many_arguments)]
pub async fn upkeep(
    db: &mut Client,
    world: &crate::world::World,
    atlas: &Atlas,
    p: &mut Politics,
    ids: &mut [super::presence::Identity],
    watched: &HashSet<i32>,
    sieges: &mut Vec<Siege>,
    rng: &mut Rng,
    clock: u32,
    claim_delay_secs: i64,
    speed: f64,
) -> Result<Vec<News>, tokio_postgres::Error> {
    // Fuel costs money: each bot corporation pays for its towers.
    db.execute(
        r#"UPDATE "corporation" co SET "balance" = co."balance" - t.n * $1
             FROM (SELECT e."ownerID", count(*)::float8 AS n FROM "posStructures" p JOIN entity e ON e."itemID" = p."itemID"
                    JOIN "botGroups" g ON g."ownerID" = e."ownerID" WHERE p."controlTowerID" IS NULL GROUP BY 1) t
            WHERE co."corporationID" = t."ownerID""#,
        &[&UPKEEP_PER_TICK],
    )
    .await?;
    refuel(db).await?;
    let now = crate::bulkdata::filetime_now();
    // A new claim waits out the delay, run at the bots' speed.
    let delay = claim_delay_secs as f64;
    let claim_since = now - ((delay - delay / speed.max(1.0)) as i64) * SECOND_FT;
    let mut news = Vec::new();
    let mut claimants = claimants(db, p).await?;
    let players = player_claimants(db, p).await?;
    let player_ids: HashSet<i32> = players.iter().map(|c| c.0).collect();
    claimants.extend(players);
    let info: HashMap<i32, (i32, i32, i64, u32)> = claimants.iter().map(|(a, r, e, race, n)| (*a, (*r, *e, *race, *n))).collect();
    // Which alliances have claiming towers where: what sieges aim at, rather
    // than the map's sovereignty, which follows the towers only later.
    let mut towers: HashMap<i32, HashSet<i32>> = HashMap::new();
    let sql = r#"SELECT e."locationID", COALESCE(co."allianceID", 0) FROM "posStructures" p JOIN entity e ON e."itemID" = p."itemID"
                   LEFT JOIN "corporation" co ON co."corporationID" = e."ownerID" WHERE p."claim" = 1"#;
    for r in db.query(sql, &[]).await? {
        if let (Some(system), Some(alliance)) = (int(&r, 0), int(&r, 1)) {
            towers.entry(system as i32).or_default().insert(alliance as i32);
        }
    }
    let has = |towers: &HashMap<i32, HashSet<i32>>, system: &i32, alliance: i32| towers.get(system).is_some_and(|t| t.contains(&alliance));

    // Space held, by alliance: what draws recruits.
    let mut held: HashMap<i32, u32> = HashMap::new();
    for owners in towers.values() {
        for a in owners {
            *held.entry(*a).or_default() += 1;
        }
    }
    p.held = held;

    // A player alliance's bots with no space yet claim a first block of
    // its home region.
    for (alliance, region, executor, race, _) in claimants.iter().filter(|c| player_ids.contains(&c.0)) {
        if p.held.get(alliance).copied().unwrap_or(0) > 0 {
            continue;
        }
        let mut claimed: HashSet<i32> = towers.iter().filter(|(_, t)| !t.is_empty()).map(|(s, _)| *s).collect();
        claimed.extend(watched.iter().copied());
        let tower_type = TOWERS.iter().find(|(r, _)| r == race).map_or(TOWERS[2].1, |(_, t)| *t);
        let before = claimed.clone();
        let got = settle(db, atlas, rng, *region, *executor, tower_type, MIN_SYSTEMS, &mut claimed, claim_since).await?;
        if got > 0 {
            refuel(db).await?;
            p.held.insert(*alliance, got as u32);
            for system in claimed.difference(&before) {
                towers.entry(*system).or_default().insert(*alliance);
            }
            news.push((now, format!("{}'s corporations anchor control towers in {got} systems of {} to claim it for {}",
                alliance_name(db, *alliance).await, region_name(db, *region).await, alliance_name(db, *alliance).await)));
        }
    }

    // Outposts: an alliance holding enough settled space founds one where it
    // has none.
    for (alliance, _, executor, race, _) in &claimants {
        if rng.unit() >= OUTPOST_CHANCE * super::sov_warfare().expand_rate() {
            continue;
        }
        let sql = r#"SELECT s."solarSystemID",
                            EXISTS (SELECT 1 FROM "staStations" st WHERE st."solarSystemID" = s."solarSystemID"),
                            (SELECT count(*) FROM "mapSolarSystemJumps" j JOIN "mapSolarSystems" n ON n."solarSystemID" = j."toSolarSystemID"
                              WHERE j."fromSolarSystemID" = s."solarSystemID" AND n."allianceID" = $1)
                       FROM "mapSolarSystems" s WHERE s."allianceID" = $1"#;
        let rows = db.query(sql, &[alliance]).await?;
        if rows.len() < OUTPOST_AFTER {
            continue;
        }
        fn has_station(r: &tokio_postgres::Row) -> bool {
            r.try_get::<_, bool>(1).unwrap_or(true)
        }
        // One outpost for every OUTPOST_AFTER systems held, in the best-connected place without a station.
        let stations = rows.iter().filter(|r| has_station(r)).count();
        if stations * OUTPOST_AFTER >= rows.len() {
            continue;
        }
        let Some(best) = rows.iter().filter(|r| !has_station(r)).max_by_key(|r| (int(r, 2).unwrap_or(0), -int(r, 0).unwrap_or(0))) else { continue };
        let system = int(best, 0).unwrap_or(0);
        let paid = db
            .execute(r#"UPDATE "corporation" SET "balance" = "balance" - $2 WHERE "corporationID" = $1 AND "balance" >= $2"#, &[executor, &OUTPOST_COST])
            .await?;
        if paid == 0 {
            continue;
        }
        match crate::svc::outpost::found_for_bots(db, world, system, *executor as i64, *race).await? {
            Some((_, name)) => news.push((now, format!("{} founded an outpost: {name}", alliance_name(db, *alliance).await))),
            None => {
                db.execute(r#"UPDATE "corporation" SET "balance" = "balance" + $2 WHERE "corporationID" = $1"#, &[executor, &OUTPOST_COST]).await?;
            }
        }
    }

    // Free bot corporations join the bot alliances holding the most space.
    if rng.unit() < JOIN_CHANCE {
        let free: Vec<i32> = {
            let mut v: Vec<i32> = p.corps.iter().filter(|(_, c)| c.alliance.is_none() && c.members > 0 && c.kind == super::politics::KIND_COMBAT_CORP).map(|(id, _)| *id).collect();
            v.sort_unstable();
            v
        };
        let mut holders: Vec<(i32, u32)> = p.held.iter().filter(|(a, n)| p.alliances.get(a).is_some_and(|x| x.kind == KIND_NULL_ALLIANCE) && **n > 0).map(|(a, n)| (*a, *n)).collect();
        holders.sort_unstable();
        let total: u32 = holders.iter().map(|(_, n)| n).sum();
        if let (false, true) = (free.is_empty(), total > 0) {
            let corp = free[rng.below(free.len())];
            let mut roll = rng.below(total as usize) as u32;
            let to = holders.iter().find(|(_, n)| {
                if roll < *n {
                    return true;
                }
                roll -= n;
                false
            }).map(|(a, _)| *a);
            if let Some(to) = to {
                super::events::move_corp(db, p, ids, corp, to).await?;
                news.push((now, format!("A corporation joins {}, drawn by its {} systems", alliance_name(db, to).await, p.held.get(&to).copied().unwrap_or(0))));
            }
        }
    }

    // Sieges: one at a time in each war, announced ahead; how many in all
    // and how often as the dashboard's sovereignty warfare setting says.
    let warfare = super::sov_warfare();
    let wars: Vec<(i32, i32, i32)> = p.wars.iter().map(|w| (w.a, w.b, w.id)).collect();
    for (a, b, war) in wars {
        if sieges.len() >= warfare.max_sieges() {
            break;
        }
        let (Some(&(_, attacker_exec, attacker_race, attackers)), Some(&(_, _, _, defenders))) = (info.get(&a), info.get(&b)) else { continue };
        if sieges.iter().any(|s| s.war == war) || rng.unit() >= SIEGE_CHANCE * warfare.siege_rate() || !p.in_prime(a) {
            continue;
        }
        // Rich alliances field more ships than their numbers alone would.
        let wealth = |side: i32| 1.0 + (p.treasury.get(&side).copied().unwrap_or(0.0) / RICH).clamp(0.0, 1.0);
        let (attackers, defenders) = ((attackers as f64 * wealth(a)) as u32, (defenders as f64 * wealth(b)) as u32);
        // A defender's system next to the attacker's space, else any of theirs.
        let mut theirs: Vec<i32> = towers.keys().copied().filter(|s| has(&towers, s, b) && !sieges.iter().any(|x| x.system == *s)).collect();
        theirs.sort_unstable();
        let border: Vec<i32> = theirs
            .iter()
            .copied()
            .filter(|s| atlas.systems.get(s).is_some_and(|x| x.neighbours.iter().any(|nb| has(&towers, nb, a))))
            .collect();
        let pool = if border.is_empty() { &theirs } else { &border };
        let Some(&target) = pool.get(rng.below(pool.len().max(1))) else { continue };
        let names = SiegeNames {
            attacker: alliance_name(db, a).await,
            defender: alliance_name(db, b).await,
            system: system_name(db, target).await,
            region: region_name(db, atlas.systems.get(&target).map_or(0, |x| x.region)).await,
            security: atlas.systems.get(&target).map_or(0.0, |x| x.security as f64),
        };
        let starts_at = now + (SIEGE_NOTICE as f64 / speed.max(0.001)) as i64 * SECOND_FT;
        info!("bots: {} will besiege {} ({}) of {}", names.attacker, names.system, names.region, names.defender);
        news.push((now, format!("{} are massing a fleet to besiege {} in {}, held by {}", names.attacker, names.system, names.region, names.defender)));
        sieges.push(Siege {
            war,
            attacker: a,
            defender: b,
            attacker_exec,
            attacker_race,
            attackers,
            defenders,
            system: target,
            names,
            starts: clock + SIEGE_NOTICE,
            starts_at,
            fight: None,
            showcase: None,
            field: false,
        });
    }

    // Expansion into empty home systems.
    for (alliance, region, executor, race, pilots) in &claimants {
        if rng.unit() >= EXPAND_CHANCE * warfare.expand_rate() {
            continue;
        }
        let size = atlas.systems.values().filter(|s| s.region == *region && s.security <= 0.0).count();
        let want = ((*pilots / PILOTS_PER_SYSTEM) as usize).max(MIN_SYSTEMS).min((size as f64 * MAX_SHARE) as usize);
        let have = towers.values().filter(|t| t.contains(alliance)).count();
        if have >= want {
            continue;
        }
        let empty: Vec<i32> = atlas
            .systems
            .iter()
            .filter(|(id, s)| s.region == *region && s.security <= 0.0 && towers.get(id).is_none_or(|t| t.is_empty()) && !watched.contains(id))
            .map(|(id, _)| *id)
            .collect();
        let Some(&system) = empty.get(rng.below(empty.len().max(1))) else { continue };
        let Some(place) = spot(db, system, rng).await? else { continue };
        let tower_type = TOWERS.iter().find(|(r, _)| r == race).map_or(TOWERS[2].1, |(_, t)| *t);
        // A tower is bought out of the executor's wallet, or not at all.
        let paid = db.execute(r#"UPDATE "corporation" SET "balance" = "balance" - $2 WHERE "corporationID" = $1 AND "balance" >= $2"#, &[executor, &TOWER_COST]).await?;
        if paid == 0 {
            continue;
        }
        if anchor(db, *executor, tower_type, &place, claim_since).await? {
            towers.entry(system).or_default().insert(*alliance);
            news.push((now, format!("{} anchored a control tower in {}", alliance_name(db, *alliance).await, system_name(db, system).await)));
        } else {
            db.execute(r#"UPDATE "corporation" SET "balance" = "balance" + $2 WHERE "corporationID" = $1"#,
                &[executor, &TOWER_COST]).await?;
        }
    }
    Ok(news)
}

/// How a siege ended: `won` by the attacker or not, with `lost` ships on
/// each side. A won siege destroys the defender's towers in the system and
/// anchors the attacker's, its claim to stand the delay (scaled by the bots'
/// clock). Returns the news.
#[allow(clippy::too_many_arguments)]
pub async fn conclude(
    db: &mut Client,
    p: &mut Politics,
    s: &Siege,
    won: bool,
    lost: (u32, u32),
    rng: &mut Rng,
    claim_delay_secs: i64,
    speed: f64,
) -> Result<News, tokio_postgres::Error> {
    let now = crate::bulkdata::filetime_now();
    if let Some(w) = p.wars.iter_mut().find(|w| w.id == s.war) {
        w.kills += lost.0 + lost.1;
    }
    p.lost += (lost.0 + lost.1) as u64;
    let n = &s.names;
    if !won {
        info!("bots: {} driven back from {} by {} ({} and {} ships lost)", n.attacker, n.system, n.defender, lost.0, lost.1);
        return Ok((now, format!("{} were driven back from {} by {}", n.attacker, n.system, n.defender)));
    }
    let gone = destroy_towers(db, s.system, s.defender).await?;
    // The victors buy their tower with what is left in their executor's wallet.
    db.execute(r#"UPDATE "corporation" SET "balance" = "balance" - LEAST(GREATEST("balance", 0), $2) WHERE "corporationID" = $1"#, &[&s.attacker_exec, &TOWER_COST]).await?;
    let delay = claim_delay_secs as f64;
    let claim_since = now - ((delay - delay / speed.max(1.0)) as i64) * SECOND_FT;
    let tower_type = TOWERS.iter().find(|(r, _)| *r == s.attacker_race).map_or(TOWERS[2].1, |(_, t)| *t);
    let anchored = match spot(db, s.system, rng).await? {
        Some(place) => anchor(db, s.attacker_exec, tower_type, &place, claim_since).await?,
        None => false,
    };
    if anchored {
        refuel(db).await?;
    }
    info!("bots: {} besieged {}: {gone} tower(s) of {} destroyed", n.attacker, n.system, n.defender);
    let followup = if anchored { " and anchored their own" } else { "" };
    Ok((now, format!("{} destroyed {}'s towers in {}{followup}", n.attacker, n.defender, n.system)))
}

/// Brings a system's park up to date with its towers after a siege: the
/// destroyed ones explode, the new one appears.
pub async fn show_towers(db: &Client, world: &crate::world::World, system: i32) {
    let loaded = match crate::pos::load(db, system as i64).await {
        Ok(l) => l,
        Err(e) => {
            tracing::error!("bots: reading the towers of {system}: {}", crate::logging::chain(&e));
            return;
        }
    };
    let mut w = world.lock();
    let Some(park) = w.parks.get_mut(&(system as i64)) else { return };
    park.advance(crate::destiny::tick_now());
    let tick = park.tick;
    let mut entries = Vec::new();
    let gone: Vec<i64> = park.structures.keys().copied().filter(|id| !loaded.iter().any(|s| s.id == *id)).collect();
    for id in gone {
        let Some(s) = park.structures.remove(&id) else { continue };
        entries.push(crate::svc::space::entry(tick, "TerminalExplosion", vec![eve_marshal::Value::Int(id)]));
        let mut balls = vec![eve_marshal::Value::Int(id)];
        if let Some(f) = &s.field {
            balls.push(eve_marshal::Value::Int(f.id));
        }
        entries.push(crate::svc::space::entry(tick, "RemoveBalls", vec![eve_marshal::Value::List(balls)]));
    }
    for s in loaded {
        if !park.structures.contains_key(&s.id) {
            // Its force field goes up with the next pulse (`pos::step`).
            entries.push(crate::pos::add_entry(tick, &s));
            park.structures.insert(s.id, s);
        }
    }
    crate::svc::space::broadcast(&w, system as i64, None, entries);
}

/// Destroys an alliance's towers (and what they hold) in a system.
async fn destroy_towers(db: &Client, system: i32, alliance: i32) -> Result<u64, tokio_postgres::Error> {
    let towers: Vec<i32> = db
        .query(
            r#"SELECT e."itemID" FROM entity e JOIN "posStructures" p ON p."itemID" = e."itemID"
                 JOIN "corporation" co ON co."corporationID" = e."ownerID"
                WHERE e."locationID" = $1 AND co."allianceID" = $2"#,
            &[&system, &alliance],
        )
        .await?
        .iter()
        .filter_map(|r| int(r, 0).map(|v| v as i32))
        .collect();
    if towers.is_empty() {
        return Ok(0);
    }
    db.execute(r#"DELETE FROM entity WHERE "locationID" = ANY($1)"#, &[&towers]).await?;
    db.execute(r#"DELETE FROM "posStructures" WHERE "itemID" = ANY($1) OR "controlTowerID" = ANY($1)"#, &[&towers]).await?;
    db.execute(r#"DELETE FROM entity WHERE "itemID" = ANY($1)"#, &[&towers]).await
}

/// A siege fought at once, asked for by hand (`/bots battle`): in `place`
/// (a system by ID or name), else wherever a bot alliance holds a claiming
/// tower, those at war first. The defender is that tower's alliance; the
/// attacker an alliance at war with it, else another bot alliance, which
/// declares war. Returns what to tell the one who asked.
#[allow(clippy::too_many_arguments)]
pub async fn siege_now(
    db: &Client,
    atlas: &Atlas,
    p: &mut Politics,
    sieges: &mut Vec<Siege>,
    rng: &mut Rng,
    clock: u32,
    place: Option<&str>,
    stakes: super::battle::Stakes,
    speed: f64,
) -> Result<Result<String, String>, tokio_postgres::Error> {
    let wanted: Option<i32> = match place {
        None => None,
        Some(p) => match p.trim().parse::<i32>() {
            Ok(id) => Some(id),
            Err(_) => {
                let sql = r#"SELECT "solarSystemID" FROM "mapSolarSystems" WHERE lower("solarSystemName") = lower($1)"#;
                match db.query_opt(sql, &[&p.trim()]).await? {
                    Some(r) => Some(r.get(0)),
                    None => return Ok(Err(format!("No solar system called {p}."))),
                }
            }
        },
    };
    let info: HashMap<i32, (i32, i64, u32)> = claimants(db, p).await?.into_iter().map(|(a, _, e, race, n)| (a, (e, race, n))).collect();
    let sql = r#"SELECT DISTINCT e."locationID", co."allianceID" FROM "posStructures" ps JOIN entity e ON e."itemID" = ps."itemID"
                   JOIN "corporation" co ON co."corporationID" = e."ownerID"
                  WHERE ps."claim" = 1 AND co."allianceID" IS NOT NULL"#;
    let mut candidates: Vec<(i32, i32)> = Vec::new();
    for r in db.query(sql, &[]).await? {
        let (Some(system), Some(alliance)) = (int(&r, 0), int(&r, 1)) else { continue };
        let (system, alliance) = (system as i32, alliance as i32);
        if info.contains_key(&alliance) && wanted.is_none_or(|w| w == system) && !sieges.iter().any(|s| s.system == system) {
            candidates.push((system, alliance));
        }
    }
    candidates.sort_unstable();
    candidates.dedup();
    let at_war = |a: i32| p.wars.iter().any(|w| w.a == a || w.b == a);
    // Where no bot alliance holds a tower, the battle is fought in open
    // space, round a beacon by the pilot watching: two bot alliances (at
    // war already, if any are) meet there.
    let mut field = false;
    if candidates.is_empty() {
        let Some(system) = wanted else {
            return Ok(Err("No bot alliance holds space with a claiming tower yet, or every such system is already under siege. Name a system to fight in open space there.".into()));
        };
        if sieges.iter().any(|s| s.system == system) {
            return Ok(Err("A battle is already under way there.".into()));
        }
        if !atlas.systems.contains_key(&system) {
            return Ok(Err(format!("No solar system {system}.")));
        }
        let mut alliances: Vec<i32> = info.keys().copied().collect();
        alliances.sort_unstable();
        let warring: Vec<i32> = alliances.iter().copied().filter(|a| at_war(*a)).collect();
        let pool = if warring.is_empty() { &alliances } else { &warring };
        let Some(&defender) = pool.get(rng.below(pool.len().max(1))) else {
            return Ok(Err("There are no bot alliances to fight.".into()));
        };
        candidates.push((system, defender));
        field = true;
    }
    let warring: Vec<(i32, i32)> = candidates.iter().copied().filter(|(_, d)| at_war(*d)).collect();
    let pool = if warring.is_empty() { &candidates } else { &warring };
    let (system, defender) = pool[rng.below(pool.len())];
    let enemies: Vec<(i32, i32)> = p
        .wars
        .iter()
        .filter_map(|w| if w.a == defender { Some((w.b, w.id)) } else if w.b == defender { Some((w.a, w.id)) } else { None })
        .filter(|(a, _)| info.contains_key(a))
        .collect();
    let (attacker, war) = match enemies.get(rng.below(enemies.len().max(1))) {
        Some(&x) => x,
        None => {
            let mut others: Vec<i32> = info.keys().copied().filter(|a| *a != defender).collect();
            others.sort_unstable();
            let Some(&a) = others.get(rng.below(others.len().max(1))) else {
                return Ok(Err("Only one bot alliance holds space: there is nobody to attack it.".into()));
            };
            let id = super::politics::war_between(p, db, a, defender, 7, clock, speed).await?;
            crate::svc::war::refresh(db).await;
            (a, id)
        }
    };
    let (attacker_exec, attacker_race, attackers) = info[&attacker];
    let defenders = info[&defender].2;
    let names = SiegeNames {
        attacker: alliance_name(db, attacker).await,
        defender: alliance_name(db, defender).await,
        system: system_name(db, system).await,
        region: region_name(db, atlas.systems.get(&system).map_or(0, |x| x.region)).await,
        security: atlas.systems.get(&system).map_or(0.0, |x| x.security as f64),
    };
    let said = if field {
        format!(
            "{} by request: {} and {} meet in open space in {} ({}), now. Be in space in {} to watch it: the fleets form up round a beacon about 45 km from you.",
            stakes.label(),
            names.attacker,
            names.defender,
            names.system,
            names.region,
            names.system
        )
    } else {
        format!(
            "{} by request: {} attack {} in {} ({}), now. Be in space in {} to watch it; the fleets rally round {}'s tower.",
            stakes.label(),
            names.attacker,
            names.defender,
            names.system,
            names.region,
            names.system,
            names.defender
        )
    };
    info!("bots: {said}");
    sieges.push(Siege {
        war,
        attacker,
        defender,
        attacker_exec,
        attacker_race,
        attackers,
        defenders,
        system,
        names,
        starts: clock,
        starts_at: crate::bulkdata::filetime_now(),
        fight: None,
        showcase: Some(stakes),
        field,
    });
    Ok(Ok(said))
}

pub async fn alliance_name(db: &Client, id: i32) -> String {
    let sql = r#"SELECT "allianceName" FROM alliances WHERE "allianceID" = $1"#;
    db.query_opt(sql, &[&id]).await.ok().flatten().and_then(|r| text(&r, 0)).unwrap_or_else(|| id.to_string())
}

pub async fn system_name(db: &Client, id: i32) -> String {
    let sql = r#"SELECT "solarSystemName" FROM "mapSolarSystems" WHERE "solarSystemID" = $1"#;
    db.query_opt(sql, &[&id]).await.ok().flatten().and_then(|r| text(&r, 0)).unwrap_or_else(|| id.to_string())
}

async fn region_name(db: &Client, id: i32) -> String {
    let sql = r#"SELECT "regionName" FROM "mapRegions" WHERE "regionID" = $1"#;
    db.query_opt(sql, &[&id]).await.ok().flatten().and_then(|r| text(&r, 0)).unwrap_or_else(|| id.to_string())
}
