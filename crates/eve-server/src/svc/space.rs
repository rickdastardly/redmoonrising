//! `beyonce`, the remote ballpark: what the client sees in space and how its
//! ship moves (michelle.py, docs: destiny/ballpark/michelle).
//!
//! Every solar system with a ship in it has a [`Park`] in the shared world. A
//! movement command updates the park and goes out as a destiny opcode to every
//! client in the system, which then runs the same motion locally. A client that
//! (re)builds its ballpark gets a SetState of the whole park.

use eve_marshal::Value;
use std::collections::{BTreeMap, BTreeSet};
use tracing::{debug, warn};

use crate::ballpark::{Celestial, Command, Park, Ship, Vec3};
use crate::destiny::{self, flags, mode, Ball};
use crate::services::{Call, Ctx, Outcome};
use crate::svc::rows::{float, int, key_val, rowset, text};
use crate::world::WorldState;

const TYPE_SOLAR_SYSTEM: i64 = 5;
const OWNER_SYSTEM: i64 = 1;
const GROUP_STARGATE: i64 = 10;
const GROUP_STATION: i64 = 15;
/// Sun, planet, moon, asteroid belt, stargate, station.
const CELESTIAL_GROUPS: [i32; 6] = [6, 7, 8, 9, 10, 15];
/// WarpToStuff without minRange (menusvc's default warp-to distance).
const DEFAULT_WARP_RANGE: f64 = 15_000.0;
/// The menu's "approach" with no range (menusvc.py:1598 `range or 50`).
const DEFAULT_FOLLOW_RANGE: f64 = 50.0;
/// Ship overview visibility is limited to one million metres around each pilot.
const SHIP_OVERVIEW_RANGE: f64 = 1_000_000.0;
/// 14.1 AU in metres. The legacy scanner's input displays kilometres.
const MAX_SCAN_RANGE: f64 = 2_109_329_976_870.0;
/// A zero-range station warp otherwise stops at the exact collision boundary.
/// This keeps the ship clear of the hull before its docking approach.
const STATION_WARP_CLEARANCE: f64 = 2_000.0;

/// `ship`: Undock is served by `conn` (it changes the session); nothing else
/// on the ship object is implemented yet.
/// The `ship` service. Undock and Board change the session, so the connection
/// handles them.
pub async fn ship(call: &Call, ctx: &Ctx<'_>) -> Option<Outcome> {
    match call.method.as_str() {
        "AssembleShip" => {
            let ids = call.args.first().and_then(|v| v.seq()).unwrap_or(&[]);
            let ids = ids.iter().filter_map(|v| v.as_i64()).collect();
            Some(crate::svc::moves::assemble_ship(ctx, ids).await)
        }
        // Drop(itemIDs, ownerID=None, ignoreWarning=False): launch structures or
        // drones, one per ID listed (a stack of five is listed five times).
        "Drop" => {
            let ids = call.args.first().and_then(|v| v.seq()).unwrap_or(&[]);
            let ids: Vec<i64> = ids.iter().filter_map(|v| v.as_i64()).collect();
            let owner = call.args.get(1).and_then(|v| v.as_i64());
            if let Some(outcome) = crate::svc::starbase::launch(ctx, &ids, owner).await {
                return Some(outcome);
            }
            Some(crate::drones::launch(ctx, ids).await)
        }
        // Scoop(itemID, password=None): a structure floating free.
        "Scoop" => crate::svc::starbase::scoop(ctx, call.arg_id(0)).await,
        "ScoopDrone" => {
            let ids = call.args.first().and_then(|v| v.seq()).unwrap_or(&[]);
            let ids: Vec<i64> = ids.iter().filter_map(|v| v.as_i64()).collect();
            Some(crate::drones::scoop(ctx, ids).await)
        }
        _ => None,
    }
}

pub async fn beyonce(call: &Call, ctx: &Ctx<'_>) -> Option<Outcome> {
    let value = match call.method.as_str() {
        // LoadFormations accepts a tuple; no formations exist.
        "GetFormations" => {
            push_set_state(ctx).await;
            Value::Tuple(Vec::new())
        }
        // michelle.RequestReset: the client lost sync and wants the world again.
        "UpdateStateRequest" => {
            push_set_state(ctx).await;
            Value::None
        }
        "GetGangState" => Value::None,
        "GetRelativity" => relativity(ctx, call),
        // The scanner service expects a mapping even when no probes are out.
        "GetScans" => Value::Dict(vec![
            (Value::bytes("probes"), Value::List(Vec::new())),
            (Value::bytes("results"), Value::List(Vec::new())),
        ]),
        "ConeScan" => cone_scan(ctx, call).await,
        method if is_movement(method) => return Some(movement(ctx, call).await),
        _ => return None,
    };
    Some(Outcome::Value(value))
}

/// Directional scanner: scanAngle, range in metres, and the camera's unit
/// vector. Names and distances are supplied even for balls outside the overview.
async fn cone_scan(ctx: &Ctx<'_>, call: &Call) -> Value {
    const HEADER: &[&str] = &["id", "groupID", "typeID", "name", "distance"];
    let Some(system_id) = ctx.session.solar_system_id else { return rowset(HEADER, Vec::new()) };
    let Some(ship_id) = ctx.session.ship_id else { return rowset(HEADER, Vec::new()) };
    let arg = |i: usize| call.args.get(i).and_then(Value::as_f64).unwrap_or(0.0);
    let angle = arg(0);
    let range = arg(1);
    let direction = Vec3::new(arg(2), arg(3), arg(4));
    if !range.is_finite() || range <= 0.0 || !angle.is_finite()
        || !direction.x.is_finite() || !direction.y.is_finite() || !direction.z.is_finite() {
        return rowset(HEADER, Vec::new());
    }

    let hits = {
        let world = ctx.world.lock();
        let Some(park) = world.parks.get(&system_id) else { return rowset(HEADER, Vec::new()) };
        let Some(ego) = park.ships.get(&ship_id) else { return rowset(HEADER, Vec::new()) };
        let heading = direction.unit();
        let half_angle_cos = (angle.clamp(0.0, std::f64::consts::TAU) / 2.0).cos();
        let mut hits = BTreeMap::new();
        let mut consider = |id: i64, type_id: i64, group_id: Option<i64>, name: &str, pos: Vec3| {
            let offset = pos - ego.pos;
            if id != ship_id && within_scan(offset, range, half_angle_cos, heading) {
                hits.entry(id).or_insert_with(|| (type_id, group_id, name.to_owned(), offset.len()));
            }
        };
        for c in &park.celestials { consider(c.id, c.type_id, Some(c.group_id), &c.name, c.pos); }
        for s in park.ships.values().filter(|s| !s.in_warp() && !crate::drones::in_transit(s)) {
            consider(s.id, s.pilot.type_id, Some(s.pilot.group_id), &s.pilot.name, s.pos);
        }
        for r in park.rocks.values() { consider(r.id, r.type_id, None, &r.name, r.pos); }
        for c in park.cans.values() { consider(c.id, c.type_id, None, &c.name, c.pos); }
        for s in park.structures.values() { consider(s.id, s.type_id, Some(s.group_id), &s.name, s.pos); }
        for s in park.sites.values() { consider(s.id, crate::sites::TYPE_BEACON, None, &s.name, s.pos); }
        hits
    };

    let unknown_types: BTreeSet<i32> = hits.values().filter(|(_, group, _, _)| group.is_none())
        .filter_map(|(type_id, _, _, _)| i32::try_from(*type_id).ok()).collect();
    let mut groups = BTreeMap::new();
    if !unknown_types.is_empty() {
        let sql = r#"SELECT "typeID", "groupID" FROM "invTypes" WHERE "typeID" = ANY($1)"#;
        for r in ctx.query(sql, &[&unknown_types.into_iter().collect::<Vec<_>>()]).await {
            if let (Ok(t), Ok(g)) = (r.try_get::<_, i32>(0), r.try_get::<_, i32>(1)) {
                groups.insert(t as i64, g as i64);
            }
        }
    }
    rowset(HEADER, hits.into_iter().filter_map(|(id, (type_id, group_id, name, distance))| {
        let group_id = group_id.or_else(|| groups.get(&type_id).copied())?;
        Some(Value::List(vec![Value::Int(id), Value::Int(group_id), Value::Int(type_id), Value::text(name), Value::Double(distance)]))
    }).collect())
}

fn within_scan(offset: Vec3, range: f64, half_angle_cos: f64, heading: Vec3) -> bool {
    let distance = offset.len();
    if distance > range.min(MAX_SCAN_RANGE) { return false; }
    // The 2006 client sends radians. A full 360 degree scan is a sphere.
    if half_angle_cos <= -1.0 || distance == 0.0 { return true; }
    heading.len_sq() > 0.0 && offset.dot(heading) / distance >= half_angle_cos
}

pub fn is_movement(method: &str) -> bool {
    matches!(
        method,
        "SetSpeedFraction" | "GotoDirection" | "GotoPoint" | "Stop" | "FollowBall" | "Orbit" | "WarpToStuff"
    )
}

// ---- the park ---------------------------------------------------------------

/// Makes sure `solar_system_id` has a park, loading its celestials if not.
pub async fn ensure_park(ctx: &Ctx<'_>, solar_system_id: i64) {
    if ctx.world.lock().parks.contains_key(&solar_system_id) {
        return;
    }
    let celestials = load_celestials(ctx, solar_system_id).await;
    let now = crate::combat::now_ms();
    if let Err(e) = crate::loot::ensure_table(ctx.db).await {
        warn!("no loot table: {}", crate::logging::chain(&e));
    }
    let cans = crate::loot::load_cans(ctx.db, solar_system_id, now).await.unwrap_or_else(|e| {
        warn!("cans in {solar_system_id} not loaded: {}", crate::logging::chain(&e));
        Vec::new()
    });
    let structures = crate::pos::load(ctx.db, solar_system_id).await.unwrap_or_else(|e| {
        warn!("structures in {solar_system_id} not loaded: {}", crate::logging::chain(&e));
        Vec::new()
    });
    let wildlife = match crate::npc::load(ctx.db, solar_system_id).await {
        Ok(w) => w,
        Err(e) => {
            warn!("system {solar_system_id} gets no belt life: {}", crate::logging::chain(&e));
            Default::default()
        }
    };
    let mut world = ctx.world.lock();
    world.parks.entry(solar_system_id).or_insert_with(|| {
        let mut park = Park::new(solar_system_id, celestials, destiny::tick_now());
        park.wildlife = std::sync::Arc::new(wildlife);
        crate::npc::populate(&mut park);
        park.cans = cans.into_iter().map(|c| (c.id, c)).collect();
        park.structures = structures.into_iter().map(|s| (s.id, s)).collect();
        park
    });
}

async fn load_celestials(ctx: &Ctx<'_>, solar_system_id: i64) -> Vec<Celestial> {
    let sql = r#"SELECT d."itemID", d."typeID", d."groupID", COALESCE(d."itemName", ''),
                        d."x", d."y", d."z", COALESCE(d."radius", 0),
                        j."celestialID", dest."solarSystemID",
                        st."dockEntryX", st."dockEntryY", st."dockEntryZ",
                        st."dockOrientationX", st."dockOrientationY", st."dockOrientationZ"
                   FROM "mapDenormalize" d
                   LEFT JOIN "mapJumps" j ON j."stargateID" = d."itemID"
                   LEFT JOIN "mapDenormalize" dest ON dest."itemID" = j."celestialID"
                   LEFT JOIN "staStations" s ON s."stationID" = d."itemID"
                   LEFT JOIN "staStationTypes" st ON st."stationTypeID" = s."stationTypeID"
                  WHERE d."solarSystemID" = $1 AND d."groupID" = ANY($2)
                  ORDER BY d."itemID""#;
    let rows = ctx.query(sql, &[&(solar_system_id as i32), &&CELESTIAL_GROUPS[..]]).await;
    rows.iter()
        .map(|r| Celestial {
            id: int(r, 0).unwrap_or(0),
            type_id: int(r, 1).unwrap_or(0),
            group_id: int(r, 2).unwrap_or(0),
            name: text(r, 3).unwrap_or_default(),
            pos: Vec3::new(float(r, 4).unwrap_or(0.0), float(r, 5).unwrap_or(0.0), float(r, 6).unwrap_or(0.0)),
            radius: float(r, 7).unwrap_or(0.0),
            undock: float(r, 10).map(|x| {
                (Vec3::new(x, float(r, 11).unwrap_or(0.0), float(r, 12).unwrap_or(0.0)),
                 Vec3::new(float(r, 13).unwrap_or(0.0), float(r, 14).unwrap_or(0.0), float(r, 15).unwrap_or(0.0)))
            }),
            jump: match (int(r, 9), int(r, 8)) {
                (Some(system), Some(gate)) => Some((system, gate)),
                _ => None,
            },
        })
        .collect()
}

// ---- what the client is sent --------------------------------------------------

pub fn entry(tick: i64, opcode: &str, args: Vec<Value>) -> Value {
    Value::Tuple(vec![
        Value::Int(destiny::stamp(tick) as i64),
        Value::Tuple(vec![Value::bytes(opcode), Value::Tuple(args)]),
    ])
}

/// DoDestinyUpdate(state, waitForBubble=0) to every client in the system, or
/// only to `only`.
pub fn broadcast(world: &WorldState, solar_system_id: i64, only: Option<i64>, entries: Vec<Value>) {
    if entries.is_empty() {
        return;
    }
    let targets = match only {
        Some(c) => vec![c],
        None => world.in_space(solar_system_id),
    };
    let args = Value::Tuple(vec![Value::List(entries), Value::Int(0)]);
    world.notify(&targets, "DoDestinyUpdate", args);
}

fn ship_ball(ship: &Ship) -> Ball {
    Ball {
        id: ship.id,
        mode: mode::STOP,
        radius: ship.stats.radius,
        pos: ship.pos.tuple(),
        flags: if ship.in_warp() { flags::SHIP_IN_WARP } else { flags::SHIP },
        mass: ship.stats.mass,
        corp_id: ship.pilot.corp_id as i32,
        alliance_id: ship.pilot.alliance_id.unwrap_or(-1) as i32,
        harmonic: ship.harmonic,
        max_velocity: ship.stats.max_velocity,
        velocity: ship.vel.tuple(),
        agility: ship.stats.agility,
        speed_fraction: ship.speed_fraction,
        name: ship.pilot.name.clone(),
    }
}

pub fn slim(fields: Vec<(&str, Value)>) -> Value {
    Value::Instance {
        class: Box::new(Value::bytes("foo.SlimItem")),
        state: Box::new(Value::Dict(fields.into_iter().map(|(k, v)| (Value::bytes(k), v)).collect())),
    }
}

/// groupID/categoryID are left out on purpose: the client derives them from typeID.
fn ship_slim(ship: &Ship) -> Value {
    let p = &ship.pilot;
    // An NPC belongs to its corporation and has no pilot to show.
    if ship.is_npc() {
        return slim(vec![
            ("itemID", Value::Int(ship.id)),
            ("typeID", Value::Int(p.type_id)),
            ("ownerID", Value::Int(p.corp_id)),
            ("corpID", Value::Int(p.corp_id)),
        ]);
    }
    // A drone belongs to its pilot but is not flown by one.
    if ship.drone.is_some() {
        return slim(vec![
            ("itemID", Value::Int(ship.id)),
            ("typeID", Value::Int(p.type_id)),
            ("ownerID", Value::Int(p.char_id)),
            ("corpID", Value::Int(p.corp_id)),
        ]);
    }
    slim(vec![
        ("itemID", Value::Int(ship.id)),
        ("typeID", Value::Int(p.type_id)),
        ("ownerID", Value::Int(p.char_id)),
        ("charID", Value::Int(p.char_id)),
        ("corpID", Value::Int(p.corp_id)),
        ("allianceID", Value::int_or_none(p.alliance_id)),
        // consider.py rounds it; it must be a number.
        ("securityStatus", Value::Double(p.security)),
        ("bounty", Value::Double(p.bounty)),
    ])
}

fn celestial_slim(c: &Celestial, owners: &[(i64, i64)]) -> Value {
    let owner = owners.iter().find(|(id, _)| *id == c.id).map_or(OWNER_SYSTEM, |(_, o)| *o);
    let mut fields = vec![
        ("itemID", Value::Int(c.id)),
        ("typeID", Value::Int(c.type_id)),
        ("ownerID", Value::Int(owner)),
    ];
    if let Some((system, gate)) = c.jump {
        let jump = crate::svc::rows::row(
            &["locationID", "toCelestialID"],
            vec![Value::Int(system), Value::Int(gate)],
        );
        fields.push(("jumps", Value::List(vec![jump])));
    }
    slim(fields)
}

/// [shield (fraction, tau), armor, structure], as the ship is now.
fn health(ship: &Ship) -> Value {
    match &ship.fight {
        Some(f) => crate::fight::damage_state(&f.vitals, crate::combat::now_ms()),
        None => Value::List(vec![
            Value::Tuple(vec![Value::Double(1.0), Value::Double(1.0)]),
            Value::Double(1.0),
            Value::Double(1.0),
        ]),
    }
}

/// The destiny opcode for a ship's current command.
pub fn command_entry(ship: &Ship, tick: i64) -> Value {
    let id = Value::Int(ship.id);
    let point = |p: &Vec3| vec![id.clone(), Value::Double(p.x), Value::Double(p.y), Value::Double(p.z)];
    match &ship.command {
        Command::Stop => entry(tick, "Stop", vec![id]),
        Command::Direction(d) => entry(tick, "GotoDirection", point(d)),
        Command::Point(p) => entry(tick, "GotoPoint", point(p)),
        Command::Follow { target, range } => {
            entry(tick, "FollowBall", vec![id, Value::Int(*target), Value::Double(*range)])
        }
        Command::Orbit { target, range } => {
            entry(tick, "Orbit", vec![id, Value::Int(*target), Value::Double(*range)])
        }
        // WarpTo(ballID, x, y, z, range, warpFactor): without the factor
        // destiny warps everyone at 2 AU/s.
        Command::Warp { target, range } => {
            let mut args = point(target);
            args.push(Value::Double(*range));
            args.push(Value::Int(ship.stats.warp_factor));
            entry(tick, "WarpTo", args)
        }
    }
}

/// The opcodes that put a ship back on its current course, for a client that
/// has just been given the ship standing still.
pub fn course_entries(ship: &Ship, tick: i64) -> Vec<Value> {
    if ship.command == Command::Stop {
        return Vec::new();
    }
    let speed = entry(tick, "SetSpeedFraction", vec![Value::Int(ship.id), Value::Double(ship.speed_fraction)]);
    let mut entries = vec![command_entry(ship, tick), speed];
    if ship.in_warp() {
        entries.push(warp_fx(tick, ship.id, true));
    }
    entries
}

/// Synchronize an existing ball with the server's position and velocity.
pub fn ship_motion_entries(ship: &Ship, tick: i64) -> [Value; 2] {
    [("SetBallPosition", ship.pos), ("SetBallVelocity", ship.vel)].map(|(opcode, v)| {
        entry(tick, opcode, vec![Value::Int(ship.id), Value::Double(v.x), Value::Double(v.y), Value::Double(v.z)])
    })
}

/// Leave native warp and restore the ball's sublight motion in one update.
/// Replacing the whole park makes build 3913 append the scenery to its overview.
pub fn warp_landing_entries(ship: &Ship, tick: i64) -> Vec<Value> {
    let mut entries = vec![entry(tick, "Stop", vec![Value::Int(ship.id)])];
    entries.extend(ship_motion_entries(ship, tick));
    entries.push(entry(tick, "SetBallMassive", vec![Value::Int(ship.id), Value::Int(1)]));
    entries.push(warp_fx(tick, ship.id, false));
    entries.extend(course_entries(ship, tick));
    entries
}

/// SetState for `ego`: the park's fixed scenery, nearby ships and their courses.
pub fn set_state_entries(park: &Park, ego: i64, station_owners: &[(i64, i64)], char_gangs: &[(i64, i64)]) -> Vec<Value> {
    const DRONE_HEADER: &[&str] =
        &["droneID", "ownerID", "controllerID", "activityState", "typeID", "controllerOwnerID"];
    let mut balls: Vec<Ball> = park
        .celestials
        .iter()
        .map(|c| Ball::celestial(c.id, c.radius, c.pos.tuple(), &c.name, c.massive()))
        .collect();
    let mut slims: Vec<Value> = park.celestials.iter().map(|c| celestial_slim(c, station_owners)).collect();
    let visible_objects = visible_object_ids(park, ego);
    for rock in park.rocks.values() {
        if !visible_objects.contains(&rock.id) { continue; }
        balls.push(Ball::celestial(rock.id, rock.radius, rock.pos.tuple(), &rock.name, true));
        slims.push(crate::npc::rock_slim(rock));
    }
    for can in park.cans.values() {
        if !visible_objects.contains(&can.id) { continue; }
        balls.push(Ball::celestial(can.id, can.radius, can.pos.tuple(), &can.name, false));
        slims.push(crate::loot::slim(can));
    }
    for site in park.sites.values() {
        if !visible_objects.contains(&site.id) { continue; }
        balls.push(site.ball());
        slims.push(site.slim());
    }
    let mut damage = Vec::new();
    let now_ms = crate::combat::now_ms();
    for s in park.structures.values() {
        if !visible_objects.contains(&s.id) { continue; }
        balls.push(s.ball());
        slims.push(s.slim());
        damage.push((Value::Int(s.id), crate::fight::damage_state(&s.vitals, now_ms)));
        if let Some(f) = &s.field {
            balls.push(f.ball(s));
            slims.push(f.slim(s));
        }
    }
    let visible = visible_ship_ids(park, ego);
    for ship in park.ships.values().filter(|ship| visible.contains(&ship.id)) {
        balls.push(ship_ball(ship));
        slims.push(ship_slim(ship));
        damage.push((Value::Int(ship.id), health(ship)));
    }
    let bag = key_val(vec![
        ("state", Value::Bytes(destiny::encode(destiny::stamp(park.tick), &balls))),
        ("ego", Value::Int(ego)),
        ("slims", Value::List(slims)),
        (
            "solItem",
            slim(vec![
                ("itemID", Value::Int(park.solar_system_id)),
                ("typeID", Value::Int(TYPE_SOLAR_SYSTEM)),
                ("ownerID", Value::Int(OWNER_SYSTEM)),
            ]),
        ),
        ("aggressors", park.watch.aggressors()),
        ("droneState", rowset(DRONE_HEADER, crate::drones::state_lines(park))),
        ("gangCorps", Value::List(Vec::new())),
        (
            "charGangs",
            Value::List(char_gangs.iter().map(|(c, g)| Value::Tuple(vec![Value::Int(*c), Value::Int(*g)])).collect()),
        ),
        ("damageState", Value::Dict(damage)),
        ("effectStates", Value::List(Vec::new())),
    ]);
    let mut entries = vec![entry(park.tick, "SetState", vec![bag])];
    for ship in park.ships.values().filter(|ship| visible.contains(&ship.id)) {
        entries.extend(course_entries(ship, park.tick));
    }
    entries
}

pub(crate) fn visible_ship_ids(park: &Park, ego: i64) -> BTreeSet<i64> {
    park.ships
        .values()
        .filter(|ship| !crate::drones::in_transit(ship) && (ship.id == ego || within_overview_range(park, ego, ship.id)))
        .map(|ship| ship.id)
        .collect()
}

fn within_overview_range(park: &Park, viewer: i64, target: i64) -> bool {
    park.ships
        .get(&viewer)
        .zip(park.ships.get(&target))
        .is_some_and(|(viewer, target)| !crate::drones::in_transit(target) && (viewer.pos - target.pos).len() <= SHIP_OVERVIEW_RANGE)
}

fn within_overview_range_at(park: &Park, viewer: i64, at: Vec3) -> bool {
    park.ships.get(&viewer).is_some_and(|ship| (ship.pos - at).len() <= SHIP_OVERVIEW_RANGE)
}

fn object_positions(park: &Park) -> BTreeMap<i64, Vec3> {
    let mut positions = BTreeMap::new();
    positions.extend(park.rocks.values().map(|r| (r.id, r.pos)));
    positions.extend(park.cans.values().map(|c| (c.id, c.pos)));
    positions.extend(park.sites.values().map(|s| (s.id, s.pos)));
    for s in park.structures.values() {
        positions.insert(s.id, s.pos);
        if let Some(field) = &s.field {
            positions.insert(field.id, s.pos);
        }
    }
    positions
}

pub(crate) fn visible_object_ids(park: &Park, ego: i64) -> BTreeSet<i64> {
    visible_object_ids_at(park, ego, &object_positions(park))
}

fn visible_object_ids_at(park: &Park, ego: i64, positions: &BTreeMap<i64, Vec3>) -> BTreeSet<i64> {
    positions.iter().filter(|(_, at)| within_overview_range_at(park, ego, **at)).map(|(id, _)| *id).collect()
}

fn object_entry(park: &Park, id: i64) -> Option<Value> {
    let (ball, slim, health) = if let Some(rock) = park.rocks.get(&id) {
        (Ball::celestial(rock.id, rock.radius, rock.pos.tuple(), &rock.name, true), crate::npc::rock_slim(rock), None)
    } else if let Some(can) = park.cans.get(&id) {
        (Ball::celestial(can.id, can.radius, can.pos.tuple(), &can.name, false), crate::loot::slim(can), None)
    } else if let Some(site) = park.sites.get(&id) {
        (site.ball(), site.slim(), None)
    } else if let Some(s) = park.structures.get(&id) {
        (s.ball(), s.slim(), Some(crate::fight::damage_state(&s.vitals, crate::combat::now_ms())))
    } else {
        let (s, field) = park.structures.values().find_map(|s| s.field.as_ref().filter(|f| f.id == id).map(|f| (s, f)))?;
        (field.ball(s), field.slim(s), None)
    };
    let blob = destiny::encode(destiny::stamp(park.tick), &[ball]);
    let damage = health.map_or_else(Vec::new, |health| vec![(Value::Int(id), health)]);
    let chunk = Value::Tuple(vec![Value::Bytes(blob), Value::List(vec![slim]), Value::Dict(damage)]);
    Some(entry(park.tick, "AddBalls", vec![chunk]))
}

/// Add a new non-ship ball only to pilots who can see its position.
pub fn add_object_entries_for_nearby(park: &mut Park, id: i64) -> Vec<(i64, Value)> {
    let Some(at) = object_positions(park).get(&id).copied() else { return Vec::new() };
    let Some(add) = object_entry(park, id) else { return Vec::new() };
    let viewers: Vec<(i64, i64)> = park.ships.values().filter(|s| s.is_player()).map(|s| (s.client_id, s.id)).collect();
    let mut updates = Vec::new();
    for (client, ego) in viewers {
        if within_overview_range_at(park, ego, at)
            && park.visible_objects.get_mut(&client).is_some_and(|seen| seen.insert(id))
        {
            updates.push((client, add.clone()));
        }
    }
    updates
}

pub fn sync_object_visibility(park: &mut Park) -> Vec<(i64, Value)> {
    let viewers: Vec<(i64, i64)> = park.ships.values().filter(|s| s.is_player()).map(|s| (s.client_id, s.id)).collect();
    let active: BTreeSet<i64> = viewers.iter().map(|(client, _)| *client).collect();
    park.visible_objects.retain(|client, _| active.contains(client));
    let positions = object_positions(park);
    let mut updates = Vec::new();
    for (client, ego) in viewers {
        let Some(previous) = park.visible_objects.get(&client).cloned() else { continue };
        let current = visible_object_ids_at(park, ego, &positions);
        for id in current.difference(&previous) {
            if let Some(add) = object_entry(park, *id) {
                updates.push((client, add));
            }
        }
        for id in previous.difference(&current).filter(|id| positions.contains_key(*id)) {
            updates.extend(remove_ball_entries(park.tick, *id).into_iter().map(|entry| (client, entry)));
        }
        park.visible_objects.insert(client, current);
    }
    updates
}

/// Reconcile each connected pilot's visible ships with the one-million-metre
/// range. Newly visible ships arrive with their current position and course.
pub fn sync_ship_visibility(park: &mut Park) -> Vec<(i64, Value)> {
    let viewers: Vec<(i64, i64)> = park.ships.values().filter(|s| s.is_player()).map(|s| (s.client_id, s.id)).collect();
    let active: BTreeSet<i64> = viewers.iter().map(|(client, _)| *client).collect();
    park.visible_ships.retain(|client, _| active.contains(client));

    let mut updates = Vec::new();
    for (client, ego) in viewers {
        let Some(previous) = park.visible_ships.get(&client).cloned() else { continue };
        let current = visible_ship_ids(park, ego);
        let added: Vec<i64> = current.difference(&previous).copied().filter(|id| *id != ego).collect();
        let removed: Vec<i64> = previous
            .difference(&current)
            .copied()
            .filter(|id| *id != ego && park.ships.contains_key(id))
            .collect();
        let mut entries = Vec::new();
        for id in added {
            entries.extend(add_ball_entries(park, id));
        }
        for id in removed {
            entries.extend(remove_ball_entries(park.tick, id));
        }
        park.visible_ships.insert(client, current);
        updates.extend(entries.into_iter().map(|entry| (client, entry)));
    }
    updates
}

/// Add a newly spawned ship only to pilots who have received SetState and are
/// close enough to see it. The visibility set prevents later duplicate adds.
pub fn add_ball_entries_for_nearby(park: &mut Park, ship_id: i64) -> Vec<(i64, Value)> {
    let Some(ship) = park.ships.get(&ship_id) else { return Vec::new() };
    let owner = ship.client_id;
    let is_player_ship = ship.is_player();
    let viewers: Vec<(i64, i64)> = park
        .visible_ships
        .keys()
        .filter_map(|client| {
            park.ships.values().find(|s| s.is_player() && s.client_id == *client).map(|s| (*client, s.id))
        })
        .collect();
    let entries = add_ball_entries(park, ship_id);
    let mut updates = Vec::new();
    for (client, ego) in viewers {
        if (is_player_ship && client == owner)
            || !within_overview_range(park, ego, ship_id)
            || !park.visible_ships.get_mut(&client).is_some_and(|seen| seen.insert(ship_id))
        {
            continue;
        }
        updates.extend(entries.iter().cloned().map(|entry| (client, entry)));
    }
    updates
}

pub fn notify_targeted_updates(world: &WorldState, updates: Vec<(i64, Value)>) {
    let mut by_client: BTreeMap<i64, Vec<Value>> = BTreeMap::new();
    for (client, entry) in updates {
        by_client.entry(client).or_default().push(entry);
    }
    for (client, entries) in by_client {
        let args = Value::Tuple(vec![Value::List(entries), Value::Int(0)]);
        world.notify(&[client], "DoDestinyUpdate", args);
    }
}

pub fn ship_viewers(park: &Park, ship_id: i64) -> Vec<i64> {
    park.visible_ships
        .iter()
        .filter(|(_, seen)| seen.contains(&ship_id))
        .map(|(client, _)| *client)
        .collect()
}

pub fn queue_ship_entries(park: &Park, ship_id: i64, entries: Vec<Value>, out: &mut crate::heartbeat::Out) {
    for client in ship_viewers(park, ship_id) {
        for entry in &entries {
            out.destiny_to(park.solar_system_id, client, entry.clone());
        }
    }
}

/// AddBalls for one ship joining the park, then its course.
pub fn add_ball_entries(park: &Park, ship_id: i64) -> Vec<Value> {
    let Some(ship) = park.ships.get(&ship_id) else { return Vec::new() };
    let blob = destiny::encode(destiny::stamp(park.tick), &[ship_ball(ship)]);
    let chunk = Value::Tuple(vec![
        Value::Bytes(blob),
        Value::List(vec![ship_slim(ship)]),
        Value::Dict(vec![(Value::Int(ship.id), health(ship))]),
    ]);
    let mut entries = vec![entry(park.tick, "AddBalls", vec![chunk])];
    entries.extend(course_entries(ship, park.tick));
    entries
}

pub fn remove_ball_entries(tick: i64, ship_id: i64) -> Vec<Value> {
    vec![entry(tick, "RemoveBalls", vec![Value::List(vec![Value::Int(ship_id)])])]
}

/// OnSpecialFX(shipID, moduleID, moduleTypeID, targetID, otherTypeID, area, guid,
/// isOffensive, start, active) as a destiny entry (spaceMgr.py:120).
pub fn special_fx(tick: i64, ship_id: i64, target_id: Option<i64>, guid: &str) -> Value {
    special_fx_state(tick, ship_id, target_id, guid, true, false)
}

/// The build 3913 client's Warping effect owns the warp tunnel, sound and
/// camera shake. Keep it active from actual warp entry until landing.
pub fn warp_fx(tick: i64, ship_id: i64, start: bool) -> Value {
    special_fx_state(tick, ship_id, None, "effects.Warping", start, start)
}

fn special_fx_state(tick: i64, ship_id: i64, target_id: Option<i64>, guid: &str, start: bool, active: bool) -> Value {
    entry(
        tick,
        "OnSpecialFX",
        vec![
            Value::Int(ship_id),
            Value::None,
            Value::None,
            Value::int_or_none(target_id),
            Value::None,
            Value::List(Vec::new()),
            Value::bytes(guid),
            Value::Int(0),
            Value::Int(i64::from(start)),
            Value::Int(i64::from(active)),
        ],
    )
}

/// Stations' owning corporations, for their slims.
pub async fn station_owners(ctx: &Ctx<'_>, solar_system_id: i64) -> Vec<(i64, i64)> {
    let sql = r#"SELECT "stationID", "corporationID" FROM "staStations" WHERE "solarSystemID" = $1"#;
    ctx.query(sql, &[&(solar_system_id as i32)])
        .await
        .iter()
        .filter_map(|r| Some((int(r, 0)?, int(r, 1)?)))
        .collect()
}

/// Puts a ship into its system's park and shows it to everyone else there.
pub fn add_ship(world: &mut WorldState, system: i64, ship: Ship) {
    let ship_id = ship.id;
    let owner = ship.client_id;
    let in_space = world.in_space(system);
    let Some(park) = world.parks.get_mut(&system) else { return };
    park.advance(destiny::tick_now());
    park.ships.insert(ship_id, ship);
    let entries = add_ball_entries(park, ship_id);
    let others: Vec<i64> = in_space
        .into_iter()
        .filter(|client| *client != owner && park.visible_ships.contains_key(client))
        .filter(|client| {
            park.ships
                .values()
                .find(|s| s.is_player() && s.client_id == *client)
                .is_some_and(|viewer| within_overview_range(park, viewer.id, ship_id))
        })
        .collect();
    for client in &others {
        if let Some(visible) = park.visible_ships.get_mut(client) {
            visible.insert(ship_id);
        }
    }
    let args = Value::Tuple(vec![Value::List(entries), Value::Int(0)]);
    world.notify(&others, "DoDestinyUpdate", args);
}

/// Gives a ship in a park a new course and tells the system (a warp opens
/// the throttle first, as destiny wants).
pub fn order(world: &mut WorldState, system: i64, ship_id: i64, command: Command) {
    let Some(park) = world.parks.get_mut(&system) else { return };
    park.advance(destiny::tick_now());
    if !park.ships.contains_key(&ship_id) {
        return;
    }
    let tick = park.tick;
    let mut entries = Vec::new();
    if matches!(command, Command::Warp { .. }) && park.ships[&ship_id].speed_fraction < 1.0 {
        park.set_speed_fraction(ship_id, 1.0);
        entries.push(entry(tick, "SetSpeedFraction", vec![Value::Int(ship_id), Value::Double(1.0)]));
    }
    park.command(ship_id, command);
    entries.insert(0, command_entry(&park.ships[&ship_id], tick));
    let updates = ship_viewers(park, ship_id).into_iter().flat_map(|client| entries.iter().cloned().map(move |entry| (client, entry))).collect();
    notify_targeted_updates(world, updates);
}

/// Takes a ship out of its system's park, out of everyone's view and out of
/// every fight it was in; returns it.
pub fn remove_ship(world: &mut WorldState, system: i64, ship_id: i64) -> Option<Ship> {
    let park = world.parks.get_mut(&system)?;
    park.advance(destiny::tick_now());
    let mut out = crate::heartbeat::Out::default();
    let now = crate::combat::now_ms();
    crate::fight::stand_down(park, ship_id, now, &mut out);
    crate::drones::recover_nonfighters_for_controller(park, ship_id, now, &mut out);
    let ship = crate::fight::detach(park, ship_id, now, &mut out)?;
    for visible in park.visible_ships.values_mut() {
        visible.remove(&ship_id);
    }
    if ship.is_player() {
        park.visible_ships.remove(&ship.client_id);
    }
    let entries = remove_ball_entries(park.tick, ship_id);
    let jobs = out.deliver(world);
    world.pending_jobs.extend(jobs);
    let others: Vec<i64> = world.in_space(system).into_iter().filter(|c| *c != ship.client_id).collect();
    let args = Value::Tuple(vec![Value::List(entries), Value::Int(0)]);
    world.notify(&others, "DoDestinyUpdate", args);
    Some(ship)
}

/// Queues a SetState for the caller's own ship.
async fn push_set_state(ctx: &Ctx<'_>) {
    let (Some(system), Some(ship_id)) = (ctx.session.solar_system_id, ctx.session.ship_id) else {
        return;
    };
    ensure_park(ctx, system).await;
    let owners = station_owners(ctx, system).await;
    let mut world = ctx.world.lock();
    let Some(park) = world.parks.get_mut(&system) else { return };
    park.advance(destiny::tick_now());
    if !park.ships.contains_key(&ship_id) {
        warn!("SetState for ship {ship_id}, which is not in the park of {system}");
    }
    // Just out of a gate: the pilot sees the arrival too, once their park is up.
    let arrival = park.ships.get_mut(&ship_id).and_then(|s| s.arrived_through.take());
    let pilots: Vec<i64> = park.ships.values().map(|s| s.pilot.char_id).filter(|c| *c != 0).collect();
    let char_gangs = world.gangs.memberships(pilots);
    let Some(park) = world.parks.get(&system) else { return };
    let mut entries = set_state_entries(park, ship_id, &owners, &char_gangs);
    let visible = visible_ship_ids(park, ship_id);
    let visible_objects = visible_object_ids(park, ship_id);
    if let Some(gate) = arrival {
        entries.push(special_fx(park.tick, ship_id, Some(gate), "effects.JumpIn"));
    }
    debug!("SetState: {} celestial(s) and {} nearby ship(s), ego {ship_id}", park.celestials.len(), visible.len());
    if let Some(park) = world.parks.get_mut(&system) {
        park.visible_ships.insert(ctx.client_id, visible);
        park.visible_objects.insert(ctx.client_id, visible_objects);
    }
    broadcast(&world, system, Some(ctx.client_id), entries);
    world.sees(ctx.client_id, system);
}

/// (diff, x, y, z, serverTick, tTime, rTime): where the server has the ship, and
/// how far that is from where the client has it (michelle.GetRelativity).
fn relativity(ctx: &Ctx<'_>, call: &Call) -> Value {
    let client_pos = Vec3::new(
        call.args.first().and_then(|v| v.as_f64()).unwrap_or(0.0),
        call.args.get(1).and_then(|v| v.as_f64()).unwrap_or(0.0),
        call.args.get(2).and_then(|v| v.as_f64()).unwrap_or(0.0),
    );
    let tick = destiny::tick_now();
    let server_pos = ctx
        .session
        .solar_system_id
        .zip(ctx.session.ship_id)
        .and_then(|(system, ship)| {
            let mut world = ctx.world.lock();
            let park = world.parks.get_mut(&system)?;
            park.advance(tick);
            park.ships.get(&ship).map(|s| s.pos)
        })
        .unwrap_or(client_pos);
    Value::Tuple(vec![
        Value::Double((server_pos - client_pos).len()),
        Value::Double(server_pos.x),
        Value::Double(server_pos.y),
        Value::Double(server_pos.z),
        Value::Int(destiny::stamp(tick) as i64),
        Value::Long(crate::bulkdata::filetime_now()),
        Value::Long(crate::bulkdata::filetime_now()),
    ])
}

// ---- movement -------------------------------------------------------------------

enum Order {
    Command(Command),
    SpeedFraction(f64),
}

/// A movement call on the caller's ship: update the park, tell the system.
async fn movement(ctx: &Ctx<'_>, call: &Call) -> Outcome {
    let (Some(system), Some(ship_id)) = (ctx.session.solar_system_id, ctx.session.ship_id) else {
        return Outcome::UserError { msg: "ShipNotYours".into(), args: Vec::new() };
    };
    let Some(order) = parse_order(ctx, call).await else {
        return Outcome::Unknown(format!("beyonce::{} with arguments it cannot use", call.method));
    };
    steer(ctx, system, ship_id, order, &call.method)
}

/// Warps the caller's ship to `range` off something at `at` of `radius`
/// (an agent's location, say).
pub fn warp_to_point(ctx: &Ctx<'_>, at: Vec3, radius: f64, range: f64, why: &str) -> Outcome {
    let (Some(system), Some(ship_id)) = (ctx.session.solar_system_id, ctx.session.ship_id) else {
        return Outcome::UserError { msg: "ShipNotYours".into(), args: Vec::new() };
    };
    let Some(target) = landing_point(ctx, at, radius + range.max(0.0)) else {
        return Outcome::custom("Your ship is not in space here.");
    };
    steer(ctx, system, ship_id, Order::Command(Command::Warp { target, range: 0.0 }), why)
}

/// Flies the caller's ship straight to `at`.
pub fn approach_point(ctx: &Ctx<'_>, at: Vec3, why: &str) -> Outcome {
    let (Some(system), Some(ship_id)) = (ctx.session.solar_system_id, ctx.session.ship_id) else {
        return Outcome::UserError { msg: "ShipNotYours".into(), args: Vec::new() };
    };
    steer(ctx, system, ship_id, Order::Command(Command::Point(at)), why)
}

/// Puts a movement order to a ship: the park follows it, and the system is told.
fn steer(ctx: &Ctx<'_>, system: i64, ship_id: i64, order: Order, method: &str) -> Outcome {
    let mut world = ctx.world.lock();
    let Some(park) = world.parks.get_mut(&system) else {
        return Outcome::Unknown(format!("beyonce::{method}: no ballpark for {system}"));
    };
    park.advance(destiny::tick_now());
    let tick = park.tick;
    match park.ships.get(&ship_id) {
        None => return Outcome::Unknown(format!("beyonce::{method}: ship {ship_id} is not in space")),
        // destiny ignores course changes mid-warp; so does the server.
        Some(ship) if ship.in_warp() => return Outcome::Value(Value::None),
        Some(ship)
            if matches!(order, Order::Command(Command::Warp { .. }))
                && ship.fight.as_ref().is_some_and(|f| f.afflictions.scrambled >= 1.0) =>
        {
            return Outcome::UserError { msg: "WarpScrambled".into(), args: Vec::new() };
        }
        Some(_) => {}
    }
    let warp_order = matches!(order, Order::Command(Command::Warp { .. }));
    let (entries, what) = match order {
        Order::SpeedFraction(f) => {
            park.set_speed_fraction(ship_id, f);
            let speed = entry(tick, "SetSpeedFraction", vec![Value::Int(ship_id), Value::Double(f)]);
            (vec![speed], format!("speed {f:.2}"))
        }
        Order::Command(command) => {
            let what = format!("{command:?}");
            let mut entries = Vec::new();
            // destiny aligns at the throttle's speed and only warps above
            // 75% of the top one, so a warp opens it.
            if matches!(command, Command::Warp { .. }) && park.ships[&ship_id].speed_fraction < 1.0 {
                park.set_speed_fraction(ship_id, 1.0);
                entries.push(entry(tick, "SetSpeedFraction", vec![Value::Int(ship_id), Value::Double(1.0)]));
            }
            park.command(ship_id, command);
            entries.insert(0, command_entry(&park.ships[&ship_id], tick));
            (entries, what)
        }
    };
    debug!("ship {ship_id}: {what}");
    let ship = &park.ships[&ship_id];
    let mut updates = Vec::new();
    if warp_order && ship.warping() {
        updates.extend(ship_motion_entries(ship, tick).map(|entry| (ctx.client_id, entry)));
    }
    let mut viewers = ship_viewers(park, ship_id);
    if !viewers.contains(&ctx.client_id) {
        viewers.push(ctx.client_id);
    }
    updates.extend(viewers.into_iter().flat_map(|client| entries.iter().cloned().map(move |entry| (client, entry))));
    notify_targeted_updates(&world, updates);
    Outcome::Value(Value::None)
}

async fn parse_order(ctx: &Ctx<'_>, call: &Call) -> Option<Order> {
    let f = |i: usize| call.args.get(i).and_then(|v| v.as_f64());
    let vec3 = || Some(Vec3::new(f(0)?, f(1)?, f(2)?));
    let range = |default: f64| f(1).unwrap_or(default).max(0.0);
    Some(match call.method.as_str() {
        "SetSpeedFraction" => Order::SpeedFraction(f(0)?.clamp(0.0, 1.0)),
        "GotoDirection" => Order::Command(Command::Direction(vec3()?)),
        "GotoPoint" => Order::Command(Command::Point(vec3()?)),
        "Stop" => Order::Command(Command::Stop),
        "FollowBall" => Order::Command(Command::Follow {
            target: call.arg_i64(0)?,
            range: range(DEFAULT_FOLLOW_RANGE),
        }),
        "Orbit" => Order::Command(Command::Orbit { target: call.arg_i64(0)?, range: range(0.0) }),
        "WarpToStuff" => {
            let kind = call.arg_text(0)?;
            let id = call.arg_i64(1)?;
            let range = call.kwarg("minRange").and_then(|v| v.as_f64()).unwrap_or(DEFAULT_WARP_RANGE);
            let (at, radius, station) = warp_target(ctx, &kind, id).await?;
            let target = if station {
                let world = ctx.world.lock();
                let ship = world.parks.get(&ctx.session.solar_system_id?)?.ships.get(&ctx.session.ship_id?)?;
                station_warp_point(ship, at, radius, range)
            } else {
                landing_point(ctx, at, warp_reach(radius, range, false, 0.0))?
            };
            Order::Command(Command::Warp { target, range: 0.0 })
        }
        _ => return None,
    })
}

/// What WarpToStuff(kind, id) goes to, as its centre and radius: an item in
/// the system, a bookmark, or another character's ship.
async fn warp_target(ctx: &Ctx<'_>, kind: &str, id: i64) -> Option<(Vec3, f64, bool)> {
    let point = |r: &tokio_postgres::Row, i: usize| {
        Some(Vec3::new(float(r, i)?, float(r, i + 1)?, float(r, i + 2)?))
    };
    match kind {
        "item" => {
            if let Some(ball) = ctx.world.lock().parks.get(&ctx.session.solar_system_id?).and_then(|p| {
                p.ball(id).map(|(at, radius)| (at, radius, is_station(p, id)))
            }) {
                return Some(ball);
            }
            let sql = r#"SELECT "x","y","z", COALESCE("radius", 0), "groupID" FROM "mapDenormalize" WHERE "itemID" = $1"#;
            let r = ctx.query_one(sql, &[&(id as i32)]).await?;
            Some((point(&r, 0)?, float(&r, 3).unwrap_or(0.0), int(&r, 4) == Some(GROUP_STATION)))
        }
        "bookmark" => {
            let sql = r#"SELECT b."x", b."y", b."z", d."x", d."y", d."z", COALESCE(d."radius", 0), d."groupID"
                           FROM bookmarks b
                           LEFT JOIN "mapDenormalize" d ON d."itemID" = b."itemID" AND d."solarSystemID" = b."locationID"
                          WHERE b."bookmarkID" = $1 AND b."ownerID" = $2 AND b."locationID" = $3"#;
            let owner = ctx.session.char_id? as i32;
            let system = ctx.session.solar_system_id? as i32;
            let r = ctx.query_one(sql, &[&(id as i32), &owner, &system]).await?;
            // Map objects use local celestial coordinates. A coordinate bookmark
            // names its system, whose map row is in galaxy coordinates, so use its own XYZ.
            match point(&r, 3) {
                Some(at) => Some((at, float(&r, 6).unwrap_or(0.0), int(&r, 7) == Some(GROUP_STATION))),
                None => Some((point(&r, 0)?, 0.0, false)),
            }
        }
        "char" => {
            let world = ctx.world.lock();
            let park = world.parks.get(&ctx.session.solar_system_id?)?;
            park.ships.values().find(|s| s.pilot.char_id == id).map(|s| (s.pos, s.stats.radius, false))
        }
        _ => {
            warn!("WarpToStuff to a {kind:?} is not supported");
            None
        }
    }
}

fn warp_reach(radius: f64, range: f64, station: bool, coast: f64) -> f64 {
    radius + range.max(0.0) + if station { STATION_WARP_CLEARANCE + coast.max(0.0) } else { 0.0 }
}

/// The normal station landing point, including clearance for the exit coast.
pub(crate) fn station_warp_point(ship: &Ship, at: Vec3, radius: f64, range: f64) -> Vec3 {
    let offset = ship.pos - at;
    let coast = ship.stats.warp_exit_coast(offset.len());
    at + offset.unit() * (warp_reach(radius, range, true, coast) + ship.stats.radius)
}

/// Where the warp lands: `reach` (the target's radius plus the warp range)
/// out from `at` towards the ship, plus the ship's own radius, so the
/// overview's surfaceDist reads the range asked for.
///
/// The server works the point out and sends WarpTo a range of 0. Left to
/// destiny, the range is counted from wherever the ship finished aligning,
/// which beside a planet can point under its surface; and a warp of under
/// 100 km is a GotoPoint to the raw point, flying the ship into the planet.
fn landing_point(ctx: &Ctx<'_>, at: Vec3, reach: f64) -> Option<Vec3> {
    let world = ctx.world.lock();
    let park = world.parks.get(&ctx.session.solar_system_id?)?;
    let ship = park.ships.get(&ctx.session.ship_id?)?;
    Some(at + (ship.pos - at).unit() * (reach + ship.stats.radius))
}

// ---- jumps --------------------------------------------------------------------------

pub struct Jump {
    pub solar_system_id: i64,
    pub constellation_id: i64,
    pub region_id: i64,
    pub arrival: Vec3,
}

pub async fn resolve_jump(ctx: &Ctx<'_>, solar_system_id: i64, from_gate: i64, to_gate: i64) -> Option<Jump> {
    let sql = r#"
        SELECT d."solarSystemID", d."constellationID", d."regionID",
               d."x", d."y", d."z", COALESCE(d."radius", 0)
          FROM "mapJumps" j
          JOIN "mapDenormalize" g ON g."itemID" = j."stargateID"
          JOIN "mapDenormalize" d ON d."itemID" = j."celestialID"
         WHERE j."stargateID" = $1 AND j."celestialID" = $2
           AND g."solarSystemID" = $3 AND g."groupID" = $4"#;
    let params: [&(dyn tokio_postgres::types::ToSql + Sync); 4] =
        [&(from_gate as i32), &(to_gate as i32), &(solar_system_id as i32), &(GROUP_STARGATE as i32)];
    let r = ctx.query_one(sql, &params).await?;
    let gate = Vec3::new(float(&r, 3)?, float(&r, 4)?, float(&r, 5)?);
    Some(Jump {
        solar_system_id: int(&r, 0)?,
        constellation_id: int(&r, 1)?,
        region_id: int(&r, 2)?,
        arrival: arrival_point(gate, float(&r, 6).unwrap_or(0.0)),
    })
}

const ARRIVAL_CLEARANCE: f64 = 1500.0;

/// Where a ship comes out of a gate: just off its surface, on the sun's side.
pub fn arrival_point(gate: Vec3, gate_radius: f64) -> Vec3 {
    let offset = gate_radius + ARRIVAL_CLEARANCE;
    if gate.len() < offset {
        return gate + Vec3::new(offset, 0.0, 0.0);
    }
    gate - gate.unit() * offset
}

const UNDOCK_ALONG_MIN_COS: f64 = 0.25;
// The client measures undock distance from the hull's surface. Leave a
// small gap from the station's declared collision boundary, within 0-300 m.
const UNDOCK_CLEARANCE: f64 = 200.0;

/// The undock point and push of a station: its type's dock entry, heading out
/// along the dock orientation. The ship's centre starts beyond the entry by
/// enough to clear its hull, even when the exit direction is oblique.
///
/// Some types' orientation runs along the hull or back into it (type 56 has
/// its entry out along +Z and its orientation down -Y), which slides the
/// undocking ship into the station; those leave straight out from the centre
/// through the entry instead.
pub async fn undock_point(ctx: &Ctx<'_>, station_id: i64, ship_radius: f64) -> Option<(Vec3, Vec3)> {
    let sql = r#"SELECT d."x", d."y", d."z",
                        st."dockEntryX", st."dockEntryY", st."dockEntryZ",
                        st."dockOrientationX", st."dockOrientationY", st."dockOrientationZ",
                        d."radius"
                   FROM "staStations" s
                   JOIN "mapDenormalize" d ON d."itemID" = s."stationID"
                   LEFT JOIN "staStationTypes" st ON st."stationTypeID" = s."stationTypeID"
                  WHERE s."stationID" = $1"#;
    let r = ctx.query_one(sql, &[&(station_id as i32)]).await?;
    let v = |i: usize| Vec3::new(float(&r, i).unwrap_or(0.0), float(&r, i + 1).unwrap_or(0.0), float(&r, i + 2).unwrap_or(0.0));
    let (station, entry) = (v(0), v(3));
    let (offset, heading) = undock_exit(entry, v(6), float(&r, 9).unwrap_or(0.0), ship_radius);
    Some((station + offset, heading))
}

pub(crate) fn undock_exit(entry: Vec3, orientation: Vec3, station_radius: f64, ship_radius: f64) -> (Vec3, Vec3) {
    let heading = undock_heading(entry, orientation);
    let hull_clearance = ship_radius.max(0.0) + UNDOCK_CLEARANCE;
    // The dock entry is a point on the station model, not its outer bound.
    // Solve the ray/sphere intersection so every hull clears the station's
    // declared radius, even on large station models or oblique undock lanes.
    let outer_radius = station_radius.max(10_000.0) + hull_clearance;
    let forward = entry.dot(heading);
    let discriminant = forward * forward + outer_radius * outer_radius - entry.dot(entry);
    let distance = if discriminant > 0.0 {
        (-forward + discriminant.sqrt()).max(0.0)
    } else {
        0.0
    };
    (entry + heading * distance, heading)
}

/// See `undock_point`: the orientation if it leads away from the station,
/// within ~75 degrees of straight out (the sound types are within 70).
fn undock_heading(entry: Vec3, orientation: Vec3) -> Vec3 {
    let (out, along) = (entry.unit(), orientation.unit());
    if along != Vec3::ZERO && (out == Vec3::ZERO || out.dot(along) > UNDOCK_ALONG_MIN_COS) {
        along
    } else if out != Vec3::ZERO {
        out
    } else {
        Vec3::new(1.0, 0.0, 0.0)
    }
}

/// Is `id` a station in the park?
pub fn is_station(park: &Park, id: i64) -> bool {
    park.celestial(id).is_some_and(|c| c.group_id == GROUP_STATION)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ballpark::{Pilot, ShipStats};

    #[test]
    fn station_warp_keeps_a_zero_range_inside_dock_range_but_clear_of_the_hull() {
        assert_eq!(warp_reach(43_259.0, 0.0, true, 3_000.0), 48_259.0);
        assert_eq!(warp_reach(43_259.0, 100_000.0, true, 3_000.0), 148_259.0);
        assert_eq!(warp_reach(1.0, 0.0, false, 3_000.0), 1.0);
        assert!(STATION_WARP_CLEARANCE > 1_000.0);
    }

    #[test]
    fn a_destiny_entry_is_stamp_then_opcode_then_args() {
        let e = entry(7, "Stop", vec![Value::Int(140_000_016)]);
        let Value::Tuple(outer) = &e else { panic!("entry is a tuple") };
        assert_eq!(outer[0], Value::Int(7));
        let Value::Tuple(event) = &outer[1] else { panic!("event is a tuple") };
        assert_eq!(event[0], Value::bytes("Stop"));
        assert_eq!(event[1], Value::Tuple(vec![Value::Int(140_000_016)]));
    }

    #[test]
    fn warp_visual_starts_and_stops_with_the_client_effect() {
        for (start, expected) in [(true, 1), (false, 0)] {
            let Value::Tuple(entry) = warp_fx(7, 140_000_016, start) else { panic!() };
            let Value::Tuple(event) = &entry[1] else { panic!() };
            assert_eq!(event[0], Value::bytes("OnSpecialFX"));
            let Value::Tuple(args) = &event[1] else { panic!() };
            assert_eq!(args[0], Value::Int(140_000_016));
            assert_eq!(args[6], Value::bytes("effects.Warping"));
            assert_eq!(args[8], Value::Int(expected));
            assert_eq!(args[9], Value::Int(expected));
        }
    }

    fn park_with_moving_ship() -> Park {
        let mut park = Park::new(30002780, Vec::new(), 100);
        let stats = ShipStats::new(37.0, 1_100_000.0, 3.1, 320.0, 2.0);
        let pilot = Pilot {
            type_id: 587,
            group_id: 25,
            name: "Rifter".into(),
            char_id: 140000001,
            corp_id: 1000002,
            alliance_id: None,
            security: -1.5,
            bounty: 0.0,
        };
        park.ships.insert(1, Ship::new(1, 10, stats, pilot, Vec3::ZERO, Vec3::ZERO));
        park.command(1, Command::Orbit { target: 2, range: 500.0 });
        park
    }

    #[test]
    fn set_state_comes_first_and_moving_ships_are_put_back_on_course() {
        let park = park_with_moving_ship();
        let entries = set_state_entries(&park, 1, &[], &[]);
        let opcode = |e: &Value| match e {
            Value::Tuple(t) => match &t[1] {
                Value::Tuple(ev) => ev[0].clone(),
                _ => unreachable!(),
            },
            _ => unreachable!(),
        };
        let names: Vec<Value> = entries.iter().map(opcode).collect();
        assert_eq!(
            names,
            vec![Value::bytes("SetState"), Value::bytes("Orbit"), Value::bytes("SetSpeedFraction")]
        );
    }

    #[test]
    fn a_ship_slim_carries_a_numeric_security_status() {
        let park = park_with_moving_ship();
        let Value::Instance { state, .. } = ship_slim(&park.ships[&1]) else { panic!() };
        assert_eq!(state.get_str("securityStatus"), Some(&Value::Double(-1.5)));
        assert_eq!(state.get_str("allianceID"), Some(&Value::None));
    }

    #[test]
    fn a_jump_lands_outside_the_gate_and_inside_jump_range() {
        let gate = Vec3::new(-3.2e11, 4.1e10, 7.7e11);
        let at = arrival_point(gate, 2960.0);
        let surface = (at - gate).len() - 2960.0;
        assert!((surface - ARRIVAL_CLEARANCE).abs() < 0.01, "surface distance {surface}");
        assert!(at.len() < gate.len(), "on the sun's side");
    }

    #[test]
    fn an_undock_heads_away_from_the_station() {
        // Type 3870: the orientation leads out of the bay, so it is kept.
        let h = undock_heading(Vec3::new(6765.0, 938.0, 0.0), Vec3::new(6947.0, -2257.0, 0.0));
        assert!((h - Vec3::new(6947.0, -2257.0, 0.0).unit()).len() < 1e-9);
        // Type 56: the orientation runs down the hull; leave through the entry.
        let h = undock_heading(Vec3::new(0.0, -198.0, 4800.0), Vec3::new(0.0, -4967.0, 0.0));
        assert!((h - Vec3::new(0.0, -198.0, 4800.0).unit()).len() < 1e-9);
        // Types 58/59: no entry offset, only an orientation.
        assert_eq!(undock_heading(Vec3::ZERO, Vec3::new(0.0, 0.0, -1.0)), Vec3::new(0.0, 0.0, -1.0));
    }

    #[test]
    fn an_undocking_ship_clears_every_station_radius() {
        for (entry, orientation) in [
            (Vec3::new(6765.0, 938.0, 0.0), Vec3::new(6947.0, -2257.0, 0.0)),
            (Vec3::new(1_000.0, 0.0, 0.0), Vec3::new(1.0, 2.0, 0.0)),
            (Vec3::new(-1792.0, -9488.0, 27.0), Vec3::new(0.0, -1911.0, 0.0)),
            (Vec3::new(0.0, -198.0, 4800.0), Vec3::new(0.0, -4967.0, 0.0)),
        ] {
            let (at, heading) = undock_exit(entry, orientation, 30_000.0, 1_650.0);
            assert!((at.len() - (30_000.0 + 1_650.0 + UNDOCK_CLEARANCE)).abs() < 1e-6);
            assert!((at - entry).dot(heading) >= 0.0);
            assert!(heading.dot(entry.unit()) > 0.0);
        }
    }

    #[test]
    fn an_undock_without_an_entry_starts_outside_the_station() {
        let (at, heading) = undock_exit(Vec3::ZERO, Vec3::new(0.0, 0.0, -1.0), 20_000.0, 500.0);
        assert_eq!(heading, Vec3::new(0.0, 0.0, -1.0));
        assert_eq!(at.len(), 20_000.0 + 500.0 + UNDOCK_CLEARANCE);
    }

    #[test]
    fn directional_scan_filters_by_range_and_camera_cone() {
        let ahead = Vec3::new(100.0, 0.0, 0.0);
        let behind = Vec3::new(-100.0, 0.0, 0.0);
        let flank = Vec3::new(0.0, 100.0, 0.0);
        let heading = Vec3::new(1.0, 0.0, 0.0);
        let narrow = std::f64::consts::FRAC_PI_6.cos();
        assert!(within_scan(ahead, 100.0, narrow, heading));
        assert!(!within_scan(ahead, 99.0, narrow, heading));
        assert!(!within_scan(flank, 100.0, narrow, heading));
        assert!(!within_scan(behind, 100.0, narrow, heading));
        assert!(within_scan(behind, 100.0, -1.0, heading));
        let edge = Vec3::new(MAX_SCAN_RANGE, 0.0, 0.0);
        assert!(within_scan(edge, MAX_SCAN_RANGE * 2.0, narrow, heading));
        assert!(!within_scan(edge + heading, MAX_SCAN_RANGE * 2.0, -1.0, heading));
    }
}
