//! Outposts. A construction platform (group 307) is launched and anchored
//! like a starbase structure (starbase.rs). Once anchored, its owner loads
//! the materials its Show Info lists into its hold and presses Build
//! (invcontainer.Build); the materials are used up and the platform becomes
//! the conquerable station its stationTypeID attribute names: a staStations
//! row (docking, services, offices), a mapDenormalize row (its place and name
//! on the map), assembly lines for its kind, and a celestial in the system's
//! park in place of the platform. Anything else left in the hold goes to the
//! new station's hangar.
//!
//! Outposts are numbered from 61,000,000, as conquerable stations are on
//! Tranquility; the corporation that owned the platform owns the station.
//! `/outpostmats` (gm.rs) fills a platform's hold with what is missing.

use eve_marshal::Value;
use tokio_postgres::GenericClient;
use tracing::{error, info};

use crate::ballpark::{Celestial, Vec3};
use crate::items::{ItemTxn, NewItem};
use crate::pos::Structure;
use crate::services::{Ctx, Outcome};
use crate::svc::rows::{float, int, text};
use crate::svc::space;
use crate::world::World;

pub const OUTPOST_ID_MIN: i32 = 61_000_000;
pub const OUTPOST_ID_MAX: i32 = 63_999_999;
/// dgmAttributeTypes stationTypeID: the outpost a platform becomes.
const ATTR_STATION_TYPE_ID: i32 = 472;
const GROUP_STATION: i64 = 15;
const CATEGORY_SKILL: i32 = 16;
const ACTIVITY_MANUFACTURING: i32 = 1;
const FLAG_HANGAR: i64 = 4;
/// eveConstants maxConfigureDistance, with slack for the two simulations.
const BUILD_RANGE: f64 = 5_000.0 * 1.25;

/// The assembly lines an outpost opens with: (assemblyLineTypeID, count).
fn assembly_lines(station_type: i64) -> &'static [(i32, i32)] {
    match station_type {
        // Amarr Factory Outpost: Amarr Outpost Manufacturing.
        21644 => &[(31, 10)],
        // Caldari Research Outpost: ME and PE research, copying, invention.
        21642 => &[(32, 4), (33, 4), (34, 4), (40, 2)],
        // Gallente Administrative and Minmatar Service Outposts: manufacturing.
        _ => &[(13, 4)],
    }
}

/// What a platform needs in its hold to build its outpost, as (typeID,
/// quantity, name): its blueprint's consumed manufacturing inputs, the list
/// Show Info gives under Material Requirements.
pub async fn requirements(db: &impl GenericClient, platform_type: i64) -> Result<Vec<(i64, i64, String)>, tokio_postgres::Error> {
    let rows = db.query(
        r#"SELECT m."requiredTypeID", m."quantity", COALESCE(t."typeName", '')
             FROM "invBlueprintTypes" b
             JOIN "TL2MaterialsForTypeWithActivity" m ON m."typeID" = b."blueprintTypeID" AND m."activity" = $2
             JOIN "invTypes" t ON t."typeID" = m."requiredTypeID"
             JOIN "invGroups" g ON g."groupID" = t."groupID"
            WHERE b."productTypeID" = $1 AND m."damagePerJob" >= 1 AND m."quantity" > 0 AND g."categoryID" <> $3
            ORDER BY m."requiredTypeID""#,
        &[&(platform_type as i32), &ACTIVITY_MANUFACTURING, &CATEGORY_SKILL],
    ).await?;
    Ok(rows.iter().map(|r| (int(r, 0).unwrap_or(0), int(r, 1).unwrap_or(0), text(r, 2).unwrap_or_default())).collect())
}

/// What the platform holds, by type.
async fn contents(db: &impl GenericClient, platform: i64) -> Result<Vec<(i64, i64, i64)>, tokio_postgres::Error> {
    let rows = db.query(
        r#"SELECT "itemID", "typeID", "quantity" FROM entity WHERE "locationID" = $1 ORDER BY "itemID""#,
        &[&(platform as i32)],
    ).await?;
    Ok(rows.iter().map(|r| (int(r, 0).unwrap_or(0), int(r, 1).unwrap_or(0), int(r, 2).unwrap_or(0))).collect())
}

/// The platform in the caller's system.
fn platform_here(ctx: &Ctx<'_>, platform: Option<i64>) -> Result<(Structure, i64, f64), String> {
    let (Some(ship), Some(system)) = (ctx.session.ship_id, ctx.session.solar_system_id) else {
        return Err("You must be in space beside an outpost construction platform.".into());
    };
    let world = ctx.world.lock();
    let park = world.parks.get(&system).ok_or("You are not in space.")?;
    let found = match platform {
        Some(id) => park.structures.get(&id).filter(|s| s.is_platform()),
        None => park.structures.values().filter(|s| s.is_platform())
            .min_by(|a, b| {
                let d = |s: &Structure| park.surface_distance(ship, s.id).unwrap_or(f64::MAX);
                d(a).total_cmp(&d(b))
            }),
    };
    let s = found.ok_or("There is no outpost construction platform here.")?.clone();
    let distance = park.surface_distance(ship, s.id).unwrap_or(f64::MAX);
    Ok((s, system, distance))
}

/// invcontainer.Build on a platform's hold.
pub async fn build(ctx: &Ctx<'_>, platform: i64) -> Outcome {
    let (s, system, distance) = match platform_here(ctx, Some(platform)) {
        Ok(found) => found,
        Err(why) => return Outcome::custom(why),
    };
    if s.owner_id != ctx.session.char_id.unwrap_or(0) && Some(s.corp_id) != ctx.session.corp_id {
        return Outcome::custom("Only the platform's owner or its corporation can build the outpost.");
    }
    if distance > BUILD_RANGE {
        return Outcome::custom("Move within 5 km of the platform to build the outpost.");
    }
    if !s.fully_anchored(crate::bulkdata::filetime_now()) {
        return Outcome::custom("The platform must finish anchoring before the outpost can be built.");
    }
    let result = async {
        let mut client = crate::db::connect(ctx.db_url).await?;
        create(&mut client, system, &s).await
    }.await;
    match result {
        Ok(Ok((outpost, events))) => {
            info!("outpost {} ({}) built from platform {} in {system} for corporation {}", outpost.name, outpost.id, s.id, outpost.corp);
            crate::items::notify(ctx.world, ctx.client_id, events);
            show(ctx.world, system, s.id, outpost);
            Outcome::Value(Value::None)
        }
        Ok(Err(why)) => Outcome::custom(why),
        Err(e) => {
            error!("building the outpost from platform {}: {}", s.id, crate::logging::chain(&e));
            Outcome::custom("The outpost could not be built just now.")
        }
    }
}

struct Outpost {
    id: i64,
    type_id: i64,
    name: String,
    pos: Vec3,
    radius: f64,
    corp: i64,
    undock: Option<(Vec3, Vec3)>,
}

type Built = Result<(Outpost, Vec<Value>), String>;

async fn create(client: &mut tokio_postgres::Client, system: i64, s: &Structure) -> Result<Built, tokio_postgres::Error> {
    let mut txn = ItemTxn::begin(client).await?;
    // One outpost at a time: the new ID and the one-outpost rule hold.
    txn.tx().query_one("SELECT pg_advisory_xact_lock($1, $2)", &[&307i32, &0i32]).await?;
    let Some(p) = txn.tx().query_opt(
        r#"SELECT e."x", e."y", e."z", COALESCE(co."corporationName", cc."corporationName", ''),
                  (SELECT a."valueInt" FROM "dgmTypeAttributes" a WHERE a."typeID" = e."typeID" AND a."attributeID" = $3)
             FROM entity e
             LEFT JOIN "corporation" co ON co."corporationID" = e."ownerID"
             LEFT JOIN "character_" ch ON ch."characterID" = e."ownerID"
             LEFT JOIN "corporation" cc ON cc."corporationID" = ch."corporationID"
            WHERE e."itemID" = $1 AND e."locationID" = $2"#,
        &[&(s.id as i32), &(system as i32), &ATTR_STATION_TYPE_ID],
    ).await? else {
        return Ok(Err("That platform is no longer here.".into()));
    };
    let Some(station_type) = int(&p, 4) else {
        return Ok(Err("This platform does not name the outpost it builds.".into()));
    };
    let pos = Vec3::new(float(&p, 0).unwrap_or(0.0), float(&p, 1).unwrap_or(0.0), float(&p, 2).unwrap_or(0.0));
    let corp_name = text(&p, 3).unwrap_or_default();
    let corp = s.corp_id;

    if let Some(row) = txn.tx().query_opt(
        r#"SELECT COALESCE("stationName", '') FROM "staStations"
            WHERE "solarSystemID" = $1 AND "stationID" BETWEEN $2 AND $3 LIMIT 1"#,
        &[&(system as i32), &OUTPOST_ID_MIN, &OUTPOST_ID_MAX],
    ).await? {
        return Ok(Err(format!("This system already has an outpost ({}).", text(&row, 0).unwrap_or_default())));
    }

    // The materials Show Info lists, from the hold, all of them.
    let needed = requirements(txn.tx(), s.type_id).await?;
    let held = contents(txn.tx(), s.id).await?;
    let missing: Vec<String> = needed.iter().filter_map(|(type_id, quantity, name)| {
        let have: i64 = held.iter().filter(|(_, t, _)| t == type_id).map(|(_, _, q)| q).sum();
        (have < *quantity).then(|| format!("{} x {name} (have {have})", quantity - have))
    }).collect();
    if !missing.is_empty() {
        return Ok(Err(format!("The platform's hold is missing: {}.", missing.join(", "))));
    }
    let mut leftover: Vec<i64> = Vec::new();
    let mut remaining: Vec<(i64, i64)> = needed.iter().map(|(t, q, _)| (*t, *q)).collect();
    for (item, type_id, quantity) in &held {
        let Some(need) = remaining.iter_mut().find(|(t, q)| t == type_id && *q > 0) else {
            leftover.push(*item);
            continue;
        };
        if *quantity <= need.1 {
            need.1 -= quantity;
            txn.destroy(*item).await?;
        } else {
            txn.set_quantity(*item, quantity - need.1).await?;
            need.1 = 0;
            leftover.push(*item);
        }
    }

    let outpost = match raise(&mut txn, system, corp, &corp_name, station_type, pos).await? {
        Ok(o) => o,
        Err(m) => return Ok(Err(m)),
    };
    for item in leftover {
        txn.move_to(item, outpost.id, FLAG_HANGAR).await?;
    }
    txn.tx().execute(r#"DELETE FROM "posStructures" WHERE "itemID" = $1"#, &[&(s.id as i32)]).await?;
    txn.destroy(s.id).await?;
    let events = txn.commit().await?;
    Ok(Ok((outpost, events)))
}

/// Raises an outpost of `station_type` at `pos` in `system` for `corp`: its
/// station, map entry, static location, item and assembly lines.
async fn raise(txn: &mut ItemTxn<'_>, system: i64, corp: i64, corp_name: &str, station_type: i64, pos: Vec3) -> Result<Result<Outpost, String>, tokio_postgres::Error> {
    let Some(sys) = txn.tx().query_opt(
        r#"SELECT COALESCE("solarSystemName", ''), "constellationID", "regionID", COALESCE("security", 0)::float8
             FROM "mapSolarSystems" WHERE "solarSystemID" = $1"#,
        &[&(system as i32)],
    ).await? else {
        return Ok(Err("This system is not on the map.".into()));
    };
    let system_name = text(&sys, 0).unwrap_or_default();
    let constellation = int(&sys, 1).unwrap_or(0) as i32;
    let region = int(&sys, 2).unwrap_or(0) as i32;
    let security = float(&sys, 3).unwrap_or(0.0);

    let Some(kind) = txn.tx().query_opt(
        r#"SELECT st."operationID", st."dockEntryX", st."dockEntryY", st."dockEntryZ",
                  st."dockOrientationX", st."dockOrientationY", st."dockOrientationZ",
                  COALESCE(st."reprocessingEfficiency", 0.5)::float8, COALESCE(t."radius", 15000)::float8,
                  COALESCE(t."typeName", 'Outpost')
             FROM "staStationTypes" st JOIN "invTypes" t ON t."typeID" = st."stationTypeID"
            WHERE st."stationTypeID" = $1"#,
        &[&(station_type as i32)],
    ).await? else {
        return Ok(Err("That outpost type cannot be built on this server.".into()));
    };
    let operation = int(&kind, 0).unwrap_or(0) as i32;
    let undock = float(&kind, 1).map(|x| {
        (
            Vec3::new(x, float(&kind, 2).unwrap_or(0.0), float(&kind, 3).unwrap_or(0.0)),
            Vec3::new(float(&kind, 4).unwrap_or(0.0), float(&kind, 5).unwrap_or(0.0), float(&kind, 6).unwrap_or(0.0)),
        )
    });
    let reprocessing = float(&kind, 7).unwrap_or(0.5);
    let radius = float(&kind, 8).unwrap_or(15_000.0);
    let type_name = text(&kind, 9).unwrap_or_default();

    let id: i32 = txn.tx().query_one(
        r#"SELECT GREATEST($1,
                  COALESCE((SELECT max("stationID") FROM "staStations" WHERE "stationID" BETWEEN $1 AND $2), 0) + 1,
                  COALESCE((SELECT max("itemID") FROM "mapDenormalize" WHERE "itemID" BETWEEN $1 AND $2), 0) + 1,
                  COALESCE((SELECT max("locationID") FROM "eveStaticLocations" WHERE "locationID" BETWEEN $1 AND $2), 0) + 1)"#,
        &[&OUTPOST_ID_MIN, &OUTPOST_ID_MAX],
    ).await?.get(0);
    let mut name = if corp_name.is_empty() { format!("{system_name} - {type_name}") } else { format!("{system_name} - {corp_name} Outpost") };
    if name.chars().count() > 100 {
        name = name.chars().take(100).collect();
    }

    txn.tx().execute(
        r#"INSERT INTO "staStations" ("stationID", security, "dockingCostPerVolume", "maxShipVolumeDockable",
                  "officeRentalCost", "operationID", "stationTypeID", "corporationID", "solarSystemID",
                  "constellationID", "regionID", "stationName", x, y, z,
                  "reprocessingEfficiency", "reprocessingStationsTake", "reprocessingHangarFlag")
           VALUES ($1, $2, 0, 50000000, 10000, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, 0.05, 4)"#,
        &[&id, &((security * 1000.0).round() as i32), &operation, &(station_type as i32), &(corp as i32),
          &(system as i32), &constellation, &region, &name, &pos.x, &pos.y, &pos.z, &reprocessing],
    ).await?;
    txn.tx().execute(
        r#"INSERT INTO "mapDenormalize" ("itemID", "typeID", "groupID", "solarSystemID", "constellationID",
                  "regionID", "orbitID", x, y, z, radius, "itemName", security)
           VALUES ($1, $2, $3, $4, $5, $6, NULL, $7, $8, $9, $10, $11, $12)"#,
        &[&id, &(station_type as i32), &(GROUP_STATION as i32), &(system as i32), &constellation, &region,
          &pos.x, &pos.y, &pos.z, &radius, &name, &security],
    ).await?;
    // The station is an item too, as NPC stations are: its hangar's container.
    txn.tx().execute(
        r#"INSERT INTO "entity" ("itemID", "itemName", "typeID", "ownerID", "locationID", flag, contraband, singleton, quantity, x, y, z)
           VALUES ($1, $2, $3, $4, $5, 0, 0, 1, 1, $6, $7, $8)
           ON CONFLICT ("itemID") DO NOTHING"#,
        &[&id, &name, &(station_type as i32), &(corp as i32), &(system as i32), &pos.x, &pos.y, &pos.z],
    ).await?;
    // The client knows stations by their static locations: the outpost's own.
    txn.tx().execute(
        r#"UPDATE "eveStaticLocations" SET "locationName" = $2, x = $3, y = $4, z = $5 WHERE "locationID" = $1"#,
        &[&id, &name, &pos.x, &pos.y, &pos.z],
    ).await?;
    txn.tx().execute(
        r#"INSERT INTO "eveStaticLocations" ("locationID", "locationName", x, y, z)
           SELECT $1::int, $2::text, $3::float8, $4::float8, $5::float8
            WHERE NOT EXISTS (SELECT 1 FROM "eveStaticLocations" WHERE "locationID" = $1::int)"#,
        &[&id, &name, &pos.x, &pos.y, &pos.z],
    ).await?;
    for &(line_type, count) in assembly_lines(station_type) {
        txn.tx().execute(
            r#"INSERT INTO "ramAssemblyLines" ("assemblyLineID", "assemblyLineTypeID", "containerID", "nextFreeTime",
                      "UIGroupingID", "costInstall", "costPerHour", "restrictionMask", "discountPerGoodStandingPoint",
                      "surchargePerBadStandingPoint", "minimumStanding", "minimumCharSecurity", "minimumCorpSecurity",
                      "maximumCharSecurity", "maximumCorpSecurity", "ownerID", "activityID")
               SELECT (SELECT COALESCE(max("assemblyLineID"), 0) FROM "ramAssemblyLines") + g, $1::int, $2::int,
                      TIMESTAMP '1900-01-01 00:00:00', 0, 1000, 333, 0, 1.5, 0.5, -10, -10, -10, 10, 10, $3::int, t."activityID"
                 FROM generate_series(1, $4::int) g
                 JOIN "ramAssemblyLineTypes" t ON t."assemblyLineTypeID" = $1::int"#,
            &[&line_type, &id, &(corp as i32), &count],
        ).await?;
        txn.tx().execute(
            r#"INSERT INTO "ramAssemblyLineStations" ("stationID", "assemblyLineTypeID", quantity, "stationTypeID",
                      "ownerID", "solarSystemID", "regionID")
               VALUES ($1, $2, $3, $4, $5, $6, $7)"#,
            &[&id, &line_type, &count, &(station_type as i32), &(corp as i32), &(system as i32), &region],
        ).await?;
    }
    // Whatever else was in the hold waits in the new station's hangar.
    Ok(Ok(Outpost { id: i64::from(id), type_id: station_type, name, pos, radius, corp, undock }))
}

/// Outpost types by race (Caldari, Minmatar, Amarr, Gallente).
const OUTPOSTS: [(i64, i64); 4] = [(1, 21642), (2, 21646), (4, 21644), (8, 21645)];
const GROUP_PLANET: i32 = 7;

/// A bot alliance raises an outpost for `corp` in `system` (its race's
/// kind, beside a planet), as a platform would build one. Its ID and name;
/// None if the system already has an outpost or nowhere to put one.
pub async fn found_for_bots(client: &mut tokio_postgres::Client, world: &World, system: i64, corp: i64, race: i64) -> Result<Option<(i64, String)>, tokio_postgres::Error> {
    let station_type = OUTPOSTS.iter().find(|(r, _)| *r == race).map_or(21644, |(_, t)| *t);
    let mut txn = ItemTxn::begin(client).await?;
    txn.tx().query_one("SELECT pg_advisory_xact_lock($1, $2)", &[&307i32, &0i32]).await?;
    let taken = txn.tx().query_opt(
        r#"SELECT 1 FROM "staStations" WHERE "solarSystemID" = $1 AND "stationID" BETWEEN $2 AND $3 LIMIT 1"#,
        &[&(system as i32), &OUTPOST_ID_MIN, &OUTPOST_ID_MAX],
    ).await?;
    if taken.is_some() {
        return Ok(None);
    }
    let Some(planet) = txn.tx().query_opt(
        r#"SELECT "x", "y", "z", COALESCE("radius", 5000000) FROM "mapDenormalize"
            WHERE "solarSystemID" = $1 AND "groupID" = $2 ORDER BY "itemID" LIMIT 1"#,
        &[&(system as i32), &GROUP_PLANET],
    ).await? else {
        return Ok(None);
    };
    let reach = float(&planet, 3).unwrap_or(5_000_000.0) + 150_000.0;
    let dir = Vec3::new(1.0, 0.2, 0.5).unit();
    let pos = Vec3::new(float(&planet, 0).unwrap_or(0.0), float(&planet, 1).unwrap_or(0.0), float(&planet, 2).unwrap_or(0.0)) + dir * reach;
    let corp_name = txn.tx()
        .query_opt(r#"SELECT COALESCE("corporationName", '') FROM "corporation" WHERE "corporationID" = $1"#, &[&(corp as i32)])
        .await?
        .and_then(|r| text(&r, 0))
        .unwrap_or_default();
    let outpost = match raise(&mut txn, system, corp, &corp_name, station_type, pos).await? {
        Ok(o) => o,
        Err(m) => {
            info!("bots: no outpost in {system}: {m}");
            return Ok(None);
        }
    };
    txn.commit().await?;
    let found = (outpost.id, outpost.name.clone());
    info!("outpost {} ({}) founded by bots in {system} for corporation {corp}", found.1, found.0);
    show_new(world, system, None, outpost);
    Ok(Some(found))
}

/// The platform goes and the outpost appears, for everyone in the system.
fn show(world: &World, system: i64, platform: i64, o: Outpost) {
    show_new(world, system, Some(platform), o);
}

/// An outpost appears (from its platform, if it had one) for everyone in the system.
fn show_new(world: &World, system: i64, platform: Option<i64>, o: Outpost) {
    let mut world = world.lock();
    let Some(park) = world.parks.get_mut(&system) else { return };
    if let Some(platform) = platform {
        park.structures.remove(&platform);
        for seen in park.visible_objects.values_mut() {
            seen.remove(&platform);
        }
    }
    park.celestials.push(Celestial {
        id: o.id,
        type_id: o.type_id,
        group_id: GROUP_STATION,
        name: o.name,
        pos: o.pos,
        radius: o.radius,
        undock: o.undock,
        jump: None,
    });
    let mut entries: Vec<Value> = platform.map(|p| space::remove_ball_entries(park.tick, p)).unwrap_or_default();
    entries.extend(space::celestial_add_entry(park, o.id, o.corp));
    space::broadcast(&world, system, None, entries);
}

/// /outpostmats [platformID]: puts into the platform's hold (the given one,
/// else the nearest in the system) whatever it still lacks to build.
pub async fn spawn_materials(ctx: &Ctx<'_>, platform: Option<i64>) -> Result<String, String> {
    let (s, _, _) = platform_here(ctx, platform)?;
    let me = ctx.session.char_id.ok_or("Select a character first.")?;
    let mut client = crate::db::connect(ctx.db_url).await.map_err(|e| e.to_string())?;
    let mut txn = ItemTxn::begin(&mut client).await.map_err(|e| e.to_string())?;
    let needed = requirements(txn.tx(), s.type_id).await.map_err(|e| e.to_string())?;
    if needed.is_empty() {
        return Err(format!("{} needs no materials on this server.", s.name));
    }
    let held = contents(txn.tx(), s.id).await.map_err(|e| e.to_string())?;
    let mut added = Vec::new();
    for (type_id, quantity, name) in &needed {
        let have: i64 = held.iter().filter(|(_, t, _)| t == type_id).map(|(_, _, q)| q).sum();
        if have >= *quantity {
            continue;
        }
        // Owned by the caller, as items dragged into the hold are, so the open
        // Access Resources window shows them at once.
        txn.create(NewItem { type_id: *type_id, owner_id: me, location_id: s.id, flag: 0, quantity: quantity - have, singleton: false })
            .await
            .map_err(|e| e.to_string())?;
        added.push(format!("{} x {name}", quantity - have));
    }
    let events = txn.commit().await.map_err(|e| e.to_string())?;
    crate::items::notify(ctx.world, ctx.client_id, events);
    Ok(if added.is_empty() {
        format!("{} already holds everything it needs. Press Build in its hold once it has anchored.", s.name)
    } else {
        format!("Added to {} ({}): {}. Press Build in its hold once it has anchored.", s.name, s.id, added.join(", "))
    })
}

// ---- hangars --------------------------------------------------------------------------

/// Each outpost docks into its race's hangar, as that race's NPC stations do
/// (Caldari 241, Minmatar 242, Amarr 1075, Gallente 2067); the data gives all
/// four the one shared hangar.
/// The static data's own positions for its places in the outpost ID range
/// (locationID, x, y, z).
const ORIGINAL_POSITIONS: &str = "(61000001, -553052037120, 73071943680, 102584770560), (61000002, 2645580718080, -34165678080, -5771872542720), (61000003, 863843328000, 18544926720, 512362536960), (61000004, -4841876152320, -932665958400, -5842790522880), (61000005, 571594383360, -16523059200, 981875220480), (61000006, -2736985251840, 383628288000, -6037808701440), (61000007, -950028165120, -103419371520, 568085176320), (61000008, -1645208248320, 213354946560, -1200596213760), (61000009, 170776043520, -27318067200, 53984747520), (61000010, -324399882240, 12049981440, 584501944320), (61000011, -69260451840, 6106767360, -89242951680), (61000012, 32670474240, 566599680, 388248821760), (61000013, -262934814720, 5047050240, 39242342400), (61000014, 462903582720, 71146659840, 97839636480), (61000015, 574267760640, -19491594240, 397844275200), (61000016, 10114867200, 905011200, -66563481600), (61000017, -3915819786240, 265022791680, 619365212160), (61000018, -1719708426240, -23281213440, -1817100656640), (61000019, -47798231040, 1021747200, 465177600000), (61000020, -4781606952960, -453164359680, 5108659937280), (61000021, -305709342720, -16689684480, -1008518799360), (61000022, 407936409600, 3337297920, 135142072320), (61000023, -41039831040, -3629506560, -137668239360), (61000024, 394080460800, -8727797760, 1307712675840), (61000025, -536758394880, -7605166080, -477225492480), (61000026, 113886289920, 10016808960, -129645404160), (61000027, 1737443205120, 136211128320, 1515162378240), (61000028, -123909857280, -22810828800, -404207493120), (61000029, -124699238400, -21333565440, -161818091520), (61000030, 355011256320, -63994306560, 326110863360), (61000031, 26526228480, -2966937600, 94479605760), (61000032, -1673405276160, -134289039360, -2587709153280), (61000033, -826095083520, 143065866240, 4679996006400), (61000034, -229690368000, 5815296000, 1767506780160), (61000035, 229027799040, 40059248640, -179874078720), (61000036, -5337376972800, 135897538560, 1086049689600), (61000037, 791041597440, -1703731200, 69282570240), (61000038, 4026323312640, 626239856640, 3224493219840), (61000039, -256215736320, -4672020480, -167808737280), (61000040, -2288445726720, -431963627520, 1261072097280), (61000041, -79091589120, -11142144000, -962085888000)";

pub async fn ensure_hangars(db: &tokio_postgres::Client) -> Result<(), tokio_postgres::Error> {
    // Every station is a static location the client is sent at login. The
    // static data also has named places in the outpost ID range (61000000
    // on), and outposts used to be given their IDs; such an outpost showed
    // the place's name. It keeps the place's position, as it always had (the
    // client builds the station's surroundings from it), and takes its own
    // name. A station missing its static location gets one from itself.
    let positions = db
        .execute(
            &format!(
                r#"UPDATE "eveStaticLocations" l SET x = v.x, y = v.y, z = v.z
                     FROM (VALUES {ORIGINAL_POSITIONS}) AS v(id, x, y, z)
                    WHERE l."locationID" = v.id AND (l.x <> v.x OR l.y <> v.y OR l.z <> v.z)"#
            ),
            &[],
        )
        .await?;
    let renamed = db
        .execute(
            r#"UPDATE "eveStaticLocations" l SET "locationName" = COALESCE(s."stationName", '')
                 FROM "staStations" s
                WHERE s."stationID" = l."locationID" AND s."stationID" >= 61000000
                  AND l."locationName" IS DISTINCT FROM COALESCE(s."stationName", '')"#,
            &[],
        )
        .await?;
    let restored = db
        .execute(
            r#"INSERT INTO "eveStaticLocations" ("locationID", "locationName", x, y, z)
               SELECT s."stationID", COALESCE(s."stationName", ''), COALESCE(s.x, 0), COALESCE(s.y, 0), COALESCE(s.z, 0)
                 FROM "staStations" s
                WHERE NOT EXISTS (SELECT 1 FROM "eveStaticLocations" l WHERE l."locationID" = s."stationID")"#,
            &[],
        )
        .await?;
    // Every station is also an item, which its hangar names as its
    // container; the client reads the hangar's type from it to move items in.
    let items = db
        .execute(
            r#"INSERT INTO "entity" ("itemID", "itemName", "typeID", "ownerID", "locationID", flag, contraband, singleton, quantity, x, y, z)
               SELECT s."stationID", COALESCE(s."stationName", ''), s."stationTypeID", s."corporationID", s."solarSystemID",
                      0, 0, 1, 1, COALESCE(s.x, 0), COALESCE(s.y, 0), COALESCE(s.z, 0)
                 FROM "staStations" s
                WHERE NOT EXISTS (SELECT 1 FROM "entity" e WHERE e."itemID" = s."stationID")"#,
            &[],
        )
        .await?;
    if positions + renamed + restored + items > 0 {
        tracing::info!("stations: {restored} static location(s) restored, {renamed} outpost(s) renamed, {positions} position(s) put back, {items} station item(s) added");
    }
    // Station Management settings.
    db.batch_execute(r#"
        CREATE TABLE IF NOT EXISTS "rmrStationAccessRules" (
            "stationID" integer NOT NULL, "serviceID" integer NOT NULL,
            "minimumStanding" double precision NOT NULL, "minimumCharSecurity" double precision NOT NULL,
            "maximumCharSecurity" double precision NOT NULL, "minimumCorpSecurity" double precision NOT NULL,
            "maximumCorpSecurity" double precision NOT NULL,
            PRIMARY KEY ("stationID", "serviceID"));
        CREATE TABLE IF NOT EXISTS "rmrStationCostModifiers" (
            "stationID" integer NOT NULL, "serviceID" integer NOT NULL,
            "discountPerGoodStandingPoint" double precision NOT NULL,
            "surchargePerBadStandingPoint" double precision NOT NULL,
            PRIMARY KEY ("stationID", "serviceID"));
        CREATE TABLE IF NOT EXISTS "rmrStationDescriptions" ("stationID" integer PRIMARY KEY, description text NOT NULL);
        CREATE TABLE IF NOT EXISTS "rmrOfficePublic" ("officeID" integer PRIMARY KEY, "publiclyAvailable" boolean NOT NULL);
    "#).await?;
    // Outposts come with the CONCORD hangar (graphic 2067), a placeholder in
    // this client: see-through floor, ships that will not load. Each race's
    // outpost gets its race's station hangar instead (Gallente: 244).
    db.execute(
        r#"UPDATE "staStationTypes" st
              SET "hangarGraphicID" = CASE t."raceID" WHEN 1 THEN 241 WHEN 2 THEN 242 WHEN 4 THEN 1075 ELSE 244 END
             FROM "invTypes" t
            WHERE t."typeID" = st."stationTypeID" AND st."stationTypeID" IN (21642, 21644, 21645, 21646)"#,
        &[],
    ).await?;
    Ok(())
}

// ---- station management -----------------------------------------------------------------

/// const.typeOfficeFolder: the rentable item the Offices tab lists.
const TYPE_OFFICE_FOLDER: i64 = 26;
/// const.corpRoleStationManager.
const CORP_ROLE_STATION_MANAGER: i64 = 2048;
/// const.stationServiceDocking.
pub const SERVICE_DOCKING: i64 = 2_097_152;
const OPEN_RULE: [f64; 5] = [-10.0, -10.0, 10.0, -10.0, 10.0];

/// [minimumStanding, minimumCharSecurity, maximumCharSecurity,
/// minimumCorpSecurity, maximumCorpSecurity] for a station service.
pub async fn access_rule(db: &tokio_postgres::Client, station: i64, service: i64) -> [f64; 5] {
    let row = db.query_opt(
        r#"SELECT "minimumStanding", "minimumCharSecurity", "maximumCharSecurity",
                  "minimumCorpSecurity", "maximumCorpSecurity"
             FROM "rmrStationAccessRules" WHERE "stationID" = $1 AND "serviceID" = $2"#,
        &[&(station as i32), &(service as i32)],
    ).await;
    match row {
        Ok(Some(r)) => [0, 1, 2, 3, 4].map(|i| r.try_get::<_, f64>(i).unwrap_or(OPEN_RULE[i])),
        Ok(None) => OPEN_RULE,
        Err(e) => { error!("station access rule {station}/{service}: {}", crate::logging::chain(&e)); OPEN_RULE }
    }
}

/// Whether a pilot may dock: members of the owning corporation always may;
/// anyone else must meet the station's Docking rule (the owner's standing
/// toward them and their security status). None: allowed; Some: why not.
pub async fn docking_refusal(db: &tokio_postgres::Client, station: i64, char_id: i64) -> Option<String> {
    let rule = access_rule(db, station, SERVICE_DOCKING).await;
    if rule == OPEN_RULE {
        return None;
    }
    let row = db.query_opt(
        r#"SELECT st."corporationID", c."corporationID", COALESCE(c."securityRating", 0)::float8,
                  NULLIF((SELECT "allianceID" FROM "corporation" WHERE "corporationID" = st."corporationID"), 0),
                  NULLIF((SELECT "allianceID" FROM "corporation" WHERE "corporationID" = c."corporationID"), 0)
             FROM "staStations" st, "character_" c
            WHERE st."stationID" = $1 AND c."characterID" = $2"#,
        &[&(station as i32), &(char_id as i32)],
    ).await.ok().flatten()?;
    let (owner, corp, security) = (int(&row, 0).unwrap_or(0), int(&row, 1).unwrap_or(0), float(&row, 2).unwrap_or(0.0));
    let (owner_alliance, alliance) = (int(&row, 3), int(&row, 4));
    if owner == corp || (owner_alliance.is_some() && owner_alliance == alliance) {
        return None;
    }
    let from: Vec<i32> = [Some(owner), owner_alliance].into_iter().flatten().map(|v| v as i32).collect();
    let to: Vec<i32> = [Some(char_id), Some(corp), alliance].into_iter().flatten().map(|v| v as i32).collect();
    let standing = db.query_opt(
        r#"SELECT max("standing")::float8 FROM "ownerStandings" WHERE "fromID" = ANY($1) AND "toID" = ANY($2)"#,
        &[&from, &to],
    ).await.ok().flatten().and_then(|r| r.try_get::<_, Option<f64>>(0).ok().flatten()).unwrap_or(0.0);
    if standing < rule[0] {
        return Some(format!("The station owner requires a standing of at least {:.1} to dock here.", rule[0]));
    }
    if security < rule[1] || security > rule[2] {
        return Some(format!("This station only admits pilots with a security status between {:.1} and {:.1}.", rule[1], rule[2]));
    }
    None
}

/// Station Management changes need a director or station manager of the
/// corporation that owns the station.
async fn may_manage(ctx: &Ctx<'_>, station: i64) -> Result<(), String> {
    let owner = ctx.query_one(r#"SELECT "corporationID" FROM "staStations" WHERE "stationID" = $1"#, &[&(station as i32)]).await
        .and_then(|r| int(&r, 0));
    if owner.is_none() || owner != ctx.session.corp_id {
        return Err("Your corporation does not own this station.".into());
    }
    if !ctx.session.has_corp_role(crate::session::CORP_ROLE_DIRECTOR) && !ctx.session.has_corp_role(CORP_ROLE_STATION_MANAGER) {
        return Err("You need the Station Manager role to change this station's settings.".into());
    }
    Ok(())
}

/// A field of a util.KeyVal (or dict) the client sent back.
fn field<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
    let state = match v {
        Value::Instance { state, .. } => state.as_ref(),
        other => other,
    };
    let state = match state {
        Value::Tuple(t) => t.first().unwrap_or(state),
        other => other,
    };
    state.get_str(key).or_else(|| state.get(&Value::Unicode(key.to_string())))
}

/// A number the window sent: an int, a float or the text of an edit box.
fn number(v: Option<&Value>) -> Option<f64> {
    let v = v?;
    v.as_f64().or_else(|| v.as_string().and_then(|s| s.trim().replace(',', "").parse().ok())).filter(|n: &f64| n.is_finite())
}

/// UpdateStationManagementSettings(accessRulesByServiceID, costModifiers,
/// rentableItems, stationName, description, dockingCostPerVolume,
/// factorySlotRentalCost, officeRentalCost, researchFacilitySlotRentalCost,
/// reprocessingStationsTake, reprocessingHangarFlag).
async fn update_settings(ctx: &Ctx<'_>, station: i64, args: &[Value]) -> Result<(), String> {
    may_manage(ctx, station).await?;
    let st = station as i32;
    let failed = || "The station settings could not be saved.".to_string();

    if let Some(Value::Dict(rules)) = args.first() {
        for (service, rule) in rules {
            let Some(service) = service.as_i64() else { continue };
            let mut values = OPEN_RULE;
            for (i, key) in ["minimumStanding", "minimumCharSecurity", "maximumCharSecurity", "minimumCorpSecurity", "maximumCorpSecurity"].iter().enumerate() {
                if let Some(n) = number(field(rule, key)) { values[i] = n.clamp(-10.0, 10.0); }
            }
            if !ctx.execute(
                r#"INSERT INTO "rmrStationAccessRules" ("stationID", "serviceID", "minimumStanding", "minimumCharSecurity",
                          "maximumCharSecurity", "minimumCorpSecurity", "maximumCorpSecurity")
                   VALUES ($1, $2, $3, $4, $5, $6, $7)
                   ON CONFLICT ("stationID", "serviceID") DO UPDATE SET
                          "minimumStanding" = EXCLUDED."minimumStanding", "minimumCharSecurity" = EXCLUDED."minimumCharSecurity",
                          "maximumCharSecurity" = EXCLUDED."maximumCharSecurity", "minimumCorpSecurity" = EXCLUDED."minimumCorpSecurity",
                          "maximumCorpSecurity" = EXCLUDED."maximumCorpSecurity""#,
                &[&st, &(service as i32), &values[0], &values[1], &values[2], &values[3], &values[4]],
            ).await { return Err(failed()); }
        }
    }
    for modifier in args.get(1).and_then(|v| v.seq()).unwrap_or(&[]) {
        let Some(service) = field(modifier, "serviceID").and_then(|v| v.as_i64()) else { continue };
        let discount = number(field(modifier, "discountPerGoodStandingPoint")).unwrap_or(0.0).clamp(0.0, 10.0);
        let surcharge = number(field(modifier, "surchargePerBadStandingPoint")).unwrap_or(0.0).clamp(0.0, 10.0);
        if !ctx.execute(
            r#"INSERT INTO "rmrStationCostModifiers" ("stationID", "serviceID", "discountPerGoodStandingPoint", "surchargePerBadStandingPoint")
               VALUES ($1, $2, $3, $4)
               ON CONFLICT ("stationID", "serviceID") DO UPDATE SET
                      "discountPerGoodStandingPoint" = EXCLUDED."discountPerGoodStandingPoint",
                      "surchargePerBadStandingPoint" = EXCLUDED."surchargePerBadStandingPoint""#,
            &[&st, &(service as i32), &discount, &surcharge],
        ).await { return Err(failed()); }
    }
    for item in args.get(2).and_then(|v| v.seq()).unwrap_or(&[]) {
        let Some(office) = field(item, "officeID").and_then(|v| v.as_i64()) else { continue };
        let public = field(item, "publiclyAvailable").and_then(|v| v.as_i64()).unwrap_or(0) != 0;
        if !ctx.execute(
            r#"INSERT INTO "rmrOfficePublic" ("officeID", "publiclyAvailable") VALUES ($1, $2)
               ON CONFLICT ("officeID") DO UPDATE SET "publiclyAvailable" = EXCLUDED."publiclyAvailable""#,
            &[&(office as i32), &public],
        ).await { return Err(failed()); }
    }

    let name = args.get(3).and_then(|v| v.as_string()).map(|s| s.trim().to_string()).unwrap_or_default();
    if name.is_empty() || name.chars().count() > 100 {
        return Err("The station name must be between 1 and 100 characters.".into());
    }
    let description: String = args.get(4).and_then(|v| v.as_string()).unwrap_or_default().chars().take(5000).collect();
    let docking = number(args.get(5)).map(|n| n.clamp(0.0, 1000.0).round() as i32);
    let office_cost = number(args.get(7)).map(|n| n.clamp(0.0, i32::MAX as f64).round() as i32);
    let take = number(args.get(9)).map(|n| n.clamp(0.0, 1.0));
    let hangar = args.get(10).and_then(|v| v.as_i64()).map(|n| n as i32);
    if !ctx.execute(
        r#"UPDATE "staStations" SET "stationName" = $2,
                  "dockingCostPerVolume" = COALESCE($3, "dockingCostPerVolume"),
                  "officeRentalCost" = COALESCE($4, "officeRentalCost"),
                  "reprocessingStationsTake" = COALESCE($5, "reprocessingStationsTake"),
                  "reprocessingHangarFlag" = COALESCE($6, "reprocessingHangarFlag")
            WHERE "stationID" = $1"#,
        &[&st, &name, &docking, &office_cost, &take, &hangar],
    ).await { return Err(failed()); }
    ctx.execute(r#"UPDATE "mapDenormalize" SET "itemName" = $2 WHERE "itemID" = $1"#, &[&st, &name]).await;
    ctx.execute(r#"UPDATE "eveStaticLocations" SET "locationName" = $2 WHERE "locationID" = $1"#, &[&st, &name]).await;
    ctx.execute(r#"UPDATE entity SET "itemName" = $2 WHERE "itemID" = $1"#, &[&st, &name]).await;
    if !ctx.execute(
        r#"INSERT INTO "rmrStationDescriptions" ("stationID", description) VALUES ($1, $2)
           ON CONFLICT ("stationID") DO UPDATE SET description = EXCLUDED.description"#,
        &[&st, &description],
    ).await { return Err(failed()); }
    info!(station, name = %name, ?docking, ?office_cost, ?take, ?hangar, "station management settings saved");
    Ok(())
}

const SERVICE_HEADER: &[&str] = &["serviceID", "serviceName", "description"];

/// The services a station offers: those of its operation.
const STATION_SERVICES: &str = r#"SELECT s."serviceID", s."serviceName", COALESCE(s."description", '')
      FROM "staServices" s
     WHERE s."serviceID" IN (SELECT os."serviceID" FROM "staOperationServices" os
                               JOIN "staStations" st ON st."operationID" = os."operationID"
                              WHERE st."stationID" = $1)
     ORDER BY s."serviceID""#;

/// The Station Management window's calls on corpStationMgr (bound to a
/// station). None for any other method.
pub async fn station_management(ctx: &Ctx<'_>, method: &str, args: &[Value], station: i64) -> Option<Outcome> {
    let station_param = station as i32;
    let services = || async {
        ctx.query(STATION_SERVICES, &[&station_param]).await.iter().filter_map(|r| int(r, 0)).collect::<Vec<i64>>()
    };
    let value = match method {
        "GetStationServiceIdentifiers" => ctx.rowset(SERVICE_HEADER, STATION_SERVICES, &[&station_param]).await,
        "GetStationDetails" => {
            let row = ctx.query_one(
                r#"SELECT "stationID", COALESCE("stationName", ''), "stationTypeID", "corporationID", "solarSystemID",
                          COALESCE("security", 0), COALESCE("dockingCostPerVolume", 0), COALESCE("maxShipVolumeDockable", 0),
                          COALESCE("officeRentalCost", 0), COALESCE("reprocessingStationsTake", 0.05)::float8,
                          COALESCE("reprocessingHangarFlag", 4), x, y, z, "operationID"
                     FROM "staStations" WHERE "stationID" = $1"#,
                &[&station_param],
            ).await?;
            let i = |n| Value::Int(int(&row, n).unwrap_or(0));
            let f = |n| Value::Double(float(&row, n).unwrap_or(0.0));
            crate::svc::rows::key_val(vec![
                ("stationID", i(0)),
                ("stationName", Value::text(text(&row, 1).unwrap_or_default())),
                ("stationTypeID", i(2)),
                ("ownerID", i(3)),
                ("corporationID", i(3)),
                ("standingOwnerID", i(3)),
                ("solarSystemID", i(4)),
                ("security", Value::Double(float(&row, 5).unwrap_or(0.0) / 1000.0)),
                ("dockingCostPerVolume", f(6)),
                ("maxShipVolumeDockable", f(7)),
                ("officeRentalCost", f(8)),
                ("reprocessingStationsTake", f(9)),
                ("reprocessingHangarFlag", i(10)),
                ("x", f(11)),
                ("y", f(12)),
                ("z", f(13)),
                ("operationID", i(14)),
                ("description", Value::text(
                    ctx.query_one(r#"SELECT description FROM "rmrStationDescriptions" WHERE "stationID" = $1"#, &[&station_param]).await
                        .and_then(|r| text(&r, 0)).unwrap_or_default())),
                ("exitTime", Value::None),
                ("upgradeLevel", Value::Int(0)),
                ("orbitID", Value::None),
            ])
        }
        // {serviceID: state}: every service the station offers, enabled.
        "GetStationServiceStates" => {
            let owner = ctx.query_one(r#"SELECT "corporationID", "solarSystemID" FROM "staStations" WHERE "stationID" = $1"#, &[&station_param]).await?;
            let (owner, system) = (int(&owner, 0).unwrap_or(0), int(&owner, 1).unwrap_or(0));
            Value::Dict(services().await.into_iter().map(|service| (Value::Int(service), crate::svc::rows::key_val(vec![
                ("solarSystemID", Value::Int(system)),
                ("stationID", Value::Int(station)),
                ("serviceID", Value::Int(service)),
                ("stationOwnerID", Value::Int(owner)),
                ("isEnabled", Value::Int(1)),
            ]))).collect())
        }
        // (stationID, serviceID): who may use the service. Anyone, until
        // the owner sets a rule in Station Management.
        "GetStationServiceAccessRule" => {
            let service = args.get(1).and_then(|v| v.as_i64()).unwrap_or(0);
            let rule = access_rule(ctx.db, station, service).await;
            crate::svc::rows::key_val(vec![
                ("stationID", Value::Int(station)),
                ("serviceID", Value::Int(service)),
                ("minimumStanding", Value::Double(rule[0])),
                ("minimumCharSecurity", Value::Double(rule[1])),
                ("maximumCharSecurity", Value::Double(rule[2])),
                ("minimumCorpSecurity", Value::Double(rule[3])),
                ("maximumCorpSecurity", Value::Double(rule[4])),
            ])
        }
        "GetStationManagementServiceCostModifiers" => {
            let saved: Vec<(i64, f64, f64)> = ctx.query(
                r#"SELECT "serviceID", "discountPerGoodStandingPoint", "surchargePerBadStandingPoint"
                     FROM "rmrStationCostModifiers" WHERE "stationID" = $1"#, &[&station_param]).await
                .iter().map(|r| (int(r, 0).unwrap_or(0), float(r, 1).unwrap_or(0.0), float(r, 2).unwrap_or(0.0))).collect();
            Value::List(services().await.into_iter().map(|service| {
                let (_, discount, surcharge) = saved.iter().copied().find(|s| s.0 == service).unwrap_or((service, 0.0, 0.0));
                crate::svc::rows::key_val(vec![
                    ("serviceID", Value::Int(service)),
                    ("discountPerGoodStandingPoint", Value::Double(discount)),
                    ("surchargePerBadStandingPoint", Value::Double(surcharge)),
                ])
            }).collect())
        }
        // The station's offices (typeOfficeFolder rows), numbered 1.. in
        // the order they were rented.
        "GetRentableItems" => {
            let rows = ctx.query(
                r#"SELECT o."itemID", o."corporationID", COALESCE(p."publiclyAvailable", false),
                          COALESCE(s."officeRentalCost", 0)
                     FROM "crpOffices" o JOIN "staStations" s ON s."stationID" = o."stationID"
                     LEFT JOIN "rmrOfficePublic" p ON p."officeID" = o."itemID"
                    WHERE o."stationID" = $1 ORDER BY o."itemID""#,
                &[&station_param],
            ).await;
            let now = crate::bulkdata::filetime_now();
            Value::List(rows.iter().enumerate().map(|(n, r)| crate::svc::rows::key_val(vec![
                ("stationID", Value::Int(station)),
                ("officeID", Value::Int(int(r, 0).unwrap_or(0))),
                ("number", Value::Int(n as i64 + 1)),
                ("typeID", Value::Int(TYPE_OFFICE_FOLDER)),
                ("rentedToID", Value::Int(int(r, 1).unwrap_or(0))),
                ("publiclyAvailable", Value::Int(r.try_get::<_, bool>(2).unwrap_or(false) as i64)),
                ("startDate", Value::Long(now)),
                ("rentPeriodInDays", Value::Int(30)),
                ("periodCost", Value::Double(float(r, 3).unwrap_or(0.0))),
                ("balanceDueDate", Value::Long(now + 30 * 24 * 3600 * 10_000_000)),
            ])).collect())
        }
        // -1: [corporationID] of every corporation with members' clones
        // here; a corporation: [characterID] of its members' clones here.
        "GetOwnerIDsOfClonesAtStation" => {
            let corp = args.first().and_then(|v| v.as_i64()).unwrap_or(-1);
            let rows = ctx.query(
                r#"SELECT DISTINCT c."corporationID", c."characterID"
                     FROM entity e JOIN "invTypes" t ON t."typeID" = e."typeID"
                     JOIN "character_" c ON c."characterID" = e."ownerID"
                    WHERE t."groupID" = $1 AND trim(e."customInfo") = $2
                      AND ($3 = -1 OR c."corporationID" = $3)"#,
                &[&crate::svc::medical::GROUP_CLONE, &station.to_string(), &(corp as i32)],
            ).await;
            let mut seen = Vec::new();
            Value::List(rows.iter().filter_map(|r| {
                let (corp_id, char_id) = (int(r, 0).unwrap_or(0), int(r, 1).unwrap_or(0));
                if corp == -1 {
                    if seen.contains(&corp_id) { return None; }
                    seen.push(corp_id);
                    Some(crate::svc::rows::key_val(vec![("corporationID", Value::Int(corp_id))]))
                } else {
                    Some(crate::svc::rows::key_val(vec![("characterID", Value::Int(char_id)), ("corporationID", Value::Int(corp_id))]))
                }
            }).collect())
        }
        // (corporationID, characterID or -1): those clones lose this station
        // as their home; they wake at the corporation's headquarters instead.
        "RevokeCloneContractsAtStation" => {
            if let Err(why) = may_manage(ctx, station).await { return Some(Outcome::custom(why)); }
            let corp = args.first().and_then(|v| v.as_i64()).unwrap_or(0);
            let pilot = args.get(1).and_then(|v| v.as_i64()).unwrap_or(-1);
            ctx.execute(
                r#"UPDATE entity e SET "customInfo" = NULL
                     FROM "invTypes" t, "character_" c
                    WHERE t."typeID" = e."typeID" AND c."characterID" = e."ownerID"
                      AND t."groupID" = $1 AND trim(e."customInfo") = $2
                      AND c."corporationID" = $3 AND ($4 = -1 OR c."characterID" = $4)"#,
                &[&crate::svc::medical::GROUP_CLONE, &station.to_string(), &(corp as i32), &(pilot as i32)],
            ).await;
            info!(station, corp, pilot, "station management: clone contracts revoked");
            Value::None
        }
        "UpdateStationManagementSettings" => match update_settings(ctx, station, args).await {
            Ok(()) => Value::None,
            Err(why) => return Some(Outcome::custom(why)),
        },
        m if m.starts_with("UpdateStation") || m.starts_with("SetStation") => {
            info!("station management {m} for {station} is not handled: {args:?}");
            return Some(Outcome::custom("That station setting cannot be changed on this server yet."));
        }
        _ => return None,
    };
    Some(Outcome::Value(value))
}
