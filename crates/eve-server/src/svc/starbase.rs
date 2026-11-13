//! Starbases: launching structures from a cargo hold, anchoring them, putting
//! them online and offline, unanchoring and scooping them; the control
//! tower's fuel; the posMgr calls of the tower window (moonmining.py); and
//! sovereignty, which claiming towers win for their alliance. The park side
//! of a structure is pos.rs.
//!
//! A control tower anchors near a moon, one to a moon. Mobile warp disruptors
//! anchor independently. Other structures anchor within reach of an online
//! tower of the same owner and draw on its CPU and power grid. A tower burns fuel every
//! posControlTowerPeriod: the online resources in full, power and CPU fuel
//! in proportion to what its structures draw, a quarter less where its
//! alliance holds sovereignty (moonmining.ShowFuel). Short of online fuel it
//! goes offline, and its structures with it.
//!
//! In a system no NPC faction holds, the alliance with the most claiming,
//! online towers holds sovereignty, once their claims have stood for
//! `RMR_SOV_CLAIM_DELAY` (the claim tab says up to six days); a tie leaves it
//! where it was.

use std::collections::HashMap;

use eve_marshal::Value;
use tokio_postgres::{Client, GenericClient};
use tracing::{debug, error, info};

use crate::ballpark::Vec3;
use crate::items::{self, ItemTxn, NewItem};
use crate::pos::{self, state, Structure};
use crate::services::{Call, Ctx, Outcome};
use crate::session::CORP_ROLE_DIRECTOR;
use crate::svc::membership::{access_denied, db_failed, message, Refused};
use crate::svc::moonmining;
use crate::svc::rows::{float, int, line, rowset};
use crate::svc::space::{self, entry, GROUP_MOON, MOON_WARP_RANGE};
use crate::world::World;

/// dgmEffects: onlineForStructures, anchorLiftForStructures.
const EFFECT_ONLINE: i64 = 901;
const EFFECT_ANCHOR: i64 = 1022;
const EFFECT_UNANCHOR: i64 = 1023;
const EFFECT_ANCHOR_DEPLOYABLE: i64 = 649;
const EFFECT_UNANCHOR_DEPLOYABLE: i64 = 650;
const ATTR_POWER: i32 = 30;
const ATTR_CPU: i32 = 50;
const ATTR_POWER_OUTPUT: i32 = 11;
const ATTR_CPU_OUTPUT: i32 = 48;
const ATTR_HARVESTER_CPU_BONUS: i32 = 756;
const ATTR_MAX_STRUCTURE_DISTANCE: i32 = 650;
const ATTR_MOON_ANCHOR_DISTANCE: i32 = 711;
const ATTR_TOWER_PERIOD: i32 = 722;
const ATTR_ANCHORING_SECURITY_MAX: i32 = 1032;
const ATTR_ANCHORING_SOVEREIGNTY: i32 = 1033;
/// invControlTowerResourcePurposes.
const PURPOSE_ONLINE: i64 = 1;
const PURPOSE_POWER: i64 = 2;
const PURPOSE_CPU: i64 = 3;
const PURPOSE_REINFORCE: i64 = 4;
pub const STRONTIUM: i64 = 16275;
/// The Blood dump's 12,500/25,000/50,000 m3 tower capacities are kept for
/// strontium. The expanded fuel allocation is 35,000/70,000/140,000 m3.
pub const TOWER_FUEL_CAPACITY_RATIO: f64 = 2.8;
/// eveConstants corpRoleStarbaseConfig.
const CORP_ROLE_STARBASE_CONFIG: i64 = 1 << 53;
const FLAG_CARGO: i64 = 5;
/// eveConstants maxConfigureDistance, with slack for the two simulations.
const CONFIGURE_RANGE: f64 = 5_000.0 * 1.25;
/// How far from the ship a structure may be anchored (pwn.CheckAnchoringPosition).
const ANCHOR_REACH: f64 = 30_000.0;
/// How far from a moon's normal warp-in point a control tower may anchor.
const MOON_RANGE: f64 = 100_000.0;
/// No anchoring at or above this security ('clean spacelanes').
const CONCORD_ANCHOR_LIMIT: f64 = 0.8;
/// Room left around a launched structure.
const LAUNCH_CLEARANCE: f64 = 500.0;
/// Fuel burnt where the owner's alliance holds sovereignty.
const SOVEREIGN_FUEL: f64 = 0.75;
const TICKS_PER_MS: i64 = 10_000;
/// Staff who may anchor an outpost platform without sovereignty: CCP, GM and
/// BH carry role 8, ISD BH 2048 (config::staff_role).
const OUTPOST_STAFF_ROLES: i64 = 8 | 2_048;
const TICKS_PER_SECOND: i64 = 10_000_000;
const DEFAULT_PERIOD_MS: i64 = 3_600_000;

// ---- launching -------------------------------------------------------------------------

/// ship.Drop(itemIDs, ownerID): structures in the hold go into space, owned
/// by the pilot or their corporation (menusvc LaunchForSelf/LaunchForCorp).
/// None if none of the items is a structure (drones are launched elsewhere).
pub async fn launch(ctx: &Ctx<'_>, ids: &[i64], owner: Option<i64>) -> Option<Outcome> {
    let structures: Vec<i64> = {
        let sql = r#"SELECT e."itemID" FROM entity e
                       JOIN "invTypes" t ON t."typeID" = e."typeID"
                       JOIN "invGroups" g ON g."groupID" = t."groupID"
                      WHERE e."itemID" = ANY($1) AND (g."categoryID" = $2 OR g."groupID" = ANY($3))"#;
        let wanted: Vec<i32> = ids.iter().map(|i| *i as i32).collect();
        match ctx.db.query(sql, &[&wanted, &pos::CATEGORY_STRUCTURE, &&pos::LOOSE_STRUCTURE_GROUPS[..]]).await {
            Ok(rows) => rows.iter().filter_map(|r| int(r, 0)).collect(),
            Err(e) => {
                error!("checking items to launch: {}", crate::logging::chain(&e));
                return Some(Outcome::custom("The items could not be launched."));
            }
        }
    };
    if structures.is_empty() {
        return None;
    }
    Some(launch_structures(ctx, ids.iter().copied().filter(|i| structures.contains(i)).collect(), owner).await)
}

async fn launch_structures(ctx: &Ctx<'_>, ids: Vec<i64>, owner: Option<i64>) -> Outcome {
    let (Some(char_id), Some(ship_id), Some(system)) = (ctx.session.char_id, ctx.session.ship_id, ctx.session.solar_system_id) else {
        return Outcome::custom("Structures can only be launched in space.");
    };
    let owner = owner.unwrap_or(char_id);
    if owner != char_id && Some(owner) != ctx.session.corp_id {
        return Outcome::custom("You can launch structures for yourself or your corporation.");
    }
    let Some((ship_pos, ship_radius)) = ctx.world.lock().parks.get(&system).and_then(|p| p.ball(ship_id)) else {
        return Outcome::custom("You are not in space.");
    };
    let mut rng = crate::combat::Rng::seeded(crate::combat::now_ms() as u64 ^ ship_id as u64);
    let result: Result<(Vec<(i64, i64, Vec3)>, Vec<Value>), tokio_postgres::Error> = async {
        let mut client = crate::db::connect(ctx.db_url).await?;
        let mut txn = ItemTxn::begin(&mut client).await?;
        let mut launched = Vec::new();
        for id in ids {
            let Some(item) = txn.item(id).await? else { continue };
            if item.owner_id != char_id || item.location_id != ship_id || item.flag != FLAG_CARGO {
                continue;
            }
            let structure = if item.quantity > 1 {
                txn.set_quantity(id, item.quantity - 1).await?;
                txn.create(NewItem { type_id: item.type_id, owner_id: owner, location_id: system, flag: 0, quantity: 1, singleton: true })
                    .await?
            } else {
                txn.move_to(id, system, 0).await?;
                txn.set_singleton(id, true).await?;
                if owner != char_id {
                    txn.set_owner(id, owner).await?;
                }
                id
            };
            let radius: f64 = txn
                .tx()
                .query_one(r#"SELECT COALESCE("radius", 100) FROM "invTypes" WHERE "typeID" = $1"#, &[&(item.type_id as i32)])
                .await?
                .get(0);
            let pos = ship_pos + rng.in_sphere(1.0).unit() * (ship_radius + radius + LAUNCH_CLEARANCE);
            txn.set_position(structure, pos.tuple()).await?;
            txn.tx()
                .execute(r#"INSERT INTO "posStructures" ("itemID") VALUES ($1) ON CONFLICT ("itemID") DO NOTHING"#, &[&(structure as i32)])
                .await?;
            launched.push((structure, item.type_id, pos));
        }
        Ok((launched, txn.commit().await?))
    }
    .await;
    let (launched, events) = match result {
        Ok(v) => v,
        Err(e) => {
            error!("launching structures: {}", crate::logging::chain(&e));
            return Outcome::custom("The structure could not be launched.");
        }
    };
    if launched.is_empty() {
        return Outcome::custom("Those structures are not in your cargo hold.");
    }
    items::notify(ctx.world, ctx.client_id, events);
    // Read back through the loader, for the names, owners and delays.
    let fresh = match pos::load(ctx.db, system).await {
        Ok(all) => all,
        Err(e) => {
            error!("reading launched structures: {}", crate::logging::chain(&e));
            Vec::new()
        }
    };
    let mut world = ctx.world.lock();
    let mut new_ids = Vec::new();
    if let Some(park) = world.parks.get_mut(&system) {
        let mut adds = Vec::new();
        for s in fresh.into_iter().filter(|s| launched.iter().any(|(id, _, _)| *id == s.id)) {
            new_ids.push(Value::Int(s.id));
            info!("launched {} ({}) for {owner} in {system}", s.name, s.id);
            let id = s.id;
            park.structures.insert(s.id, s);
            adds.extend(space::add_object_entries_for_nearby(park, id));
        }
        space::notify_targeted_updates(&world, adds);
    }
    Outcome::Value(Value::List(new_ids))
}

// ---- anchoring, onlining, unanchoring --------------------------------------------------

/// The caller's view of one structure: it, and how far the caller's ship is
/// from it and from a point.
struct Seen {
    s: Structure,
    system: i64,
    ship_pos: Vec3,
    distance: f64,
}

fn seen(ctx: &Ctx<'_>, id: i64) -> Option<Seen> {
    let (system, ship_id) = (ctx.session.solar_system_id?, ctx.session.ship_id?);
    let world = ctx.world.lock();
    let park = world.parks.get(&system)?;
    let s = park.structures.get(&id)?.clone();
    let ship_pos = park.ships.get(&ship_id)?.pos;
    let distance = park.surface_distance(ship_id, id)?;
    Some(Seen { s, system, ship_pos, distance })
}

/// posMgr.AnchorStructure(itemID, (x, y, z)).
async fn anchor(ctx: &Ctx<'_>, id: i64, mut at: Vec3) -> Refused<Value> {
    let Some(Seen { s, system, ship_pos, distance }) = seen(ctx, id) else {
        return Err(Outcome::custom("That structure is not here."));
    };
    let now = crate::bulkdata::filetime_now();
    if !s.is_free(now) {
        return Err(message_with("AlreadyUnanchoring", &[("group", Value::text(&s.name))]));
    }
    if !allowed(ctx, &s, deploy_flag(ctx.db, s.tower.unwrap_or(s.id), "deployAnchor").await) || distance > ANCHOR_REACH {
        return Err(access_denied("you may not anchor that structure from here"));
    }
    let security = (system_security(ctx.db, system).await * 10.0).round() / 10.0;
    if s.is_warp_disruptor() && security > 0.0 {
        return Err(message_with("CantAnchorConcordRestriction01", &[("security", Value::Double(security))]));
    }
    if security >= CONCORD_ANCHOR_LIMIT {
        return Err(message("CantAnchorConcordRestriction"));
    }
    if s.is_platform() {
        check_outpost_site(ctx, &s, system, security).await?;
    }
    let attrs = type_attributes(ctx.db, &[s.type_id], &[ATTR_ANCHORING_SECURITY_MAX, ATTR_ANCHORING_SOVEREIGNTY, ATTR_MOON_ANCHOR_DISTANCE]).await;
    if let Some(max) = attrs.get(&(s.type_id, ATTR_ANCHORING_SECURITY_MAX)) {
        if security > *max {
            return Err(Outcome::custom(format!(
                "CONCORD forbids anchoring the {} in systems above {max:.1} security.",
                s.name
            )));
        }
    }
    if attrs.get(&(s.type_id, ATTR_ANCHORING_SOVEREIGNTY)).is_some_and(|v| *v > 0.0) {
        let sovereign = ctx.query_one(
            r#"SELECT 1 FROM "mapSolarSystems" WHERE "solarSystemID" = $1 AND "allianceID" = $2"#,
            &[&(system as i32), &s.alliance_id.map(|id| id as i32)],
        ).await.is_some();
        if !sovereign {
            return Err(Outcome::custom("Your alliance must hold sovereignty in this system to anchor that structure."));
        }
    }

    let moon_range = attrs.get(&(s.type_id, ATTR_MOON_ANCHOR_DISTANCE)).copied().unwrap_or(MOON_RANGE);
    // The 30 km interaction limit is measured to the launched tower. The
    // placement cursor can choose a point farther away on the same moon grid.
    let placement_limit = if s.is_tower() { moon_range.max(ANCHOR_REACH) } else { ANCHOR_REACH };
    let reach = (at - ship_pos).len();
    if !reach.is_finite() || reach > placement_limit {
        return Err(Outcome::custom(format!(
            "The selected anchoring position is {reach:.0} m away. Choose a point within {placement_limit:.0} m."
        )));
    }

    let (tower, moon) = if s.is_tower() {
        if (0.4..0.8).contains(&security) {
            check_empire_anchor(ctx, &s, system, security).await?;
        }
        let moon = moon_for(ctx, system, at, ship_pos, moon_range)?;
        at = tower_anchor_point(&moon, at, ship_pos, s.radius, moon_range);
        (None, Some(moon))
    } else if s.is_deployable() {
        (None, None)
    } else {
        (Some(tower_for(ctx, system, &s, at, now).await?), None)
    };
    let stamp = pos::timer_start(now, s.delays[0]);
    let moon_id = moon.as_ref().map(|m| m.id as i32);
    let saved = async {
        let mut db = crate::db::connect(ctx.db_url).await?;
        let tx = db.transaction().await?;
        if let Some(moon) = moon_id {
            if !reserve_moon(&tx, moon).await? {
                return Ok(false);
            }
        }
        let changed = tx.execute(
            r#"UPDATE "posStructures" SET state = $2, "stateTimestamp" = $3, "controlTowerID" = $4, "moonID" = $5
                WHERE "itemID" = $1 AND state = 0 AND "stateTimestamp" IS NOT DISTINCT FROM $6"#,
            &[&(id as i32), &state::ANCHORED, &stamp, &tower.map(|t| t as i32), &moon_id, &s.stamp],
        ).await?;
        if changed == 0 {
            return Ok(false);
        }
        tx.execute(r#"UPDATE entity SET "x" = $2, "y" = $3, "z" = $4 WHERE "itemID" = $1"#, &[&(id as i32), &at.x, &at.y, &at.z]).await?;
        tx.commit().await?;
        Ok::<_, tokio_postgres::Error>(true)
    }.await;
    match saved {
        Ok(true) => {}
        Ok(false) => return Err(match moon {
            Some(m) => message_with("CantAnchorMoonAlreadyHasControlTower", &[("tower", Value::text(&s.name)), ("moon", Value::text(m.name))]),
            None => Outcome::custom("That structure is already anchoring."),
        }),
        Err(e) => {
            error!("anchoring {id}: {}", crate::logging::chain(&e));
            return Err(Outcome::custom("The structure could not be anchored."));
        }
    }
    update(ctx.world, system, id, |s, tick| {
        s.state = state::ANCHORED;
        s.stamp = Some(stamp);
        s.tower = tower;
        s.pos = at;
        let mut out = vec![entry(tick, "SetBallPosition", vec![Value::Int(id), Value::Double(at.x), Value::Double(at.y), Value::Double(at.z)])];
        if !s.is_warp_disruptor() {
            out.extend(pos::free_entries(tick, id, false));
        }
        out
    });
    info!("anchoring {} ({id}) in {system}{}", s.name, tower.map(|t| format!(" at tower {t}")).unwrap_or_default());
    // Anchoring ends at stamp + anchoringDelay (timer_start scales the stamp).
    let ends = stamp + s.delays[0];
    let text = format!(
        "Anchoring {}: {} to go, finishing at {} EVE.",
        s.name, pos::duration_text((ends - now) / 10_000_000), pos::eve_clock(ends)
    );
    ctx.world.lock().notify(&[ctx.client_id], "OnRemoteMessage", pos::custom_notify(&text));
    Ok(Value::None)
}

/// Empire space checks corporate faction standing and the charter carried
/// by the ship. The matching charter is then burned with each fuel cycle.
async fn check_empire_anchor(ctx: &Ctx<'_>, tower: &Structure, system: i64, security: f64) -> Refused<()> {
    let row = ctx.query_one(
        r#"SELECT COALESCE(s."factionID", c."factionID", g."factionID") AS faction,
                  r."resourceTypeID", it."typeName"
             FROM "mapSolarSystems" s
             JOIN "mapConstellations" c ON c."constellationID" = s."constellationID"
             JOIN "mapRegions" g ON g."regionID" = s."regionID"
             JOIN "invControlTowerResources" r ON r."factionID" = COALESCE(s."factionID", c."factionID", g."factionID")
             JOIN "invTypes" it ON it."typeID" = r."resourceTypeID"
            WHERE s."solarSystemID" = $1 AND r."controlTowerTypeID" = $2 AND r."minSecurityLevel" IS NOT NULL"#,
        &[&(system as i32), &(tower.type_id as i32)],
    ).await.ok_or_else(|| Outcome::custom("There is no matching starbase charter for this tower in this system."))?;
    let faction: i32 = row.get(0);
    let charter: i32 = row.get(1);
    let name: String = row.get(2);
    let status = ctx.query_one(
        r#"SELECT COALESCE(
                      (SELECT "standing" FROM "ownerStandings" WHERE "fromID" = $1 AND "toID" = $2),
                      (SELECT avg(n."standing") FROM "chrNPCStandings" n
                         JOIN "character_" ch ON ch."characterID" = n."characterID"
                        WHERE ch."corporationID" = $1 AND n."fromID" = $2 AND n."standing" <> 0),
                      0)::float8,
                  EXISTS (SELECT 1 FROM entity WHERE "locationID" = $3 AND flag = 5 AND "typeID" = $4 AND quantity > 0)"#,
        &[&(tower.corp_id as i32), &faction, &ctx.session.ship_id.map(|id| id as i32), &charter],
    ).await.ok_or_else(|| Outcome::custom("The starbase anchoring requirements could not be checked."))?;
    let required = security * 10.0;
    let standing: f64 = status.get(0);
    if standing < required {
        return Err(Outcome::custom(format!("Your corporation needs {required:.1} standing with the faction that holds this system.")));
    }
    if !status.get::<_, bool>(1) {
        return Err(Outcome::custom(format!("Carry a {name} in your cargo hold to anchor here.")));
    }
    Ok(())
}

/// /rushanchor [structureID]: the structure's current timed step (anchoring,
/// onlining, unanchoring) finishes now; the next heartbeat completes it. With
/// no ID, the nearest structure with a timer running.
pub async fn rush(ctx: &Ctx<'_>, wanted: Option<i64>) -> Result<String, String> {
    let (Some(ship), Some(system)) = (ctx.session.ship_id, ctx.session.solar_system_id) else {
        return Err("You must be in space beside the structure.".into());
    };
    let now = crate::bulkdata::filetime_now();
    let (id, name, step, old, new) = {
        let world = ctx.world.lock();
        let park = world.parks.get(&system).ok_or("You are not in space.")?;
        let found = match wanted {
            Some(id) => park.structures.get(&id),
            None => park.structures.values().filter(|s| s.busy(now)).min_by(|a, b| {
                let d = |s: &Structure| park.surface_distance(ship, s.id).unwrap_or(f64::MAX);
                d(a).total_cmp(&d(b))
            }),
        };
        let s = found.ok_or("No structure here has a timer running.")?;
        let old = s.stamp.filter(|_| s.busy(now)).ok_or(format!("{} has no timer running.", s.name))?;
        (s.id, s.name.clone(), s.step_name(), old, now - s.step_delay())
    };
    let changed = ctx.db.execute(
        r#"UPDATE "posStructures" SET "stateTimestamp" = $2 WHERE "itemID" = $1 AND "stateTimestamp" = $3"#,
        &[&(id as i32), &new, &old],
    ).await.map_err(|e| e.to_string())?;
    if changed == 0 {
        return Err(format!("{name}'s timer changed meanwhile; try again."));
    }
    if let Some(s) = ctx.world.lock().parks.get_mut(&system).and_then(|p| p.structures.get_mut(&id)) {
        if s.stamp == Some(old) {
            s.stamp = Some(new);
        }
    }
    Ok(format!("{step} of {name} ({id}) rushed; it completes within a second."))
}

/// An outpost platform anchors in null-sec, in a system with no outpost yet
/// (NPC stations do not count) and no other platform anchored or anchoring,
/// where its owner's alliance holds sovereignty.
/// Staff (GM, BH, ISD BH, CCP) may anchor one in unclaimed null-sec too.
async fn check_outpost_site(ctx: &Ctx<'_>, s: &Structure, system: i64, security: f64) -> Refused<()> {
    if security > 0.0 {
        return Err(Outcome::custom("Outposts can only be built in null-security space."));
    }
    let outpost = ctx.query_one(
        r#"SELECT COALESCE("stationName", '') FROM "staStations"
            WHERE "solarSystemID" = $1 AND "stationID" BETWEEN $2 AND $3 LIMIT 1"#,
        &[&(system as i32), &crate::svc::outpost::OUTPOST_ID_MIN, &crate::svc::outpost::OUTPOST_ID_MAX],
    ).await;
    if let Some(row) = outpost {
        return Err(Outcome::custom(format!(
            "This system already has an outpost ({}). Only one outpost can be built in a system.",
            row.get::<_, String>(0)
        )));
    }
    let other = ctx.world.lock().parks.get(&system).is_some_and(|park| {
        park.structures.values().any(|o| o.id != s.id && o.is_platform() && o.state == state::ANCHORED)
    });
    if other {
        return Err(Outcome::custom("Another outpost platform is already anchored in this system."));
    }
    let sovereign = ctx.query_one(
        r#"SELECT 1 FROM "mapSolarSystems" WHERE "solarSystemID" = $1 AND "allianceID" = $2"#,
        &[&(system as i32), &s.alliance_id.map(|id| id as i32)],
    ).await.is_some();
    if !sovereign {
        if ctx.session.role & OUTPOST_STAFF_ROLES == 0 {
            return Err(Outcome::custom("Your alliance must hold sovereignty in this system to build an outpost here."));
        }
        info!("staff anchoring outpost platform {} in {system} without sovereignty", s.id);
    }
    Ok(())
}

fn moon_for(ctx: &Ctx<'_>, system: i64, at: Vec3, ship_pos: Vec3, range: f64) -> Refused<crate::ballpark::Celestial> {
    let world = ctx.world.lock();
    let park = world.parks.get(&system).ok_or_else(|| Outcome::custom("You are not in space."))?;
    anchor_moon(&park.celestials, at, ship_pos, range)
        .cloned()
        .ok_or_else(|| message("CantAnchorNoMoonNearby"))
}

fn anchor_moon(
    celestials: &[crate::ballpark::Celestial], at: Vec3, ship_pos: Vec3, range: f64,
) -> Option<&crate::ballpark::Celestial> {
    celestials.iter().filter(|c| c.group_id == GROUP_MOON)
        .filter_map(|moon| {
            let shell = (moon.radius - range)..=(moon.radius + MOON_WARP_RANGE + range);
            let placement = (at - moon.pos).len();
            let ship = (ship_pos - moon.pos).len();
            // Keep existing near-surface sites valid as well as the new warp-in.
            (shell.contains(&placement) || shell.contains(&ship))
                .then_some((moon, (placement - moon.radius).abs()))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(moon, _)| moon)
}

fn tower_anchor_point(moon: &crate::ballpark::Celestial, at: Vec3, ship_pos: Vec3, tower_radius: f64, range: f64) -> Vec3 {
    let offset = at - moon.pos;
    let distance = offset.len();
    let inner = moon.radius + tower_radius + LAUNCH_CLEARANCE;
    let outer = (moon.radius + MOON_WARP_RANGE + range).max(inner);
    let clamped = distance.clamp(inner, outer);
    if distance == clamped {
        return at;
    }
    let direction = if distance > 0.0 { offset } else { ship_pos - moon.pos };
    let outward = if direction.len_sq() > 0.0 { direction.unit() } else { Vec3::new(1.0, 0.0, 0.0) };
    moon.pos + outward * clamped
}

/// Hold the moon until the caller commits its tower placement.
pub async fn reserve_moon(tx: &tokio_postgres::Transaction<'_>, moon: i32) -> Result<bool, tokio_postgres::Error> {
    tx.query_one("SELECT pg_advisory_xact_lock($1, $2)", &[&365i32, &moon]).await?;
    let tower = tx.query_opt(
        r#"SELECT 1 FROM "posStructures" WHERE "moonID" = $1
             AND (state <> 0 OR "stateTimestamp" IS NOT NULL) LIMIT 1"#,
        &[&moon],
    ).await?;
    Ok(tower.is_none())
}

/// The online tower of the same owner whose reach covers `at`.
async fn tower_for(ctx: &Ctx<'_>, system: i64, s: &Structure, at: Vec3, now: i64) -> Refused<i64> {
    let towers: Vec<Structure> = {
        let world = ctx.world.lock();
        let park = world.parks.get(&system).ok_or_else(|| Outcome::custom("You are not in space."))?;
        park.structures.values().filter(|t| t.is_tower() && t.fixed()).cloned().collect()
    };
    let types: Vec<i64> = towers.iter().map(|t| t.type_id).collect();
    let reach = type_attributes(ctx.db, &types, &[ATTR_MAX_STRUCTURE_DISTANCE]).await;
    let near = towers.iter()
        .filter(|t| (t.pos - at).len() <= reach.get(&(t.type_id, ATTR_MAX_STRUCTURE_DISTANCE)).copied().unwrap_or(0.0))
        .min_by(|a, b| (a.owner_id != s.owner_id).cmp(&(b.owner_id != s.owner_id))
            .then_with(|| (a.pos - at).len_sq().total_cmp(&(b.pos - at).len_sq())));
    let Some(tower) = near else { return Err(message_with("CantAnchorRequireTower", &[("item", Value::text(&s.name))])) };
    if tower.owner_id != s.owner_id {
        return Err(message_with("CantAnchorStructureOwnerBad", &[("what", Value::text(&s.name)), ("whom", Value::text(owner_name(ctx, tower.owner_id).await))]));
    }
    if !tower.fully_online(now) {
        return Err(message("CantTowerNotOnline"));
    }
    if (tower.pos - at).len() < tower.radius + s.radius + LAUNCH_CLEARANCE {
        return Err(Outcome::custom("Choose an anchoring position clear of the control tower."));
    }
    Ok(tower.id)
}

/// dogmaIM Activate/Deactivate of starbase and deployable anchoring effects.
pub async fn dogma_im(call: &Call, ctx: &Ctx<'_>) -> Option<Outcome> {
    let id = call.arg_id(0);
    let effect = call.arg_i64(1)?;
    if !matches!(call.method.as_str(), "Activate" | "Deactivate")
        || !matches!(effect, EFFECT_ONLINE | EFFECT_ANCHOR | EFFECT_UNANCHOR | EFFECT_ANCHOR_DEPLOYABLE | EFFECT_UNANCHOR_DEPLOYABLE) {
        return None;
    }
    let s = seen(ctx, id)?.s;
    let result = match (call.method.as_str(), effect, s.is_deployable()) {
        ("Activate", EFFECT_ANCHOR_DEPLOYABLE, true) | ("Activate", EFFECT_ANCHOR, false) => anchor(ctx, id, s.pos).await,
        ("Activate", EFFECT_UNANCHOR_DEPLOYABLE, true) | ("Activate", EFFECT_UNANCHOR, false) => unanchor(ctx, id).await,
        ("Activate", EFFECT_ONLINE, false) => online(ctx, id).await,
        ("Deactivate", EFFECT_ONLINE, false) => offline(ctx, id).await,
        _ => Ok(Value::None),
    };
    Some(result.map_or_else(|refusal| refusal, Outcome::Value))
}

async fn online(ctx: &Ctx<'_>, id: i64) -> Refused<Value> {
    let Seen { s, system, distance, .. } = seen(ctx, id).ok_or_else(|| Outcome::custom("That structure is not here."))?;
    not_reinforced(ctx, &s)?;
    let now = crate::bulkdata::filetime_now();
    if !s.fully_anchored(now) {
        return Err(message_with("CantDoWhileNotAnchored", &[("item", Value::text(&s.name))]));
    }
    if !allowed(ctx, &s, deploy_flag(ctx.db, s.tower.unwrap_or(s.id), "deployOnline").await) || distance > CONFIGURE_RANGE {
        return Err(access_denied("you may not put that structure online from here"));
    }
    let stamp = pos::timer_start(now, s.delays[1]);
    if s.is_tower() {
        let mut db = crate::db::connect(ctx.db_url).await.map_err(|e| db_failed("connecting to fuel the tower", e))?;
        let tx = db.transaction().await.map_err(|e| db_failed("onlining the tower", e))?;
        let anchored = tx.query_opt(
            r#"SELECT 1 FROM entity e JOIN "posStructures" p USING ("itemID")
                WHERE e."itemID" = $1 AND p.state = 1 FOR UPDATE OF e, p"#, &[&(id as i32)],
        ).await.map_err(|e| db_failed("locking the tower", e))?;
        if anchored.is_none() {
            return Err(Outcome::custom("The tower must be anchored before you can put it online."));
        }
        let fuel = Fuel::of(&tx, &s, system).await;
        let burn = fuel.burn(0.0, 0.0);
        if burn.is_empty() {
            return Err(Outcome::custom("The tower's fuel requirements could not be found."));
        }
        if let Some((type_id, need)) = burn.iter().find(|(t, need)| fuel.held.get(t).copied().unwrap_or(0) < *need) {
            return Err(message_with(
                "CantOnlineTowerLacksResources",
                &[("tower", Value::text(&s.name)), ("resource", Value::text(format!("{need} x {}", type_name(ctx, *type_id).await)))],
            ));
        }
        // The first period's fuel goes as it comes online.
        if !consume(&tx, id, &burn, now).await.map_err(|e| db_failed("fuelling the tower", e))? {
            return Err(Outcome::custom("The tower ran out of fuel while going online."));
        }
        tx.execute(
            r#"UPDATE "posStructures" SET state = $2, "stateTimestamp" = $3, "claimSince" = NULL WHERE "itemID" = $1"#,
            &[&(id as i32), &state::ONLINING, &stamp],
        ).await.map_err(|e| db_failed("saving the tower's onlining timer", e))?;
        tx.commit().await.map_err(|e| db_failed("onlining the tower", e))?;
        update(ctx.world, system, id, |s, _| {
            s.state = state::ONLINING;
            s.stamp = Some(stamp);
            Vec::new()
        });
    } else {
        draw_from_tower(ctx, system, &s, now).await?;
        set_state(ctx, system, id, state::ONLINING, Some(stamp)).await?;
    }
    info!("onlining {} ({id}) in {system}", s.name);
    Ok(Value::None)
}

/// A structure coming online needs its tower online, with the CPU and power
/// to spare and fuel for them.
async fn draw_from_tower(ctx: &Ctx<'_>, system: i64, s: &Structure, now: i64) -> Refused<()> {
    let (tower, drawing) = {
        let world = ctx.world.lock();
        let park = world.parks.get(&system).ok_or_else(|| Outcome::custom("You are not in space."))?;
        let tower = s.tower.and_then(|t| park.structures.get(&t)).cloned();
        let drawing: Vec<i64> = park
            .structures
            .values()
            .filter(|o| o.tower == s.tower && o.id != s.id && o.drawing())
            .map(|o| o.type_id)
            .collect();
        (tower, drawing)
    };
    let Some(tower) = tower else { return Err(message_with("CantOnlineRequireTower", &[("item", Value::text(&s.name))])) };
    if !tower.fully_online(now) {
        return Err(message("CantTowerNotOnline"));
    }
    let mut types = drawing.clone();
    types.extend([s.type_id, tower.type_id]);
    let a = type_attributes(ctx.db, &types, &[ATTR_CPU, ATTR_POWER, ATTR_CPU_OUTPUT, ATTR_POWER_OUTPUT]).await;
    let cpu = cpu_factors(ctx.db, tower.type_id, &types).await;
    let get = |t: i64, attr: i32| a.get(&(t, attr)).copied().unwrap_or(0.0) * if attr == ATTR_CPU { cpu(t) } else { 1.0 };
    let used = |attr: i32| drawing.iter().map(|t| get(*t, attr)).sum::<f64>();
    let fuel = Fuel::of(ctx.db, &tower, system).await;
    for (attr, output, purpose, lacks, what) in [
        (ATTR_CPU, ATTR_CPU_OUTPUT, PURPOSE_CPU, "CantOnlineTowerLacksCpuResources", "CPU"),
        (ATTR_POWER, ATTR_POWER_OUTPUT, PURPOSE_POWER, "CantOnlineTowerLacksPowerResources", "power grid"),
    ] {
        let need = get(s.type_id, attr);
        if need <= 0.0 {
            continue;
        }
        if used(attr) + need > get(tower.type_id, output) {
            return Err(Outcome::custom(format!("The {} does not have the {what} to spare for the {}.", tower.name, s.name)));
        }
        if !fuel.has_any(purpose) {
            return Err(message_with(lacks, &[("what", Value::text(&s.name)), ("tower", Value::text(&tower.name))]));
        }
    }
    Ok(())
}

/// Offline; a tower going offline takes its structures with it.
async fn offline(ctx: &Ctx<'_>, id: i64) -> Refused<Value> {
    let Seen { s, system, distance, .. } = seen(ctx, id).ok_or_else(|| Outcome::custom("That structure is not here."))?;
    not_reinforced(ctx, &s)?;
    if !s.drawing() {
        return Ok(Value::None);
    }
    if !allowed(ctx, &s, deploy_flag(ctx.db, s.tower.unwrap_or(s.id), "deployOffline").await) || distance > CONFIGURE_RANGE {
        return Err(access_denied("you may not take that structure offline from here"));
    }
    take_offline(ctx.db, ctx.world, system, id).await;
    info!("took {} ({id}) offline in {system}", s.name);
    Ok(Value::None)
}

async fn unanchor(ctx: &Ctx<'_>, id: i64) -> Refused<Value> {
    let Seen { s, system, distance, .. } = seen(ctx, id).ok_or_else(|| Outcome::custom("That structure is not here."))?;
    not_reinforced(ctx, &s)?;
    let now = crate::bulkdata::filetime_now();
    if !s.fully_anchored(now) {
        return Err(message_with("CantDoWhileNotAnchored", &[("item", Value::text(&s.name))]));
    }
    if !allowed(ctx, &s, deploy_flag(ctx.db, s.tower.unwrap_or(s.id), "deployUnanchor").await) || distance > CONFIGURE_RANGE {
        return Err(access_denied("you may not unanchor that structure from here"));
    }
    if s.is_tower() {
        let anchored_here = ctx.world.lock().parks.get(&system).is_some_and(|p| p.structures.values().any(|o| o.tower == Some(id) && o.fixed()));
        if anchored_here {
            return Err(message_with("CantUnanchorTowerStructuresPresent", &[("tower", Value::text(&s.name))]));
        }
    }
    let holding = ctx.query_one(r#"SELECT 1 FROM entity WHERE "locationID" = $1 LIMIT 1"#, &[&(id as i32)]).await.is_some();
    if holding {
        return Err(message_with("CantUnanchorContainerNotEmpty", &[("structure", Value::text(&s.name))]));
    }
    let stamp = pos::timer_start(now, s.delays[2]);
    set_state(ctx, system, id, state::UNANCHORED, Some(stamp)).await?;
    info!("unanchoring {} ({id}) in {system}", s.name);
    Ok(Value::None)
}

/// ship.Scoop(itemID) of a structure: into the cargo hold, once it floats
/// free. None if the item is not a structure.
pub async fn scoop(ctx: &Ctx<'_>, id: i64) -> Option<Outcome> {
    let Seen { s, system, distance, .. } = seen(ctx, id)?;
    Some(scoop_structure(ctx, s, system, distance).await.map_or_else(|refusal| refusal, Outcome::Value))
}

async fn scoop_structure(ctx: &Ctx<'_>, s: Structure, system: i64, distance: f64) -> Refused<Value> {
    let (char_id, ship_id) = (ctx.session.char_id.unwrap_or(0), ctx.session.ship_id.unwrap_or(0));
    let now = crate::bulkdata::filetime_now();
    if s.state != state::UNANCHORED {
        return Err(message_with("CanOnlyScoopIfUnanchored", &[("group", Value::text(&s.name))]));
    }
    if !s.is_free(now) {
        return Err(message_with("CantScoopAnchoring", &[("group", Value::text(&s.name))]));
    }
    if s.owner_id != char_id && Some(s.owner_id) != ctx.session.corp_id {
        return Err(message_with("CantScoopNotInCorp", &[("corp", Value::text(owner_name(ctx, s.owner_id).await))]));
    }
    if distance > crate::loot::TRANSFER_RANGE {
        return Err(Outcome::custom(format!("The {} is {distance:.0} m away; get within {:.0} m to scoop it.", s.name, crate::loot::TRANSFER_RANGE)));
    }
    let room = ctx
        .query_one(
            r#"SELECT (SELECT COALESCE(sum(t."volume" * e."quantity"), 0) FROM entity e JOIN "invTypes" t ON t."typeID" = e."typeID"
                        WHERE e."locationID" = $1 AND e."flag" = $3),
                      (SELECT "volume" FROM "invTypes" WHERE "typeID" = $2)"#,
            &[&(ship_id as i32), &(s.type_id as i32), &(FLAG_CARGO as i32)],
        )
        .await;
    let (used, volume) = room.map_or((0.0, f64::MAX), |r| (float(&r, 0).unwrap_or(0.0), float(&r, 1).unwrap_or(0.0)));
    let capacity = crate::svc::moves::active_cargo_capacity(ctx, ctx.db, ship_id).await.ok().flatten().unwrap_or(0.0);
    let free = (capacity - used).max(0.0);
    if volume > free + 1e-6 {
        return Err(Outcome::custom(format!("Not enough room: the {} needs {volume:.0} m3 and only {free:.0} m3 is free.", s.name)));
    }
    let result = async {
        let mut client = crate::db::connect(ctx.db_url).await?;
        let mut txn = ItemTxn::begin(&mut client).await?;
        txn.move_to(s.id, ship_id, FLAG_CARGO).await?;
        if s.owner_id != char_id {
            txn.set_owner(s.id, char_id).await?;
        }
        txn.tx().execute(r#"DELETE FROM "posStructures" WHERE "itemID" = $1"#, &[&(s.id as i32)]).await?;
        txn.commit().await
    }
    .await;
    let events = match result {
        Ok(events) => events,
        Err(e) => {
            error!("scooping {}: {}", s.id, crate::logging::chain(&e));
            return Err(Outcome::custom("The structure could not be scooped."));
        }
    };
    items::notify(ctx.world, ctx.client_id, events);
    let mut world = ctx.world.lock();
    if let Some(park) = world.parks.get_mut(&system) {
        park.structures.remove(&s.id);
        let gone = entry(park.tick, "RemoveBalls", vec![Value::List(vec![Value::Int(s.id)])]);
        space::broadcast(&world, system, None, vec![gone]);
    }
    info!("{char_id} scooped {} ({})", s.name, s.id);
    Ok(Value::None)
}

// ---- state changes ---------------------------------------------------------------------

/// Saves a structure's new state and shows it to the system.
async fn set_state(ctx: &Ctx<'_>, system: i64, id: i64, new: i16, stamp: Option<i64>) -> Refused<()> {
    let saved = ctx
        .db
        .execute(r#"UPDATE "posStructures" SET "state" = $2, "stateTimestamp" = $3, "claimSince" = NULL WHERE "itemID" = $1"#, &[&(id as i32), &new, &stamp])
        .await;
    if let Err(e) = saved {
        error!("saving the state of structure {id}: {}", crate::logging::chain(&e));
        return Err(Outcome::custom("The structure could not do that just now."));
    }
    update(ctx.world, system, id, |s, tick| {
        s.state = new;
        s.stamp = stamp;
        if s.is_warp_disruptor() && new == state::UNANCHORED {
            return pos::disruptor_field_entries(tick, id, false);
        }
        Vec::new()
    });
    Ok(())
}

/// Changes a structure in its park and broadcasts `f`'s destiny entries,
/// then its new slim item.
fn update(world: &World, system: i64, id: i64, f: impl FnOnce(&mut Structure, i64) -> Vec<Value>) {
    let mut w = world.lock();
    let Some(park) = w.parks.get_mut(&system) else { return };
    let tick = park.tick;
    let Some(s) = park.structures.get_mut(&id) else { return };
    let mut entries = f(s, tick);
    entries.push(pos::slim_change(tick, s));
    space::broadcast(&w, system, None, entries);
}

/// A structure, and if it is a tower everything drawing on it, goes offline.
pub async fn take_offline(db: &impl GenericClient, world: &World, system: i64, id: i64) {
    let sql = r#"UPDATE "posStructures" SET "state" = $2, "stateTimestamp" = NULL, "claimSince" = NULL
                  WHERE ("itemID" = $1 OR "controlTowerID" = $1) AND "state" IN (2, 3, 4, 5)
                 RETURNING "itemID""#;
    let ids: Vec<i64> = match db.query(sql, &[&(id as i32), &state::ANCHORED]).await {
        Ok(rows) => rows.iter().filter_map(|r| int(r, 0)).collect(),
        Err(e) => {
            error!("taking structure {id} offline: {}", crate::logging::chain(&e));
            return;
        }
    };
    for id in ids {
        update(world, system, id, |s, _| {
            s.state = state::ANCHORED;
            s.stamp = None;
            Vec::new()
        });
    }
}

/// Nothing changes around a reinforced tower (CantPerformTowerReinforced).
fn not_reinforced(ctx: &Ctx<'_>, s: &Structure) -> Refused<()> {
    let tower = match s.tower {
        Some(t) => ctx.world.lock().parks.get(&ctx.session.solar_system_id.unwrap_or(0)).and_then(|p| p.structures.get(&t)).cloned(),
        None => Some(s.clone()),
    };
    match tower {
        Some(t) if t.invulnerable() => Err(message_with("CantPerformTowerReinforced", &[("tower", Value::text(&t.name))])),
        _ => Ok(()),
    }
}

// ---- combat ----------------------------------------------------------------------------

/// What a structure has left after a fight, so it outlives a restart.
pub async fn save_damage(db: &Client, id: i64, shield: f64, armor: f64, hull: f64) {
    let sql = r#"UPDATE "posStructures" SET "shieldCharge" = $2, "armorDamage" = $3, "hullDamage" = $4 WHERE "itemID" = $1"#;
    if let Err(e) = db.execute(sql, &[&(id as i32), &shield, &armor, &hull]).await {
        error!("saving the damage of structure {id}: {}", crate::logging::chain(&e));
    }
}

/// A tower fell below a quarter shield. Its strontium buys whole periods of
/// reinforcement, all burnt now; until they run out it takes no damage and
/// nothing around it changes. Without strontium it stays open.
pub async fn reinforce(world: &World, db: &mut Client, system: i64, tower_id: i64) {
    let Ok(Some(tower)) = tower_as_loaded(&*db, system, tower_id).await else { return };
    let fuel = Fuel::of(&*db, &tower, system).await;
    let per_period: Vec<(i64, i64)> = fuel
        .needs
        .iter()
        .filter(|(_, purpose, _)| *purpose == PURPOSE_REINFORCE)
        .filter(|(_, _, quantity)| *quantity > 0)
        .map(|&(type_id, _, quantity)| (type_id, quantity))
        .collect();
    let periods = per_period.iter().map(|(t, q)| fuel.held.get(t).copied().unwrap_or(0) / q).min().unwrap_or(0);
    if periods == 0 {
        info!("tower {tower_id} in {system} is below a quarter shield with no strontium to reinforce it");
        return;
    }
    let period = type_attributes(&*db, &[tower.type_id], &[ATTR_TOWER_PERIOD])
        .await
        .get(&(tower.type_id, ATTR_TOWER_PERIOD))
        .map_or(DEFAULT_PERIOD_MS, |ms| *ms as i64)
        * TICKS_PER_MS;
    let until = crate::bulkdata::filetime_now() + periods * period;
    let burn: Vec<(i64, i64)> = per_period.iter().map(|(t, q)| (*t, q * periods)).collect();
    let now_ms = crate::combat::now_ms();
    let shield = world.lock().parks.get(&system).and_then(|p| p.structures.get(&tower_id))
        .map_or_else(|| tower.vitals.shield.at(now_ms), |s| s.vitals.shield.at(now_ms));
    let saved = async {
        let tx = db.transaction().await?;
        let current = tx.query_opt(
            r#"SELECT 1 FROM entity e JOIN "posStructures" p USING ("itemID")
                WHERE e."itemID" = $1 AND p.state IN (4, 5) FOR UPDATE OF e, p"#, &[&(tower_id as i32)],
        ).await?;
        if current.is_none() || !take(&tx, tower_id, &burn).await? {
            return Ok(false);
        }
        tx.execute(
            r#"UPDATE "posStructures" SET "state" = $2, "stateTimestamp" = $3, "shieldCharge" = $4 WHERE "itemID" = $1"#,
            &[&(tower_id as i32), &state::REINFORCED, &until, &shield],
        )
        .await?;
        tx.commit().await?;
        Ok::<_, tokio_postgres::Error>(true)
    }
    .await;
    match saved {
        Ok(true) => {}
        Ok(false) => return,
        Err(e) => {
            error!("reinforcing tower {tower_id}: {}", crate::logging::chain(&e));
            return;
        }
    }
    update(world, system, tower_id, |s, _| {
        s.state = state::REINFORCED;
        s.stamp = Some(until);
        s.vitals.shield.set(now_ms, shield);
        s.vitals.shield.set_paused(now_ms, true);
        Vec::new()
    });
    info!("tower {tower_id} in {system} is reinforced for {periods} period(s)");
}

/// A structure was destroyed: it goes with what it held; a tower's
/// structures go offline and come loose from it.
pub async fn lost(db: &mut Client, id: i64, tower: bool) {
    let result = async {
        let tx = db.transaction().await?;
        tx.execute(r#"DELETE FROM entity WHERE "locationID" = $1"#, &[&(id as i32)]).await?;
        tx.execute(r#"DELETE FROM entity WHERE "itemID" = $1"#, &[&(id as i32)]).await?;
        tx.execute(r#"DELETE FROM "posStructures" WHERE "itemID" = $1"#, &[&(id as i32)]).await?;
        if tower {
            tx.execute(
                r#"UPDATE "posStructures"
                      SET "state" = CASE WHEN "state" IN (2, 3, 4, 5) THEN $2 ELSE "state" END,
                          "stateTimestamp" = CASE WHEN "state" IN (2, 3, 4, 5) THEN NULL ELSE "stateTimestamp" END,
                          "controlTowerID" = NULL
                    WHERE "controlTowerID" = $1"#,
                &[&(id as i32), &state::ANCHORED],
            )
            .await?;
        }
        tx.commit().await
    }
    .await;
    if let Err(e) = result {
        error!("clearing away structure {id}: {}", crate::logging::chain(&e));
    }
}

// ---- who may, and helpers -----------------------------------------------------------------

/// Whether the caller may act on `s` under a tower deploy setting: 0 needs
/// the Starbase Config role in the owning corporation, 1 any of its members,
/// 2 any member of its alliance. Owners may always.
fn allowed(ctx: &Ctx<'_>, s: &Structure, deploy: i16) -> bool {
    if ctx.session.char_id == Some(s.owner_id) {
        return true;
    }
    let member = ctx.session.corp_id == Some(s.corp_id);
    match deploy {
        2 => member || (s.alliance_id.is_some() && ctx.session.alliance_id == s.alliance_id),
        1 => member,
        _ => member && (ctx.session.has_corp_role(CORP_ROLE_STARBASE_CONFIG) || ctx.session.has_corp_role(CORP_ROLE_DIRECTOR)),
    }
}

async fn deploy_flag(db: &impl GenericClient, tower: i64, column: &str) -> i16 {
    let sql = format!(r#"SELECT "{column}" FROM "posStructures" WHERE "itemID" = $1"#);
    db.query_opt(sql.as_str(), &[&(tower as i32)]).await.ok().flatten().and_then(|r| int(&r, 0)).unwrap_or(0) as i16
}

/// Base attribute values: (typeID, attributeID) -> value.
async fn type_attributes(db: &impl GenericClient, types: &[i64], attrs: &[i32]) -> HashMap<(i64, i32), f64> {
    let types: Vec<i32> = types.iter().map(|t| *t as i32).collect();
    let sql = r#"SELECT "typeID", "attributeID", COALESCE("valueFloat", "valueInt"::float8)
                   FROM "dgmTypeAttributes" WHERE "typeID" = ANY($1) AND "attributeID" = ANY($2)"#;
    match db.query(sql, &[&types, &attrs]).await {
        Ok(rows) => rows
            .iter()
            .filter_map(|r| Some(((int(r, 0)?, int(r, 1)? as i32), float(r, 2)?)))
            .collect(),
        Err(e) => {
            error!("reading structure attributes: {}", crate::logging::chain(&e));
            HashMap::new()
        }
    }
}

async fn system_security(db: &impl GenericClient, system: i64) -> f64 {
    let sql = r#"SELECT "security" FROM "mapSolarSystems" WHERE "solarSystemID" = $1"#;
    db.query_opt(sql, &[&(system as i32)]).await.ok().flatten().and_then(|r| float(&r, 0)).unwrap_or(0.0)
}

async fn owner_name(ctx: &Ctx<'_>, owner: i64) -> String {
    let sql = r#"SELECT COALESCE((SELECT "corporationName" FROM "corporation" WHERE "corporationID" = $1),
                                 (SELECT "characterName" FROM "character_" WHERE "characterID" = $1), '')"#;
    ctx.query_one(sql, &[&(owner as i32)]).await.and_then(|r| crate::svc::rows::text(&r, 0)).unwrap_or_default()
}

async fn type_name(ctx: &Ctx<'_>, type_id: i64) -> String {
    let sql = r#"SELECT "typeName" FROM "invTypes" WHERE "typeID" = $1"#;
    ctx.query_one(sql, &[&(type_id as i32)]).await.and_then(|r| crate::svc::rows::text(&r, 0)).unwrap_or_default()
}

fn message_with(key: &str, args: &[(&str, Value)]) -> Outcome {
    Outcome::UserError { msg: key.into(), args: args.iter().map(|(k, v)| ((*k).into(), v.clone())).collect() }
}

// ---- fuel ------------------------------------------------------------------------------

/// What a tower burns and what it holds.
struct Fuel {
    /// (resource, purpose, quantity per period), the ones this system asks
    /// for: faction charters only in that faction's high-security space.
    needs: Vec<(i64, i64, i64)>,
    held: HashMap<i64, i64>,
    /// 0.75 where the owner's alliance holds sovereignty, else 1.
    factor: f64,
}

impl Fuel {
    async fn of(db: &impl GenericClient, tower: &Structure, system: i64) -> Fuel {
        let needs = db
            .query(
                r#"SELECT r."resourceTypeID", r."purpose", r."quantity"
                     FROM "invControlTowerResources" r, "mapSolarSystems" s
                     LEFT JOIN "mapConstellations" c ON c."constellationID" = s."constellationID"
                     LEFT JOIN "mapRegions" g ON g."regionID" = s."regionID"
                    WHERE r."controlTowerTypeID" = $1 AND s."solarSystemID" = $2
                      AND (r."minSecurityLevel" IS NULL OR round(s."security"::numeric, 1) >= r."minSecurityLevel")
                      AND (r."factionID" IS NULL OR r."factionID" = COALESCE(s."factionID", c."factionID", g."factionID"))"#,
                &[&(tower.type_id as i32), &(system as i32)],
            )
            .await
            .map(|rows| rows.iter().map(|r| (int(r, 0).unwrap_or(0), int(r, 1).unwrap_or(0), int(r, 2).unwrap_or(0))).collect())
            .unwrap_or_default();
        let held = db
            .query(r#"SELECT "typeID", sum("quantity") FROM entity WHERE "locationID" = $1 GROUP BY "typeID""#, &[&(tower.id as i32)])
            .await
            .map(|rows| rows.iter().map(|r| (int(r, 0).unwrap_or(0), int(r, 1).unwrap_or(0))).collect())
            .unwrap_or_default();
        let sovereign = db
            .query_opt(r#"SELECT 1 FROM "mapSolarSystems" WHERE "solarSystemID" = $1 AND "allianceID" = $2"#, &[&(system as i32), &tower.alliance_id.map(|a| a as i32)])
            .await
            .ok()
            .flatten()
            .is_some();
        Fuel { needs, held, factor: if sovereign { SOVEREIGN_FUEL } else { 1.0 } }
    }

    /// One period's burn, for power and CPU loads (fractions of the tower's
    /// output in use).
    fn burn(&self, power_load: f64, cpu_load: f64) -> Vec<(i64, i64)> {
        self.needs
            .iter()
            .filter_map(|&(type_id, purpose, quantity)| {
                let load = match purpose {
                    PURPOSE_ONLINE => 1.0,
                    PURPOSE_POWER => power_load,
                    PURPOSE_CPU => cpu_load,
                    _ => return None,
                };
                let amount = (quantity as f64 * self.factor * load).ceil() as i64;
                (amount > 0).then_some((type_id, amount))
            })
            .collect()
    }

    fn has_any(&self, purpose: i64) -> bool {
        self.needs.iter().any(|(t, p, _)| *p == purpose && self.held.get(t).copied().unwrap_or(0) > 0)
    }
}

/// Takes `burn` out of a tower's bay and records the period as paid for.
async fn consume(db: &tokio_postgres::Transaction<'_>, tower: i64, burn: &[(i64, i64)], now: i64) -> Result<bool, tokio_postgres::Error> {
    if !take(db, tower, burn).await? {
        return Ok(false);
    }
    db.execute(r#"UPDATE "posStructures" SET "lastFuel" = $2 WHERE "itemID" = $1"#, &[&(tower as i32), &now]).await?;
    Ok(true)
}

/// Takes `burn` out of a tower's bay, smallest stacks first.
async fn take(db: &tokio_postgres::Transaction<'_>, tower: i64, burn: &[(i64, i64)]) -> Result<bool, tokio_postgres::Error> {
    for &(type_id, mut amount) in burn {
        let stacks = db
            .query(
                r#"SELECT "itemID", "quantity" FROM entity WHERE "locationID" = $1 AND "typeID" = $2
                    AND "quantity" > 0 ORDER BY "quantity", "itemID" FOR UPDATE"#,
                &[&(tower as i32), &(type_id as i32)],
            )
            .await?;
        for r in stacks {
            if amount == 0 {
                break;
            }
            let (item, quantity) = (int(&r, 0).unwrap_or(0) as i32, int(&r, 1).unwrap_or(0));
            let take = amount.min(quantity);
            if take == quantity {
                db.execute(r#"DELETE FROM entity WHERE "itemID" = $1"#, &[&item]).await?;
            } else {
                db.execute(r#"UPDATE entity SET "quantity" = "quantity" - $2 WHERE "itemID" = $1"#, &[&item, &(take as i32)]).await?;
            }
            amount -= take;
        }
        if amount > 0 {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Every online tower whose period is up burns another period's fuel, or
/// goes offline without it. Run by the heartbeat.
pub async fn burn_fuel(world: &World, db: &mut Client) {
    let now = crate::bulkdata::filetime_now();
    let sql = r#"SELECT p."itemID", e."locationID", p."lastFuel",
                        COALESCE(a."valueFloat", a."valueInt"::float8, $2)
                   FROM "posStructures" p
                   JOIN entity e ON e."itemID" = p."itemID"
                   JOIN "invTypes" t ON t."typeID" = e."typeID"
                   LEFT JOIN "dgmTypeAttributes" a ON a."typeID" = e."typeID" AND a."attributeID" = $3
                  WHERE t."groupID" = $1 AND p."state" IN (4, 5)"#;
    let towers = match db.query(sql, &[&(pos::GROUP_CONTROL_TOWER as i32), &(DEFAULT_PERIOD_MS as f64), &ATTR_TOWER_PERIOD]).await {
        Ok(rows) => rows,
        Err(e) => {
            error!("reading the towers to fuel: {}", crate::logging::chain(&e));
            return;
        }
    };
    for r in towers {
        let (tower_id, system) = (int(&r, 0).unwrap_or(0), int(&r, 1).unwrap_or(0));
        let period = float(&r, 3).unwrap_or(DEFAULT_PERIOD_MS as f64) as i64 * TICKS_PER_MS;
        let last = int(&r, 2).unwrap_or(now);
        if now < last + period {
            continue;
        }
        let Ok(Some(tower)) = tower_as_loaded(db, system, tower_id).await else { continue };
        let fuel = Fuel::of(&*db, &tower, system).await;
        let (power, cpu) = tower_load(db, &tower).await;
        let burn = fuel.burn(power, cpu);
        let short = burn.iter().find(|(t, need)| fuel.held.get(t).copied().unwrap_or(0) < *need).copied();
        match short {
            Some((type_id, need)) => {
                info!("tower {tower_id} in {system} goes offline: it needs {need} of type {type_id}");
                take_offline(&*db, world, system, tower_id).await;
            }
            None => {
                let paid = async {
                    let tx = db.transaction().await?;
                    let current = tx.query_opt(
                        r#"SELECT 1 FROM entity e JOIN "posStructures" p USING ("itemID")
                            WHERE e."itemID" = $1 AND p.state IN (4, 5) AND p."lastFuel" = $2
                              FOR UPDATE OF e, p"#, &[&(tower_id as i32), &last],
                    ).await?;
                    if current.is_none() || !consume(&tx, tower_id, &burn, last + period).await? {
                        return Ok(false);
                    }
                    tx.commit().await?;
                    Ok::<_, tokio_postgres::Error>(true)
                }
                .await;
                match paid {
                    Ok(true) => moonmining::cycle(db, tower_id).await,
                    Ok(false) => {}
                    Err(e) => error!("fuelling tower {tower_id}: {}", crate::logging::chain(&e)),
                }
            }
        }
    }
}

/// A tower as the park loader reads it (owner, alliance, type).
async fn tower_as_loaded(db: &impl GenericClient, system: i64, tower: i64) -> Result<Option<Structure>, tokio_postgres::Error> {
    Ok(pos::load(db, system).await?.into_iter().find(|s| s.id == tower))
}

/// The fractions of a tower's power grid and CPU its online structures use.
async fn tower_load(db: &impl GenericClient, tower: &Structure) -> (f64, f64) {
    let sql = r#"SELECT e."typeID" FROM "posStructures" p JOIN entity e ON e."itemID" = p."itemID"
                  WHERE p."controlTowerID" = $1 AND p."state" IN (2, 3, 4, 5)"#;
    let drawing: Vec<i64> = db.query(sql, &[&(tower.id as i32)]).await.map(|rows| rows.iter().filter_map(|r| int(r, 0)).collect()).unwrap_or_default();
    let mut types = drawing.clone();
    types.push(tower.type_id);
    let a = type_attributes(db, &types, &[ATTR_CPU, ATTR_POWER, ATTR_CPU_OUTPUT, ATTR_POWER_OUTPUT]).await;
    let cpu = cpu_factors(db, tower.type_id, &types).await;
    let get = |t: i64, attr: i32| a.get(&(t, attr)).copied().unwrap_or(0.0) * if attr == ATTR_CPU { cpu(t) } else { 1.0 };
    let load = |attr: i32, output: i32| {
        let out = get(tower.type_id, output);
        if out <= 0.0 { 0.0 } else { drawing.iter().map(|t| get(*t, attr)).sum::<f64>() / out }
    };
    (load(ATTR_POWER, ATTR_POWER_OUTPUT), load(ATTR_CPU, ATTR_CPU_OUTPUT))
}

/// What each type's CPU need is multiplied by at this tower: its
/// controlTowerMoonHarvesterCPUBonus (a percentage) for moon harvesters, 1
/// for everything else.
async fn cpu_factors(db: &impl GenericClient, tower_type: i64, types: &[i64]) -> impl Fn(i64) -> f64 {
    let bonus = type_attributes(db, &[tower_type], &[ATTR_HARVESTER_CPU_BONUS]).await.get(&(tower_type, ATTR_HARVESTER_CPU_BONUS)).copied().unwrap_or(0.0);
    let ids: Vec<i32> = types.iter().map(|t| *t as i32).collect();
    let sql = r#"SELECT "typeID" FROM "invTypes" WHERE "typeID" = ANY($1) AND "groupID" = $2"#;
    let harvesters: Vec<i64> = match db.query(sql, &[&ids, &(moonmining::GROUP_MOON_MINING as i32)]).await {
        Ok(rows) => rows.iter().filter_map(|r| int(r, 0)).collect(),
        Err(e) => {
            error!("reading which structures are harvesters: {}", crate::logging::chain(&e));
            Vec::new()
        }
    };
    move |t| if harvesters.contains(&t) { 1.0 + bonus / 100.0 } else { 1.0 }
}

// ---- sovereignty -----------------------------------------------------------------------

pub async fn ensure_sovereignty(db: &Client) -> Result<(), tokio_postgres::Error> {
    db.batch_execute(r#"
        ALTER TABLE "posStructures" ADD COLUMN IF NOT EXISTS "claimAllianceID" integer;
        ALTER TABLE "mapSolarSystems" ADD COLUMN IF NOT EXISTS "sovereigntySince" bigint;
    "#).await
}

/// Counts claiming towers and moves sovereignty where it has changed hands.
/// Run by the heartbeat.
pub async fn sovereignty(db: &mut Client, claim_delay_secs: i64) {
    if let Err(e) = resolve_sovereignty(db, claim_delay_secs).await {
        error!("updating sovereignty: {}", crate::logging::chain(&e));
    }
}

async fn resolve_sovereignty(db: &mut Client, claim_delay_secs: i64) -> Result<(), tokio_postgres::Error> {
    let tx = db.transaction().await?;
    // Bot startup can request an immediate pass as well as the heartbeat.
    tx.query_one("SELECT pg_advisory_xact_lock(365, 0)", &[]).await?;
    let now = crate::bulkdata::filetime_now();
    tx.execute(
        r#"WITH eligible AS (
               SELECT p."itemID",
                      CASE WHEN p.claim = 1 AND p.state IN (3, 4, 5) AND p."moonID" IS NOT NULL
                               AND s.security <= 0 AND COALESCE(s."factionID", 0) = 0
                           THEN NULLIF(co."allianceID", 0) END AS alliance
                 FROM "posStructures" p
                 JOIN entity e ON e."itemID" = p."itemID"
                 JOIN "invTypes" t ON t."typeID" = e."typeID" AND t."groupID" = 365
                 JOIN "mapSolarSystems" s ON s."solarSystemID" = e."locationID"
                 LEFT JOIN "corporation" co ON co."corporationID" = e."ownerID" AND co.deleted = 0
           )
           UPDATE "posStructures" p SET "claimAllianceID" = e.alliance,
                  "claimSince" = CASE WHEN e.alliance IS NULL THEN NULL
                                      WHEN p."claimAllianceID" IS NULL OR p."claimAllianceID" = e.alliance
                                          THEN COALESCE(p."claimSince", $1)
                                      ELSE $1 END
             FROM eligible e WHERE e."itemID" = p."itemID"
              AND (p."claimAllianceID" IS DISTINCT FROM e.alliance
                   OR (e.alliance IS NULL) <> (p."claimSince" IS NULL))"#,
        &[&now],
    ).await?;
    let standing = now.saturating_sub(claim_delay_secs.max(0).saturating_mul(TICKS_PER_SECOND));
    let claims = tx.query(
        r#"SELECT e."locationID", p."claimAllianceID", count(*)
             FROM "posStructures" p JOIN entity e USING ("itemID")
            WHERE p."claimAllianceID" IS NOT NULL AND p."claimSince" <= $1
            GROUP BY 1, 2"#,
        &[&standing],
    ).await?;
    let held = tx.query(
        r#"SELECT "solarSystemID", "allianceID" FROM "mapSolarSystems" WHERE "allianceID" IS NOT NULL"#, &[],
    ).await?;
    let mut by_system: HashMap<i64, Vec<(i64, i64)>> = HashMap::new();
    for r in &claims {
        if let (Some(system), Some(alliance)) = (int(r, 0), int(r, 1)) {
            by_system.entry(system).or_default().push((alliance, int(r, 2).unwrap_or(0)));
        }
    }
    let holders: HashMap<i64, i64> = held.iter().filter_map(|r| Some((int(r, 0)?, int(r, 1)?))).collect();
    let mut systems: Vec<i64> = by_system.keys().chain(holders.keys()).copied().collect();
    systems.sort_unstable();
    systems.dedup();
    let changes: Vec<(i64, Option<i64>, Option<i64>)> = systems
        .into_iter()
        .filter_map(|system| {
            let holder = holders.get(&system).copied();
            let winner = sovereign(by_system.get(&system).map_or(&[][..], |v| v), holder);
            (winner != holder).then_some((system, holder, winner))
        })
        .collect();
    // Many at once (null-sec settled at the start) are summed up, not listed.
    let many = changes.len() > SOV_CHANGES_LISTED;
    let mut moved = 0;
    for (system, _, winner) in &changes {
        tx.execute(
            r#"UPDATE "mapSolarSystems" SET "allianceID" = $2,
                   "sovereigntySince" = CASE WHEN $2::integer IS NULL THEN NULL ELSE $3::bigint END
               WHERE "solarSystemID" = $1"#,
            &[&(*system as i32), &winner.map(|a| a as i32), &now],
        ).await?;
        moved += 1;
    }
    tx.execute(
        r#"UPDATE "mapSolarSystems" SET "sovereigntySince" = $1
            WHERE "allianceID" IS NOT NULL AND "sovereigntySince" IS NULL"#, &[&now],
    ).await?;
    tx.commit().await?;
    for (system, holder, winner) in &changes {
        if many {
            debug!("sovereignty of {system} passes from {holder:?} to {winner:?}");
        } else {
            info!("sovereignty of {system} passes from {holder:?} to {winner:?}");
        }
    }
    if many {
        info!("sovereignty of {moved} systems changed hands");
    }
    Ok(())
}

/// More changes of sovereignty than this in one pass are logged as a count.
const SOV_CHANGES_LISTED: usize = 10;

/// The alliance with strictly the most claims; on a tie the holder keeps it
/// if it is among the leaders, else nobody holds it.
fn sovereign(claims: &[(i64, i64)], holder: Option<i64>) -> Option<i64> {
    let most = claims.iter().map(|(_, n)| *n).max()?;
    let leaders: Vec<i64> = claims.iter().filter(|(_, n)| *n == most).map(|(a, _)| *a).collect();
    match leaders.as_slice() {
        [only] => Some(*only),
        _ => holder.filter(|h| leaders.contains(h)),
    }
}

// ---- posMgr ----------------------------------------------------------------------------

/// posMgr, bound per solar system (moniker.GetPOSMgr); also called unbound
/// for the fuel table.
pub async fn pos_mgr(call: &Call, ctx: &Ctx<'_>) -> Option<Outcome> {
    let tower = call.arg_id(0);
    let flag = |i: usize| call.args.get(i).map(|v| v.as_i64().unwrap_or(0) != 0).unwrap_or(false);
    let result = match call.method.as_str() {
        "GetControlTowerFuelRequirements" => {
            let header = ["controlTowerTypeID", "resourceTypeID", "purpose", "quantity", "minSecurityLevel", "factionID"];
            let sql = r#"SELECT "controlTowerTypeID", "resourceTypeID", "purpose", "quantity", "minSecurityLevel", "factionID"
                           FROM "invControlTowerResources" ORDER BY 1, 3, 2"#;
            Ok(ctx.rowset(&header, sql, &[]).await)
        }
        "AnchorStructure" => {
            let at = call.args.get(1).and_then(|v| v.seq()).map(|p| p.iter().filter_map(|c| c.as_f64()).collect::<Vec<_>>());
            match at.as_deref() {
                Some([x, y, z]) => anchor(ctx, tower, Vec3::new(*x, *y, *z)).await,
                _ => Err(Outcome::custom("That is not a position.")),
            }
        }
        "GetMoonForTower" => Ok(moonmining::moon_for_tower(ctx, tower).await),
        "GetTowerClaim" => Ok(Value::Bool(setting(ctx, tower, r#""claim""#).await.and_then(|v| v.as_i64()) == Some(1))),
        "SetTowerClaim" => {
            set_tower_claim(ctx, tower, flag(1)).await
        }
        "GetTowerSentrySettings" => {
            let sql = r#"SELECT "sentryStanding", "sentryStatus", "sentryStandingDrop", "sentryStatusDrop", "sentryAggression", "sentryWar"
                           FROM "posStructures" WHERE "itemID" = $1"#;
            let r = ctx.query_one(sql, &[&(tower as i32)]).await;
            let cell = |i: usize| r.as_ref().map_or(Value::None, |r| crate::svc::rows::cell(r, i));
            let truth = |i: usize| Value::Bool(cell(i).as_i64() == Some(1));
            Ok(Value::Tuple(vec![cell(0), cell(1), truth(2), truth(3), truth(4), truth(5)]))
        }
        "SetTowerSentrySettings" => {
            let opt = |i: usize| call.args.get(i).and_then(|v| v.as_f64());
            let sentry = crate::guard::Sentry {
                standing: opt(1),
                status: opt(2),
                standing_drop: flag(3),
                status_drop: flag(4),
                aggression: flag(5),
                war: flag(6),
            };
            let system = ctx.session.solar_system_id.unwrap_or(0);
            if let Some(s) = ctx.world.lock().parks.get_mut(&system).and_then(|p| p.structures.get_mut(&tower)) {
                s.sentry = sentry;
            }
            configure(
                ctx,
                tower,
                r#""sentryStanding" = $2, "sentryStatus" = $3, "sentryStandingDrop" = $4, "sentryStatusDrop" = $5,
                   "sentryAggression" = $6, "sentryWar" = $7"#,
                &[&opt(1), &opt(2), &i16::from(flag(3)), &i16::from(flag(4)), &i16::from(flag(5)), &i16::from(flag(6))],
            )
            .await
        }
        // SetTowerPassword(towerID, password or False to keep it, allowCorp, allowAlliance)
        "SetTowerPassword" => {
            let password = call.arg_text(1);
            let (corp, alliance) = (flag(2), flag(3));
            let saved = configure(
                ctx,
                tower,
                r#""password" = COALESCE($2, "password"), "allowCorp" = $3, "allowAlliance" = $4"#,
                &[&password, &i16::from(corp), &i16::from(alliance)],
            )
            .await;
            if saved.is_ok() {
                // The field retunes on the next pulse (pos::step).
                let system = ctx.session.solar_system_id.unwrap_or(0);
                if let Some(s) = ctx.world.lock().parks.get_mut(&system).and_then(|p| p.structures.get_mut(&tower)) {
                    if let Some(p) = &password {
                        s.harmonic = pos::harmonic(p);
                    }
                    s.allow_corp = corp;
                    s.allow_alliance = alliance;
                }
            }
            saved
        }
        // SetShipPassword(password): the ship's harmonic, until it leaves space.
        "SetShipPassword" => ship_password(ctx, &call.arg_text(0).unwrap_or_default()),
        "GetStarbasePermissions" => Ok(permissions(ctx, tower).await),
        "SetStarbasePermissions" => set_permissions(ctx, tower, call).await,
        "GetMoonProcessInfoForTower" => Ok(moonmining::process_info(ctx, tower).await),
        "LinkResourceForTower" => match may_configure(ctx, tower) {
            Ok(()) => moonmining::link(ctx, tower, call.args.get(1)).await,
            Err(refusal) => Err(refusal),
        },
        "ChangeStructureProvisionType" => match may_configure(ctx, tower) {
            Ok(()) => moonmining::change_provision(ctx, tower, call.arg_id(1), call.arg_id(2)).await,
            Err(refusal) => Err(refusal),
        },
        // The client's shift-click debug cycle: production runs on the tower's
        // fuel period only.
        "RunMoonProcessCycleforTower" => Ok(Value::None),
        _ => return None,
    };
    Some(result.map_or_else(|refusal| refusal, Outcome::Value))
}

async fn set_tower_claim(ctx: &Ctx<'_>, tower: i64, claim: bool) -> Refused<Value> {
    let Seen { s, system, .. } = seen(ctx, tower).ok_or_else(|| Outcome::custom("That tower is not here."))?;
    if !s.is_tower() {
        return Err(Outcome::custom("Only a control tower can claim sovereignty."));
    }
    if claim {
        let eligible = ctx.query_one(
            r#"SELECT 1 FROM entity e JOIN "corporation" c ON c."corporationID" = e."ownerID"
                 JOIN "mapSolarSystems" s ON s."solarSystemID" = e."locationID"
                WHERE e."itemID" = $1 AND s."solarSystemID" = $2 AND s.security <= 0
                  AND COALESCE(s."factionID", 0) = 0 AND c."allianceID" > 0 AND c.deleted = 0"#,
            &[&(tower as i32), &(system as i32)],
        ).await.is_some();
        if !eligible {
            return Err(Outcome::custom("Only a corporation in an alliance can claim sovereignty, and the system must be outside NPC territory in nullsec."));
        }
    }
    configure(
        ctx, tower,
        r#""claim" = $2, "claimSince" = CASE WHEN "claim" = $2 THEN "claimSince" ELSE NULL END,
           "claimAllianceID" = CASE WHEN "claim" = $2 THEN "claimAllianceID" ELSE NULL END"#,
        &[&i16::from(claim)],
    ).await
}

/// A ship takes the harmonic of a password, unless it sits inside a force
/// field, which would tear it apart.
fn ship_password(ctx: &Ctx<'_>, password: &str) -> Refused<Value> {
    let (Some(system), Some(ship_id)) = (ctx.session.solar_system_id, ctx.session.ship_id) else {
        return Err(Outcome::custom("You are not in space."));
    };
    let mut world = ctx.world.lock();
    let Some(park) = world.parks.get_mut(&system) else { return Err(Outcome::custom("You are not in space.")) };
    let Some(pos) = park.ships.get(&ship_id).map(|s| s.pos) else { return Err(Outcome::custom("You are not in space.")) };
    let inside = park.inside_force_field(pos);
    if inside {
        return Err(message("CantChangeShipHarmonicWithinForceField"));
    }
    let tick = park.tick;
    let Some(ship) = park.ships.get_mut(&ship_id) else { return Err(Outcome::custom("You are not in space.")) };
    ship.harmonic = pos::harmonic(password);
    let key = (ship.harmonic, ship.pilot.corp_id, ship.pilot.alliance_id.unwrap_or(-1));
    let set = pos::harmonic_entry(tick, ship_id, key, false);
    space::broadcast(&world, system, None, vec![set]);
    Ok(Value::None)
}

/// One column of a structure's settings row.
async fn setting(ctx: &Ctx<'_>, id: i64, column: &str) -> Option<Value> {
    let sql = format!(r#"SELECT {column} FROM "posStructures" WHERE "itemID" = $1"#);
    ctx.query_one(&sql, &[&(id as i32)]).await.map(|r| crate::svc::rows::cell(&r, 0))
}

/// Whether the caller may configure the tower (Starbase Config, or owner).
fn may_configure(ctx: &Ctx<'_>, tower: i64) -> Refused<()> {
    let Some(Seen { s, .. }) = seen(ctx, tower) else { return Err(Outcome::custom("That tower is not here.")) };
    not_reinforced(ctx, &s)?;
    if !allowed(ctx, &s, 0) {
        return Err(message("CantConfigureCorpStarbaseRoleLacking"));
    }
    Ok(())
}

/// Changes a tower's settings, for those allowed to configure it.
async fn configure(ctx: &Ctx<'_>, tower: i64, set: &str, params: &[&(dyn tokio_postgres::types::ToSql + Sync)]) -> Refused<Value> {
    let Some(Seen { s, .. }) = seen(ctx, tower) else { return Err(Outcome::custom("That tower is not here.")) };
    not_reinforced(ctx, &s)?;
    if !allowed(ctx, &s, 0) {
        return Err(message("CantConfigureCorpStarbaseRoleLacking"));
    }
    let sql = format!(r#"UPDATE "posStructures" SET {set} WHERE "itemID" = $1"#);
    let id = tower as i32;
    let mut all: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = vec![&id];
    all.extend_from_slice(params);
    match ctx.db.execute(sql.as_str(), &all).await {
        Ok(_) => Ok(Value::None),
        Err(e) => {
            error!("configuring tower {tower}: {}", crate::logging::chain(&e));
            Err(Outcome::custom("The tower could not be configured."))
        }
    }
}

const USAGE_HEADER: [&str; 4] = ["structureID", "viewput", "viewputtake", "use"];

/// (deployFlags, usageFlagsList): who may anchor, unanchor, online and
/// offline around the tower; and per structure, who may look, take and use.
async fn permissions(ctx: &Ctx<'_>, tower: i64) -> Value {
    let sql = r#"SELECT "deployAnchor", "deployUnanchor", "deployOnline", "deployOffline" FROM "posStructures" WHERE "itemID" = $1"#;
    let r = ctx.query_one(sql, &[&(tower as i32)]).await;
    let flag = |i: usize| Value::Int(r.as_ref().and_then(|r| int(r, i)).unwrap_or(0));
    let deploy = Value::Dict(
        ["anchor", "unanchor", "online", "offline"].iter().enumerate().map(|(i, k)| (Value::bytes(*k), flag(i))).collect(),
    );
    let sql = r#"SELECT "itemID", "useViewPut", "useViewPutTake", "useUse" FROM "posStructures"
                  WHERE "controlTowerID" = $1 ORDER BY "itemID""#;
    let usage = rowset(&USAGE_HEADER, ctx.query(sql, &[&(tower as i32)]).await.iter().map(|r| Value::List(line(r))).collect());
    Value::Tuple(vec![deploy, usage])
}

async fn set_permissions(ctx: &Ctx<'_>, tower: i64, call: &Call) -> Refused<Value> {
    let deploy = |key: &str| -> i16 {
        match call.args.get(1) {
            Some(Value::Dict(d)) => d.iter().find(|(k, _)| *k == Value::bytes(key)).and_then(|(_, v)| v.as_i64()).unwrap_or(0) as i16,
            _ => 0,
        }
    };
    configure(
        ctx,
        tower,
        r#""deployAnchor" = $2, "deployUnanchor" = $3, "deployOnline" = $4, "deployOffline" = $5"#,
        &[&deploy("anchor"), &deploy("unanchor"), &deploy("online"), &deploy("offline")],
    )
    .await?;
    // usageFlagsList: a util.Rowset of [structureID, viewput, viewputtake, use].
    let lines = match call.args.get(2) {
        Some(Value::Instance { state, .. }) => match state.as_ref() {
            Value::Dict(d) => d.iter().find(|(k, _)| *k == Value::bytes("lines")).and_then(|(_, v)| v.seq()).map(|l| l.to_vec()),
            _ => None,
        },
        _ => None,
    };
    for l in lines.unwrap_or_default() {
        let Some(l) = l.seq() else { continue };
        let at = |i: usize| l.get(i).and_then(|v| v.as_i64()).unwrap_or(0);
        let sql = r#"UPDATE "posStructures" SET "useViewPut" = $3, "useViewPutTake" = $4, "useUse" = $5
                      WHERE "itemID" = $1 AND "controlTowerID" = $2"#;
        ctx.execute(sql, &[&(at(0) as i32), &(tower as i32), &(at(1) as i16), &(at(2) as i16), &(at(3) as i16)]).await;
    }
    Ok(Value::None)
}

// ---- reach -----------------------------------------------------------------------------

/// What a pilot does with a structure's contents, each governed by one of
/// the tower's usage settings (moonmining.StructureAccess).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    /// Look inside and put things in: "viewput".
    ViewPut,
    /// Take things out as well: "viewputtake".
    Take,
    /// Use what it does: "use".
    Use,
}

/// Whether `id` is a structure here, and if so whether the caller's ship is
/// close enough and the structure's usage setting lets it (an owner always
/// may). None for anything that is not a structure.
pub async fn may(ctx: &Ctx<'_>, id: i64, access: Access) -> Option<bool> {
    let Seen { s, distance, .. } = seen(ctx, id)?;
    if distance > crate::loot::TRANSFER_RANGE {
        return Some(false);
    }
    // Putting a reaction into a reactor is using it.
    let access = if access == Access::ViewPut && s.group_id == moonmining::GROUP_REACTOR { Access::Use } else { access };
    let column = match access {
        Access::ViewPut => "useViewPut",
        Access::Take => "useViewPutTake",
        Access::Use => "useUse",
    };
    Some(allowed(ctx, &s, deploy_flag(ctx.db, s.id, column).await))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tower_placement_near_a_moon_uses_the_warp_in_range() {
        let moons = [crate::ballpark::Celestial {
            id: 1, type_id: 14, group_id: GROUP_MOON, name: "Moon".into(),
            pos: Vec3::ZERO, radius: 130_000.0, undock: None, jump: None,
        }];
        let ship = Vec3::new(1_130_000.0, 0.0, 0.0);
        let at = Vec3::new(1_180_000.0, 20_000.0, 0.0);
        assert!((at - ship).len() > ANCHOR_REACH);
        let moon = anchor_moon(&moons, at, ship, MOON_RANGE).unwrap();
        assert_eq!(tower_anchor_point(moon, at, ship, 3_394.0, MOON_RANGE), at);
        let far = Vec3::new(1_300_000.0, 0.0, 0.0);
        assert!(anchor_moon(&moons, far, far, MOON_RANGE).is_none());
        let surface = Vec3::new(130_000.0, 0.0, 0.0);
        let moon = anchor_moon(&moons, surface, surface, MOON_RANGE).unwrap();
        assert_eq!(tower_anchor_point(moon, surface, surface, 3_394.0, MOON_RANGE), Vec3::new(133_894.0, 0.0, 0.0));
    }

    #[test]
    fn the_most_claims_win_and_a_tie_keeps_the_holder() {
        assert_eq!(sovereign(&[(1, 2), (2, 1)], None), Some(1));
        assert_eq!(sovereign(&[(1, 2), (2, 2)], Some(2)), Some(2));
        assert_eq!(sovereign(&[(1, 2), (2, 2)], Some(3)), None);
        assert_eq!(sovereign(&[], Some(3)), None);
    }

    #[test]
    fn power_and_cpu_fuel_burn_with_the_load() {
        let fuel = Fuel { needs: vec![(44, PURPOSE_ONLINE, 4), (16273, PURPOSE_POWER, 150), (16272, PURPOSE_CPU, 150), (16275, 4, 200)], held: HashMap::new(), factor: 1.0 };
        assert_eq!(fuel.burn(0.5, 0.0), vec![(44, 4), (16273, 75)]);
        let sovereign = Fuel { factor: SOVEREIGN_FUEL, ..fuel };
        assert_eq!(sovereign.burn(1.0, 1.0), vec![(44, 3), (16273, 113), (16272, 113)]);
    }
}
