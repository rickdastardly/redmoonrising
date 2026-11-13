//! Drones: launched from a ship's drone bay into space (ship.Drop), told what
//! to do through the solar system's `entity` object (menusvc.py: CmdEngage,
//! CmdMine, CmdReturnHome, CmdReturnBay...), and scooped back (ScoopDrone or
//! by flying home).
//!
//! In space a drone is its own item in the solar system and a ship in the
//! park, with a [`Drone`] instead of a pilot. It fights with the guns its type
//! has, as rats do, and mines into its controller's cargo hold. Its pilot
//! hears what it is doing as OnDroneStateChange, with the activity states of
//! drone.py (idle, fighting, mining, returning...).

use std::collections::{HashMap, HashSet};

use eve_marshal::Value;
use tokio_postgres::Client;
use tracing::{debug, error, info, warn};

use crate::ballpark::{Command, Park, Pilot, Ship, ShipStats, Vec3};
use crate::combat::{self, attr, Action, Active, Lock, REPEAT_FOREVER};
use crate::conn::Server;
use crate::heartbeat::{Job, Out};
use crate::items::{self, ItemTxn, NewItem};
use crate::npc;
use crate::services::{Call, Ctx, Outcome};
use crate::svc::space;
use crate::world::World;

const FLAG_DRONE_BAY: i64 = 87;
const CATEGORY_DRONE: i64 = 18;
const EFFECT_MINING: i64 = 17;
/// A drone this close to its ship is back in the bay.
const DOCK_RANGE: f64 = 1_500.0;
/// A returning drone also docks once it is within this many seconds of
/// flight at its top speed. destiny eases a follower's thrust over the last
/// second of travel at full speed (`steer`), so a fast light drone (5-8 km/s)
/// crawls through its last few kilometres and could take half a minute to
/// cover the final 1,500 m.
const DOCK_REACH_SECONDS: f64 = 0.6;
/// How often a drone on its way into the bay checks whether it has arrived.
const RETURN_THINK_MS: i64 = 250;
/// How far from its ship it waits.
const IDLE_ORBIT: f64 = 1_000.0;
/// How far a pilot reaches to scoop a drone (const.maxCargoContainerTransferDistance).
const SCOOP_RANGE: f64 = 2_500.0;
/// How far beyond its ship's hull a drone comes out.
const LAUNCH_CLEARANCE: f64 = 150.0;
const THINK_MS: i64 = 1_000;
const COMBAT_DRONE_GROUP: i64 = 100;
/// Legacy combat drones omit scan resolution. Give them frigate-class sensors.
const DEFAULT_COMBAT_DRONE_SCAN_RESOLUTION: f64 = 400.0;
const FIGHTER_GROUP: i64 = 549;
const FIGHTER_ARRIVAL_MS: i64 = 60_000;
const CONTROLLER_GRID_RANGE: f64 = 1_000_000.0;

/// drone.py's activity states (eveConstants entity*).
pub mod activity {
    pub const IDLE: i64 = 0;
    pub const COMBAT: i64 = 1;
    pub const MINING: i64 = 2;
    pub const DEPARTING: i64 = 4;
    pub const DEPARTING_2: i64 = 5;
}

#[derive(Debug, Clone, PartialEq)]
pub enum Order {
    Idle,
    Engage(i64),
    Mine { target: i64, repeat: bool },
    /// Back to the ship, to wait there.
    ReturnHome,
    /// Back into the drone bay.
    ReturnBay,
}

#[derive(Debug, Clone)]
pub struct Drone {
    pub owner: i64,
    pub client_id: i64,
    /// The ship it works for, while that ship is here.
    pub controller: Option<i64>,
    pub order: Order,
    pub activity: i64,
    weapons: Vec<Active>,
    miner: Option<Active>,
    /// Whether the miner has run since the last Mine order.
    mined: bool,
    orbit: f64,
    cruise_fraction: f64,
    next_think_ms: i64,
    transit: Option<Transit>,
}

#[derive(Debug, Clone)]
pub struct Transit {
    target: i64,
    arrival_ms: Option<i64>,
}

pub fn in_transit(ship: &Ship) -> bool {
    ship.drone.as_ref().is_some_and(|d| d.transit.is_some())
}

/// Deployed ordinary drones are not restored as distant-space actors after a
/// restart. Put older stranded inventory items back in their owner's active
/// ship, leaving fighters in space for their follow behaviour.
pub async fn recover_stale_nonfighters(db: &Client) -> Result<u64, tokio_postgres::Error> {
    db.execute(r#"
        UPDATE entity AS drone SET "locationID" = c."shipID", "flag" = 87
          FROM character_ c, "invTypes" t, "invGroups" g, "mapSolarSystems" sys
         WHERE drone."ownerID" = c."characterID"
           AND drone."typeID" = t."typeID" AND t."groupID" = g."groupID"
           AND g."categoryID" = 18 AND g."groupID" <> 549
           AND drone."locationID" = sys."solarSystemID"
           AND EXISTS (SELECT 1 FROM entity AS ship
                        WHERE ship."itemID" = c."shipID" AND ship."ownerID" = c."characterID")
    "#, &[]).await
}

/// Rows for SetState's droneState rowset:
/// droneID, ownerID, controllerID, activityState, typeID, controllerOwnerID.
pub fn state_lines(park: &Park) -> Vec<Value> {
    park.ships
        .values()
        .filter_map(|s| s.drone.as_ref().map(|d| (s, d)))
        .map(|(s, d)| Value::List(state_line(s.id, d, s.pilot.type_id)))
        .collect()
}

fn state_line(id: i64, d: &Drone, type_id: i64) -> Vec<Value> {
    vec![
        Value::Int(id),
        Value::Int(d.owner),
        Value::int_or_none(d.controller),
        Value::Int(d.activity),
        Value::Int(type_id),
        Value::Int(d.owner),
    ]
}

// ---- launching -----------------------------------------------------------------------------

/// ship.Drop(itemIDs): one drone into space per ID listed. Returns the
/// launched drones' IDs.
pub async fn launch(ctx: &Ctx<'_>, ids: Vec<i64>) -> Outcome {
    let (Some(char_id), Some(ship_id), Some(system)) = (ctx.session.char_id, ctx.session.ship_id, ctx.session.solar_system_id) else {
        return Outcome::custom("Drones can only be launched in space.");
    };
    let (ship_pos, ship_radius, out_already, active) = {
        let world = ctx.world.lock();
        let Some(park) = world.parks.get(&system) else { return Outcome::custom("You are not in space.") };
        let Some(ship) = park.ships.get(&ship_id) else { return Outcome::custom("You are not in space.") };
        if park.inside_force_field(ship.pos) {
            return Outcome::custom("You cannot launch drones inside a starbase force field.");
        }
        let out = park.ships.values().filter(|s| s.drone.as_ref().is_some_and(|d| d.owner == char_id)).count();
        let active = ship.fight.iter().flat_map(|f| &f.modules).map(|m| m.module_id).collect();
        (ship.pos, ship.stats.radius, out, active)
    };
    let limit = match max_drones(ctx, char_id, ship_id, &active).await {
        Ok(l) => l,
        Err(e) => {
            error!("launching drones: {}", crate::logging::chain(&e));
            return Outcome::custom("Your drones could not be launched.");
        }
    };
    if limit == 0 {
        return Outcome::custom("You need the Drones skill to launch drones.");
    }
    let room = limit.saturating_sub(out_already);
    if room == 0 {
        return Outcome::custom(format!("You can control at most {limit} drones."));
    }
    let wanted: Vec<i64> = ids.into_iter().take(room).collect();

    let mut rng = combat::Rng::seeded(combat::now_ms() as u64 ^ ship_id as u64);
    let result: Result<Option<(Vec<(i64, i64, Vec3)>, Vec<Value>)>, tokio_postgres::Error> = async {
        let mut client = crate::db::connect(ctx.db_url).await?;
        let mut txn = ItemTxn::begin(&mut client).await?;
        let mut launched = Vec::new();
        for id in wanted {
            let Some(item) = txn.item(id).await? else { continue };
            if item.owner_id != char_id || item.location_id != ship_id || item.flag != FLAG_DRONE_BAY || item.category_id != CATEGORY_DRONE {
                continue;
            }
            let drone = if item.quantity > 1 {
                txn.set_quantity(id, item.quantity - 1).await?;
                txn.create(NewItem { type_id: item.type_id, owner_id: char_id, location_id: system, flag: 0, quantity: 1, singleton: true })
                    .await?
            } else {
                txn.move_to(id, system, 0).await?;
                txn.set_singleton(id, true).await?;
                id
            };
            // Clear of the hull: destiny would shove a drone launched inside it.
            let pos = ship_pos + rng.in_sphere(1.0).unit() * (ship_radius + LAUNCH_CLEARANCE);
            txn.set_position(drone, pos.tuple()).await?;
            launched.push((drone, item.type_id, pos));
        }
        if launched.is_empty() {
            return Ok(None);
        }
        Ok(Some((launched, txn.commit().await?)))
    }
    .await;
    let (launched, events) = match result {
        Ok(Some(v)) => v,
        Ok(None) => return Outcome::custom("Those drones are not in your drone bay."),
        Err(e) => {
            error!("launching drones: {}", crate::logging::chain(&e));
            return Outcome::custom("Your drones could not be launched.");
        }
    };
    items::notify(ctx.world, ctx.client_id, events);

    let types: Vec<i64> = launched.iter().map(|(_, t, _)| *t).collect();
    let types = match npc::load_types(ctx.db, &types).await {
        Ok(t) => t,
        Err(e) => {
            error!("reading drone types: {}", crate::logging::chain(&e));
            HashMap::new()
        }
    };
    // The pilot's drone skills, through dogma.
    let drone_ids: Vec<i64> = launched.iter().map(|(id, _, _)| *id).collect();
    let skilled: HashMap<i64, HashMap<i32, f64>> =
        match crate::dogma::Loadout::load_with_drones(ctx.db, ctx.dogma, char_id, Some(ship_id), &Default::default(), &drone_ids).await {
            Ok(l) => drone_ids.iter().filter_map(|id| Some((*id, l.attributes(l.find(*id)?)))).collect(),
            Err(e) => {
                error!("applying drone skills: {}", crate::logging::chain(&e));
                HashMap::new()
            }
        };
    let now = combat::now_ms();
    let mut world = ctx.world.lock();
    let Some(park) = world.parks.get_mut(&system) else { return Outcome::Value(Value::List(Vec::new())) };
    let mut out = Out::default();
    let mut ids = Vec::new();
    for (id, type_id, pos) in launched {
        let Some(t) = types.get(&type_id) else { continue };
        let t = &t.with_attributes(skilled.get(&id));
        let pilot = Pilot {
            type_id,
            group_id: t.group_id,
            name: t.name.clone(),
            char_id,
            corp_id: ctx.session.corp_id.unwrap_or(0),
            alliance_id: ctx.session.alliance_id,
            security: 0.0,
            bounty: 0.0,
        };
        let drone = make(t, id, pilot, ctx.client_id, ship_id, pos, now, &mut park.rng);
        park.ships.insert(id, drone);
        for (client, e) in space::add_ball_entries_for_nearby(park, id) {
            out.destiny_to(system, client, e);
        }
        if let Some(d) = park.ships.get(&id).and_then(|s| s.drone.as_ref()) {
            out.note(ctx.client_id, "OnDroneStateChange", Value::Tuple(state_line(id, d, type_id)));
        }
        ids.push(id);
    }
    out.deliver(&world);
    info!("launched {} drone(s) from {ship_id}", ids.len());
    Outcome::Value(Value::List(ids.into_iter().map(Value::Int).collect()))
}

/// How many drones the pilot can run: the character's computed
/// maxActiveDrones, including carrier skills and active control units.
async fn max_drones(ctx: &Ctx<'_>, char_id: i64, ship_id: i64, active: &HashSet<i64>) -> Result<usize, tokio_postgres::Error> {
    let loadout = crate::dogma::Loadout::load(ctx.db, ctx.dogma, char_id, Some(ship_id), active).await?;
    Ok(loadout.character().map_or(0.0, |c| loadout.value(c, attr::MAX_ACTIVE_DRONES)).max(0.0) as usize)
}

#[allow(clippy::too_many_arguments)]
fn make(t: &npc::NpcType, id: i64, pilot: Pilot, client_id: i64, controller: i64, pos: Vec3, now: i64, rng: &mut combat::Rng) -> Ship {
    let a = |x: i32| t.get(x);
    let top = a(attr::MAX_VELOCITY).max(a(attr::ENTITY_CRUISE_SPEED)).max(100.0);
    let agility = if a(attr::AGILITY) > 0.0 { a(attr::AGILITY) } else { 1.0 };
    let mass = if t.mass > 0.0 { t.mass } else { 3_000.0 };
    let stats = ShipStats::new(t.radius.max(5.0), mass, agility, top, 1.0);
    let mut fighter = npc::entity_fighter(t, 60_000.0, 1, top, now);
    if t.group_id == COMBAT_DRONE_GROUP && a(attr::SCAN_RESOLUTION) <= 0.0 {
        fighter.scan_resolution = DEFAULT_COMBAT_DRONE_SCAN_RESOLUTION;
    }
    if fighter.vitals.hull_max <= 1.0 {
        fighter.vitals.hull_max = 50.0;
    }
    let miner = (a(attr::MINING_AMOUNT) > 0.0).then(|| {
        let action = Action::Mining { amount: a(attr::MINING_AMOUNT), range: a(attr::MAX_RANGE).max(1_000.0) };
        let mut m = npc::entity_module(t, id, EFFECT_MINING, "effects.Mining", action, a(attr::DURATION));
        m.offensive = false;
        m
    });
    let orbit = if a(attr::ORBIT_RANGE) > 0.0 { a(attr::ORBIT_RANGE) } else { a(attr::ENTITY_FLY_RANGE).max(500.0) };
    let mut weapons = npc::weapons(t, &HashMap::new(), id, rng);
    let mut cruise_fraction = 1.0;
    if t.group_id == COMBAT_DRONE_GROUP {
        let cruise = a(attr::ENTITY_CRUISE_SPEED);
        if cruise > 0.0 {
            cruise_fraction = (cruise / top).min(1.0);
        }
        for weapon in &mut weapons {
            if let Action::Turret { gun, .. } = weapon.action {
                let range = a(attr::ENTITY_ATTACK_RANGE);
                let range = if range > 0.0 { range } else { gun.optimal + gun.falloff };
                if range > 0.0 {
                    weapon.reach = Some((range, 0.0));
                }
            }
        }
    }
    let drone = Drone {
        owner: pilot.char_id,
        client_id,
        controller: Some(controller),
        order: Order::Idle,
        activity: activity::IDLE,
        weapons,
        miner,
        mined: false,
        orbit,
        cruise_fraction,
        next_think_ms: now,
        transit: None,
    };
    let mut ship = Ship::new(id, client_id, stats, pilot, pos, Vec3::ZERO);
    ship.fight = Some(fighter);
    ship.drone = Some(drone);
    ship.command = Command::Orbit { target: controller, range: IDLE_ORBIT };
    ship
}

// ---- orders ----------------------------------------------------------------------------------

/// The solar system's `entity` object: drone orders. Each returns the drones
/// it could not order, as {droneID: (message, args)}.
pub async fn entity(call: &Call, ctx: &Ctx<'_>) -> Option<Outcome> {
    let ids: Vec<i64> = call.args.first().and_then(|v| v.seq()).unwrap_or(&[]).iter().filter_map(|v| v.as_i64()).collect();
    let target = call.arg_i64(1);
    if matches!(call.method.as_str(), "CmdAbandonDrone" | "CmdRelinquishControl") {
        return Some(Outcome::Value(abandon(ctx, &ids)));
    }
    let order = match call.method.as_str() {
        "CmdEngage" => Order::Engage(target?),
        "CmdMine" => Order::Mine { target: target?, repeat: false },
        "CmdMineRepeatedly" => Order::Mine { target: target?, repeat: true },
        "CmdReturnHome" => Order::ReturnHome,
        "CmdReturnBay" => Order::ReturnBay,
        "CmdDelegateControl" | "CmdUnanchor" => {
            return Some(Outcome::custom("Your drones cannot do that."));
        }
        _ => return None,
    };
    Some(Outcome::Value(command(ctx, &ids, order)))
}

fn command(ctx: &Ctx<'_>, ids: &[i64], order: Order) -> Value {
    let (Some(char_id), Some(system)) = (ctx.session.char_id, ctx.session.solar_system_id) else {
        return Value::Dict(Vec::new());
    };
    let mut failed = Vec::new();
    let mut world = ctx.world.lock();
    let Some(park) = world.parks.get_mut(&system) else { return Value::Dict(Vec::new()) };
    let target_ok = match &order {
        Order::Engage(t) => park.ships.get(t).is_some_and(|s| s.fight.is_some() && s.pilot.char_id != char_id),
        Order::Mine { target, .. } => park.rocks.contains_key(target),
        _ => true,
    };
    let transit_destination = match &order {
        Order::Engage(target) => Some(*target),
        Order::ReturnHome | Order::ReturnBay => ctx.session.ship_id,
        _ => None,
    };
    let destination_in_warp = transit_destination
        .and_then(|id| park.ships.get(&id))
        .is_some_and(Ship::in_warp);
    let now = combat::now_ms();
    for id in ids {
        // A drone already back in the bay has done what a return asks.
        if matches!(order, Order::ReturnHome | Order::ReturnBay) && !park.ships.contains_key(id) {
            continue;
        }
        let Some(drone) = park.ships.get_mut(id).and_then(|s| s.drone.as_mut()).filter(|d| d.owner == char_id && d.controller.is_some()) else {
            failed.push((*id, "That is not your drone."));
            continue;
        };
        if !target_ok {
            failed.push((*id, "Your drones cannot do that to that target."));
            continue;
        }
        if matches!(order, Order::Mine { .. }) && drone.miner.is_none() {
            failed.push((*id, "That drone cannot mine."));
            continue;
        }
        order_drone(drone, &order, transit_destination, destination_in_warp, now);
    }
    debug!("drones {ids:?}: {order:?}");
    Value::Dict(
        failed
            .into_iter()
            .map(|(id, why)| {
                let args = Value::Dict(vec![(Value::bytes("error"), Value::text(why))]);
                (Value::Int(id), Value::Tuple(vec![Value::bytes("CustomError"), args]))
            })
            .collect(),
    )
}

fn order_drone(drone: &mut Drone, order: &Order, destination: Option<i64>, destination_in_warp: bool, now: i64) {
    drone.order = order.clone();
    drone.mined = false;
    drone.next_think_ms = 0;
    if let (Some(transit), Some(destination)) = (drone.transit.as_mut(), destination) {
        // A new order supersedes the fighter's old warp destination. In
        // particular, a target that left the grid must not strand a fighter
        // that has since been told to return to its carrier.
        transit.target = destination;
        transit.arrival_ms = (!destination_in_warp).then_some(now + FIGHTER_ARRIVAL_MS);
    }
}

fn abandon(ctx: &Ctx<'_>, ids: &[i64]) -> Value {
    let (Some(char_id), Some(system)) = (ctx.session.char_id, ctx.session.solar_system_id) else {
        return Value::Dict(Vec::new());
    };
    let mut world = ctx.world.lock();
    let Some(park) = world.parks.get_mut(&system) else { return Value::Dict(Vec::new()) };
    let mut out = Out::default();
    let mut failed = Vec::new();
    for id in ids {
        let owned = park.ships.get(id).and_then(|s| s.drone.as_ref()).is_some_and(|d| d.owner == char_id && d.controller.is_some());
        if !owned {
            failed.push((*id, "That is not a drone under your control."));
            continue;
        }
        let modules: Vec<i64> = park.ships.get(id).and_then(|s| s.fight.as_ref())
            .map(|f| f.modules.iter().map(|m| m.module_id).collect()).unwrap_or_default();
        for module in modules {
            crate::fight::stop(park, *id, module, combat::now_ms(), &mut out, None);
        }
        if let Some(ship) = park.ships.get_mut(id) {
            if let Some(fight) = ship.fight.as_mut() { fight.locks.clear(); }
            if let Some(drone) = ship.drone.as_mut() {
                drone.controller = None;
                drone.order = Order::Idle;
                drone.activity = activity::IDLE;
                drone.next_think_ms = 0;
                out.note(drone.client_id, "OnDroneStateChange", Value::Tuple(state_line(*id, drone, ship.pilot.type_id)));
            }
        }
        park.command(*id, Command::Stop);
        if let Some(entry) = park.ships.get(id).map(|ship| space::command_entry(ship, park.tick)) {
            space::queue_ship_entries(park, *id, vec![entry], &mut out);
        }
    }
    out.deliver(&world);
    Value::Dict(failed.into_iter().map(|(id, why)| {
        let args = Value::Dict(vec![(Value::bytes("error"), Value::text(why))]);
        (Value::Int(id), Value::Tuple(vec![Value::bytes("CustomError"), args]))
    }).collect())
}

// ---- thinking ---------------------------------------------------------------------------------

pub fn think(park: &mut Park, now: i64, out: &mut Out) {
    let due: Vec<i64> = park
        .ships
        .values()
        .filter(|s| s.drone.as_ref().is_some_and(|d| d.transit.is_none() && d.next_think_ms <= now))
        .map(|s| s.id)
        .collect();
    for id in due {
        think_one(park, id, now, out);
    }
}

/// Fighters leave the old grid with a warping target or controller. A fighter
/// pursuing its attack target keeps that order; one following its controller
/// returns to idle. Its delayed arrival is scheduled from the landing tick.
pub fn on_warp_entered(park: &mut Park, warped: i64, now: i64, out: &mut Out) {
    recover_nonfighters_for_controller(park, warped, now, out);
    let followers: Vec<(i64, bool)> = park.ships.values().filter_map(|ship| {
        let d = ship.drone.as_ref()?;
        if ship.pilot.group_id != FIGHTER_GROUP { return None; }
        if d.order == Order::Engage(warped) { Some((ship.id, true)) }
        else if d.controller == Some(warped) { Some((ship.id, false)) }
        else { None }
    }).collect();
    for (id, pursue) in followers {
        let modules: Vec<i64> = park.ships.get(&id).and_then(|s| s.fight.as_ref())
            .map(|f| f.modules.iter().map(|m| m.module_id).collect()).unwrap_or_default();
        for module in modules { crate::fight::stop(park, id, module, now, out, None); }
        park.command(id, Command::Stop);
        if let Some(ship) = park.ships.get_mut(&id) {
            if let Some(fight) = ship.fight.as_mut() { fight.locks.clear(); }
            if let Some(drone) = ship.drone.as_mut() {
                if !pursue { drone.order = Order::Idle; }
                drone.activity = activity::DEPARTING_2;
                drone.transit = Some(Transit { target: warped, arrival_ms: None });
                out.note(drone.client_id, "OnDroneStateChange", Value::Tuple(state_line(id, drone, ship.pilot.type_id)));
            }
        }
    }
}

/// Ordinary drones cannot follow a warp. Keep them out of the distant-space
/// fighter group by returning them to their controller's bay as it leaves.
pub fn recover_nonfighters_for_controller(park: &mut Park, controller: i64, now: i64, out: &mut Out) {
    let ids: Vec<i64> = park.ships.values().filter(|s| {
        s.pilot.group_id != FIGHTER_GROUP
            && s.drone.as_ref().is_some_and(|d| d.controller == Some(controller))
    }).map(|s| s.id).collect();
    for id in ids { dock(park, id, controller, now, out); }
}

/// Also cover a controller moved between grids by a jump or a corrected
/// position, without a normal warp entry notification.
pub fn recover_distant_nonfighters(park: &mut Park, now: i64, out: &mut Out) {
    let distant: Vec<(i64, i64)> = park.ships.values().filter_map(|s| {
        let d = s.drone.as_ref()?;
        if s.pilot.group_id == FIGHTER_GROUP { return None; }
        let controller = d.controller?;
        let at = park.ships.get(&controller)?.pos;
        ((s.pos - at).len() > CONTROLLER_GRID_RANGE).then_some((s.id, controller))
    }).collect();
    for (id, controller) in distant { dock(park, id, controller, now, out); }
}

pub fn on_warp_landed(park: &mut Park, landed: i64, now: i64) {
    for ship in park.ships.values_mut() {
        if let Some(transit) = ship.drone.as_mut().and_then(|d| d.transit.as_mut()) {
            if transit.target == landed { transit.arrival_ms = Some(now + FIGHTER_ARRIVAL_MS); }
        }
    }
}

pub fn arrive_following_fighters(park: &mut Park, now: i64, out: &mut Out) {
    // A pursued player can leave this park entirely. Send those fighters home
    // after the same travel delay instead of dropping their order at a remote
    // position and making them cross the system at sub-warp speed.
    let lost_targets: Vec<i64> = park.ships.values().filter_map(|s| {
        let transit = s.drone.as_ref()?.transit.as_ref()?;
        (!park.ships.contains_key(&transit.target)).then_some(s.id)
    }).collect();
    for id in lost_targets {
        let controller = park.ships.get(&id).and_then(|s| s.drone.as_ref()).and_then(|d| d.controller);
        let home = controller.filter(|c| park.ships.contains_key(c));
        if let Some(drone) = park.ships.get_mut(&id).and_then(|s| s.drone.as_mut()) {
            if let (Some(transit), Some(home)) = (drone.transit.as_mut(), home) {
                if matches!(drone.order, Order::Engage(_)) { drone.order = Order::ReturnHome; }
                transit.target = home;
                transit.arrival_ms = Some(now + FIGHTER_ARRIVAL_MS);
            }
        }
    }
    let arrivals: Vec<(i64, i64)> = park.ships.values().filter_map(|s| {
        let transit = s.drone.as_ref()?.transit.as_ref()?;
        (transit.arrival_ms.is_some_and(|due| due <= now) || !park.ships.contains_key(&transit.target))
            .then_some((s.id, transit.target))
    }).collect();
    for (id, target) in arrivals {
        let at = park.ships.get(&target).map(|s| (s.pos, s.stats.radius));
        if let Some(ship) = park.ships.get_mut(&id) {
            if let Some((pos, radius)) = at {
                ship.pos = pos + Vec3::new(radius + ship.stats.radius + 1_500.0, 0.0, 0.0);
                ship.vel = Vec3::ZERO;
                if ship.drone.as_ref().is_some_and(|d| d.client_id != 0) {
                    out.jobs.push(Job::DronePosition { drone_id: id, pos: ship.pos });
                }
            }
            if let Some(drone) = ship.drone.as_mut() {
                if at.is_none() { drone.order = Order::Idle; }
                drone.transit = None;
                drone.activity = activity::IDLE;
                drone.next_think_ms = now;
                out.note(drone.client_id, "OnDroneStateChange", Value::Tuple(state_line(id, drone, ship.pilot.type_id)));
            }
        }
    }
}

fn think_one(park: &mut Park, id: i64, now: i64, out: &mut Out) {
    let Some(drone) = park.ships.get(&id).and_then(|s| s.drone.clone()) else { return };
    let controller = drone.controller.filter(|c| park.ships.contains_key(c));
    // A bot's drones go with their ship: lost or gone, they leave too.
    if drone.client_id == 0 && controller.is_none() {
        if crate::fight::detach(park, id, now, out).is_some() {
            out.destiny(park.solar_system_id, space::entry(park.tick, "RemoveBalls", vec![Value::List(vec![Value::Int(id)])]));
        }
        return;
    }
    let mut order = drone.order.clone();
    // An order with nothing left to do becomes waiting.
    let gone = match &order {
        Order::Engage(t) => !park.ships.contains_key(t),
        Order::Mine { target, .. } => !park.rocks.contains_key(target),
        Order::ReturnHome | Order::ReturnBay => controller.is_none(),
        Order::Idle => false,
    };
    if gone {
        order = Order::Idle;
    }
    let running = |park: &Park, module: i64| {
        park.ships.get(&id).and_then(|s| s.fight.as_ref()).is_some_and(|f| f.module(module).is_some())
    };
    if let Order::Mine { repeat: false, .. } = order {
        let miner = drone.miner.as_ref().map(|m| m.module_id);
        if drone.mined && miner.is_some_and(|m| !running(park, m)) {
            order = Order::ReturnBay;
        }
    }

    let (activity, course, work): (i64, Option<Command>, Option<i64>) = match &order {
        Order::Idle => (activity::IDLE, controller.map(|c| Command::Orbit { target: c, range: IDLE_ORBIT }), None),
        Order::Engage(t) => (activity::COMBAT, Some(Command::Orbit { target: *t, range: drone.orbit }), Some(*t)),
        Order::Mine { target, .. } => (activity::MINING, Some(Command::Orbit { target: *target, range: drone.orbit.min(500.0) }), Some(*target)),
        Order::ReturnHome => (activity::DEPARTING_2, controller.map(|c| Command::Follow { target: c, range: IDLE_ORBIT }), None),
        Order::ReturnBay => (activity::DEPARTING, controller.map(|c| Command::Follow { target: c, range: 0.0 }), None),
    };

    // Home: into the bay.
    if order == Order::ReturnBay {
        if let Some(c) = controller {
            let reach = park.ships.get(&id).map_or(DOCK_RANGE, |s| DOCK_RANGE.max(s.stats.max_velocity * DOCK_REACH_SECONDS));
            if park.surface_distance(id, c).is_some_and(|d| d <= reach) {
                dock(park, id, c, now, out);
                return;
            }
        }
    }
    if order == Order::ReturnHome {
        if let Some(c) = controller {
            if park.surface_distance(id, c).is_some_and(|d| d <= IDLE_ORBIT * 1.5) {
                order = Order::Idle;
            }
        }
    }

    // Stop what it should no longer do.
    let keep: Vec<i64> = match work {
        Some(_) => drone.weapons.iter().map(|w| w.module_id).chain(drone.miner.iter().map(|m| m.module_id)).collect(),
        None => Vec::new(),
    };
    let stale: Vec<i64> = park
        .ships
        .get(&id)
        .and_then(|s| s.fight.as_ref())
        .map(|f| f.modules.iter().filter(|m| !keep.contains(&m.module_id) || m.target != work).map(|m| m.module_id).collect())
        .unwrap_or_default();
    for m in stale {
        crate::fight::stop(park, id, m, now, out, None);
    }

    let tick = park.tick;
    let system = park.solar_system_id;
    let signature = work.and_then(|t| park.ships.get(&t)).and_then(|s| s.fight.as_ref()).map_or(100.0, |f| f.sig());
    let blocked = work.is_some_and(|t| park.field_blocks_lock(id, t));
    let distance = work.and_then(|t| park.surface_distance(id, t)).unwrap_or(f64::INFINITY);
    let within_reach = |weapon: &Active| {
        !matches!(weapon.action, Action::Turret { .. }) || weapon.reach.is_none_or(|(range, _)| distance <= range)
    };
    let speed = if matches!(order, Order::Engage(_))
        && drone.weapons.iter().any(|w| matches!(w.action, Action::Turret { .. }) && within_reach(w))
    {
        drone.cruise_fraction
    } else {
        1.0
    };
    let Some(ship) = park.ships.get_mut(&id) else { return };
    let Some(fight) = ship.fight.as_mut() else { return };
    match work {
        Some(t) => {
            fight.locks.retain(|l, _| *l == t);
            if !blocked && !fight.locks.contains_key(&t) {
                let ms = (combat::lock_seconds(fight.scan_res(), signature) * 1_000.0) as i64;
                fight.locks.insert(t, Lock::Pending(now + ms));
            }
        }
        None => fight.locks.clear(),
    }
    let mut started = Vec::new();
    if let Some(t) = work.filter(|t| !blocked && fight.is_locked(*t)) {
        let tools: Vec<Active> = match order {
            Order::Mine { repeat, .. } if !drone.mined => {
                drone.miner.iter().cloned().map(|mut m| {
                    m.repeat = if repeat { REPEAT_FOREVER } else { 0 };
                    m
                }).collect()
            }
            Order::Engage(_) => drone.weapons.iter().filter(|w| within_reach(w)).cloned().collect(),
            _ => Vec::new(),
        };
        for mut m in tools {
            if fight.module(m.module_id).is_some() {
                continue;
            }
            m.target = Some(t);
            m.started_ms = now;
            m.next_ms = now;
            started.push(m.clone());
            fight.modules.push(m);
        }
    }
    let Some(d) = ship.drone.as_mut() else { return };
    if !started.is_empty() && matches!(order, Order::Mine { .. }) {
        d.mined = true;
    }
    let activity = if order == Order::Idle { activity::IDLE } else { activity };
    let changed = d.activity != activity;
    let returning = order == Order::ReturnBay;
    d.order = order;
    d.activity = activity;
    d.controller = controller.or(d.controller);
    d.next_think_ms = now + if returning { RETURN_THINK_MS } else { THINK_MS };
    let line = state_line(id, d, ship.pilot.type_id);
    let client = d.client_id;
    if changed {
        out.note(client, "OnDroneStateChange", Value::Tuple(line));
    }
    for m in &started {
        out.destiny(system, crate::fight::module_fx(tick, id, m, true));
    }
    let course = course.unwrap_or(Command::Stop);
    if ship.command != course || ship.speed_fraction != speed {
        park.command(id, course);
        park.set_speed_fraction(id, speed);
        if let Some(s) = park.ships.get(&id) {
            let mut entries = space::course_entries(s, tick);
            if s.command == Command::Stop {
                entries.push(space::command_entry(s, tick));
            }
            space::queue_ship_entries(park, id, entries, out);
        }
    }
}

/// Into the bay: out of space, and the item back where it came from.
fn dock(park: &mut Park, id: i64, controller: i64, now: i64, out: &mut Out) {
    let Some(ship) = crate::fight::detach(park, id, now, out) else { return };
    out.destiny(park.solar_system_id, space::entry(park.tick, "RemoveBalls", vec![Value::List(vec![Value::Int(id)])]));
    let client_id = ship.drone.as_ref().map_or(ship.client_id, |d| d.client_id);
    // A bot's drones are not items: back in the bay is gone from space.
    if client_id != 0 {
        out.jobs.push(Job::DroneHome { client_id, drone_id: id, ship_id: controller });
    }
}

// ---- bots' drones ------------------------------------------------------------------------------

/// How far from its target a bot ship sends its drones.
const BOT_DRONE_RANGE: f64 = 30_000.0;
/// A bot carrier sends its fighters at targets this far off.
const BOT_FIGHTER_RANGE: f64 = 150_000.0;

/// A bot ship's drones: the type in its bay and how many fly at once. They
/// fly as a player's do, with no client and no items behind them.
#[derive(Debug, Clone)]
pub struct BotDrones {
    pub kind: npc::NpcType,
    pub count: usize,
}

/// The drones a bot ship has out.
fn bot_drones_of(park: &Park, controller: i64) -> Vec<i64> {
    park.ships.values().filter(|s| s.drone.as_ref().is_some_and(|d| d.client_id == 0 && d.controller == Some(controller))).map(|s| s.id).collect()
}

/// A bot ship fighting `target` sends its drones when it is close enough,
/// sets them on a new target when it switches, and calls them back to the
/// bay when the fight is over (`None`).
pub fn bot_drones_follow(park: &mut Park, controller: i64, target: Option<i64>, now: i64, out: &mut Out) {
    let Some(ship) = park.ships.get(&controller) else { return };
    let Some(plan) = ship.bot_drones.clone() else { return };
    let (char_id, corp_id, alliance_id, pos, radius) = (ship.pilot.char_id, ship.pilot.corp_id, ship.pilot.alliance_id, ship.pos, ship.stats.radius);
    let flying = bot_drones_of(park, controller);
    let Some(target) = target else {
        for id in flying {
            if let Some(d) = park.ships.get_mut(&id).and_then(|s| s.drone.as_mut()) {
                if d.order != Order::ReturnBay {
                    order_drone(d, &Order::ReturnBay, None, false, now);
                }
            }
        }
        return;
    };
    if !flying.is_empty() {
        for id in flying {
            if let Some(d) = park.ships.get_mut(&id).and_then(|s| s.drone.as_mut()) {
                if d.order != Order::Engage(target) {
                    order_drone(d, &Order::Engage(target), Some(target), false, now);
                }
            }
        }
        return;
    }
    let range = if plan.kind.group_id == FIGHTER_GROUP { BOT_FIGHTER_RANGE } else { BOT_DRONE_RANGE };
    if park.surface_distance(controller, target).is_none_or(|d| d > range) {
        return;
    }
    let system = park.solar_system_id;
    for n in 0..plan.count {
        let id = npc::next_id();
        let angle = n as f64 * std::f64::consts::TAU / plan.count.max(1) as f64;
        let at = pos + Vec3::new(angle.cos(), 0.0, angle.sin()) * (radius + LAUNCH_CLEARANCE);
        let pilot = Pilot {
            type_id: plan.kind.type_id,
            group_id: plan.kind.group_id,
            name: plan.kind.name.clone(),
            char_id,
            corp_id,
            alliance_id,
            security: 0.0,
            bounty: 0.0,
        };
        let mut drone = make(&plan.kind, id, pilot, 0, controller, at, now, &mut park.rng);
        if let Some(d) = drone.drone.as_mut() {
            order_drone(d, &Order::Engage(target), Some(target), false, now);
        }
        park.ships.insert(id, drone);
        for (client, e) in space::add_ball_entries_for_nearby(park, id) {
            out.destiny_to(system, client, e);
        }
    }
}

/// Every drone a bot ship has out, for taking them out of space with it.
pub fn bot_drone_ids(park: &Park, controller: i64) -> Vec<i64> {
    bot_drones_of(park, controller)
}

/// ship.ScoopDrone(ids): drones close to the ship go straight into the bay.
pub async fn scoop(ctx: &Ctx<'_>, ids: Vec<i64>) -> Outcome {
    let (Some(char_id), Some(ship_id), Some(system)) = (ctx.session.char_id, ctx.session.ship_id, ctx.session.solar_system_id) else {
        return Outcome::custom("You are not in space.");
    };
    let mut scooped = Vec::new();
    {
        let mut world = ctx.world.lock();
        let Some(park) = world.parks.get_mut(&system) else { return Outcome::custom("You are not in space.") };
        let mut out = Out::default();
        for id in ids {
            let mine = park.ships.get(&id).and_then(|s| s.drone.as_ref()).is_some_and(|d| d.owner == char_id);
            let near = park.surface_distance(id, ship_id).is_some_and(|d| d <= SCOOP_RANGE * 1.25);
            if mine && near && crate::fight::detach(park, id, combat::now_ms(), &mut out).is_some() {
                out.destiny(system, space::entry(park.tick, "RemoveBalls", vec![Value::List(vec![Value::Int(id)])]));
                scooped.push(id);
            }
        }
        out.deliver(&world);
    }
    if scooped.is_empty() {
        return Outcome::custom("There is no drone of yours close enough to scoop.");
    }
    let mut client = match crate::db::connect(ctx.db_url).await {
        Ok(c) => c,
        Err(e) => {
            error!("scooping drones: {}", crate::logging::chain(&e));
            return Outcome::custom("Your drones could not be scooped.");
        }
    };
    for id in scooped {
        into_bay(&mut client, ctx.world, ctx.client_id, id, ship_id).await;
    }
    Outcome::Value(Value::None)
}

pub async fn home(server: &Server, db: &mut Client, client_id: i64, drone_id: i64, ship_id: i64) {
    into_bay(db, &server.world, client_id, drone_id, ship_id).await;
}

async fn into_bay(db: &mut Client, world: &World, client_id: i64, drone_id: i64, ship_id: i64) {
    let result = async {
        let mut txn = ItemTxn::begin(db).await?;
        txn.move_to(drone_id, ship_id, FLAG_DRONE_BAY).await?;
        txn.commit().await
    }
    .await;
    match result {
        Ok(events) => {
            items::notify(world, client_id, events);
            debug!("drone {drone_id} back in {ship_id}");
        }
        Err(e) => error!("returning drone {drone_id} to {ship_id}: {}", crate::logging::chain(&e)),
    }
}

pub async fn lost(server: &Server, db: &mut Client, client_id: Option<i64>, drone_id: i64) {
    let result = async {
        let mut txn = ItemTxn::begin(db).await?;
        txn.destroy(drone_id).await?;
        txn.commit().await
    }
    .await;
    match result {
        Ok(events) => {
            if let Some(client) = client_id {
                items::notify(&server.world, client, events);
            }
        }
        Err(e) => warn!("destroying drone {drone_id}: {}", crate::logging::chain(&e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordinary_drone_returns_to_bay_while_fighter_follows_warp() {
        let mut park = Park::new(30000001, Vec::new(), 0);
        let stats = ShipStats::new(50.0, 1_000_000.0, 1.0, 300.0, 1.0);
        let pilot = |type_id, group_id| Pilot { type_id, group_id, name: "Drone".into(),
            char_id: 200, corp_id: 2, alliance_id: None, security: 0.0, bounty: 0.0 };
        park.ships.insert(30, Ship::new(30, 2, stats, pilot(587, 25), Vec3::ZERO, Vec3::ZERO));
        for (id, group) in [(20, 100), (21, FIGHTER_GROUP)] {
            let mut ship = Ship::new(id, 2, stats, pilot(23055, group), Vec3::new(1_000.0, 0.0, 0.0), Vec3::ZERO);
            ship.drone = Some(Drone { owner: 200, client_id: 2, controller: Some(30), order: Order::Idle,
                activity: activity::IDLE, weapons: Vec::new(), miner: None, mined: false,
                orbit: 1_000.0, cruise_fraction: 1.0, next_think_ms: 0, transit: None });
            park.ships.insert(id, ship);
        }
        let mut out = Out::default();
        on_warp_entered(&mut park, 30, 0, &mut out);
        assert!(!park.ships.contains_key(&20));
        assert!(out.jobs.iter().any(|job| matches!(job, Job::DroneHome { drone_id: 20, ship_id: 30, .. })));
        assert!(in_transit(&park.ships[&21]));
    }

    #[test]
    fn fighter_reaches_warping_enemy_after_a_minute_and_keeps_engaging() {
        let mut park = Park::new(30000001, Vec::new(), 0);
        let stats = ShipStats::new(50.0, 1_000_000.0, 1.0, 300.0, 1.0);
        let pilot = Pilot { type_id: 587, group_id: 25, name: "Rifter".into(), char_id: 100, corp_id: 1, alliance_id: None, security: 0.0, bounty: 0.0 };
        park.ships.insert(10, Ship::new(10, 1, stats, pilot, Vec3::ZERO, Vec3::ZERO));
        let fighter_pilot = Pilot { type_id: 23057, group_id: FIGHTER_GROUP, name: "Einherji".into(), char_id: 200, corp_id: 2, alliance_id: None, security: 0.0, bounty: 0.0 };
        let mut fighter = Ship::new(20, 2, stats, fighter_pilot, Vec3::new(1_000.0, 0.0, 0.0), Vec3::ZERO);
        fighter.drone = Some(Drone { owner: 200, client_id: 2, controller: Some(30), order: Order::Engage(10), activity: activity::COMBAT,
            weapons: Vec::new(), miner: None, mined: false, orbit: 1_000.0, cruise_fraction: 1.0,
            next_think_ms: 0, transit: None });
        park.ships.insert(20, fighter);
        let mut out = Out::default();
        on_warp_entered(&mut park, 10, 0, &mut out);
        assert!(in_transit(&park.ships[&20]));
        park.ships.get_mut(&10).unwrap().pos = Vec3::new(1.0e12, 0.0, 0.0);
        on_warp_landed(&mut park, 10, 1_000);
        arrive_following_fighters(&mut park, 60_999, &mut out);
        assert!(in_transit(&park.ships[&20]));
        arrive_following_fighters(&mut park, 61_000, &mut out);
        assert!(!in_transit(&park.ships[&20]));
        assert_eq!(park.ships[&20].drone.as_ref().unwrap().order, Order::Engage(10));
        assert!((park.ships[&20].pos - park.ships[&10].pos).len() < 2_000.0);
        assert!(out.jobs.iter().any(|j| matches!(j, Job::DronePosition { drone_id: 20, .. })));
    }

    #[test]
    fn return_order_redirects_a_fighter_pursuing_a_warping_player() {
        let mut park = Park::new(30000001, Vec::new(), 0);
        let stats = ShipStats::new(50.0, 1_000_000.0, 1.0, 300.0, 1.0);
        let pilot = |char_id| Pilot { type_id: 587, group_id: 25, name: "Pilot".into(), char_id,
            corp_id: char_id, alliance_id: None, security: 0.0, bounty: 0.0 };
        park.ships.insert(10, Ship::new(10, 1, stats, pilot(100), Vec3::ZERO, Vec3::ZERO));
        park.ships.insert(30, Ship::new(30, 2, stats, pilot(200), Vec3::new(10_000.0, 0.0, 0.0), Vec3::ZERO));
        let mut fighter = Ship::new(20, 2, stats, Pilot { type_id: 23057, group_id: FIGHTER_GROUP,
            name: "Dragonfly".into(), ..pilot(200) }, Vec3::new(1_000.0, 0.0, 0.0), Vec3::ZERO);
        fighter.drone = Some(Drone { owner: 200, client_id: 2, controller: Some(30), order: Order::Engage(10),
            activity: activity::COMBAT, weapons: Vec::new(), miner: None, mined: false, orbit: 1_000.0,
            cruise_fraction: 1.0, next_think_ms: 0, transit: None });
        park.ships.insert(20, fighter);
        let mut out = Out::default();
        on_warp_entered(&mut park, 10, 0, &mut out);
        order_drone(park.ships.get_mut(&20).unwrap().drone.as_mut().unwrap(), &Order::ReturnBay,
            Some(30), false, 1_000);
        park.ships.remove(&10);
        arrive_following_fighters(&mut park, 60_999, &mut out);
        assert!(in_transit(&park.ships[&20]));
        arrive_following_fighters(&mut park, 61_000, &mut out);
        assert!(!in_transit(&park.ships[&20]));
        assert_eq!(park.ships[&20].drone.as_ref().unwrap().order, Order::ReturnBay);
        assert!(park.surface_distance(20, 30).unwrap() <= DOCK_RANGE);
        think(&mut park, 61_000, &mut out);
        assert!(!park.ships.contains_key(&20));
    }

    #[test]
    fn fighter_returns_home_when_its_warp_target_disappears() {
        let mut park = Park::new(30000001, Vec::new(), 0);
        let stats = ShipStats::new(50.0, 1_000_000.0, 1.0, 300.0, 1.0);
        let pilot = Pilot { type_id: 587, group_id: 25, name: "Owner".into(), char_id: 200,
            corp_id: 2, alliance_id: None, security: 0.0, bounty: 0.0 };
        park.ships.insert(30, Ship::new(30, 2, stats, pilot, Vec3::new(10_000.0, 0.0, 0.0), Vec3::ZERO));
        let fighter_pilot = Pilot { type_id: 23057, group_id: FIGHTER_GROUP, name: "Dragonfly".into(),
            char_id: 200, corp_id: 2, alliance_id: None, security: 0.0, bounty: 0.0 };
        let mut fighter = Ship::new(20, 2, stats, fighter_pilot, Vec3::ZERO, Vec3::ZERO);
        fighter.drone = Some(Drone { owner: 200, client_id: 2, controller: Some(30), order: Order::Engage(10),
            activity: activity::DEPARTING_2, weapons: Vec::new(), miner: None, mined: false, orbit: 1_000.0,
            cruise_fraction: 1.0, next_think_ms: 0, transit: Some(Transit { target: 10, arrival_ms: None }) });
        park.ships.insert(20, fighter);
        let mut out = Out::default();
        arrive_following_fighters(&mut park, 1_000, &mut out);
        assert!(in_transit(&park.ships[&20]));
        assert_eq!(park.ships[&20].drone.as_ref().unwrap().order, Order::ReturnHome);
        arrive_following_fighters(&mut park, 61_000, &mut out);
        assert!(!in_transit(&park.ships[&20]));
        assert!(park.surface_distance(20, 30).unwrap() <= DOCK_RANGE);
    }

    #[test]
    fn light_and_medium_drones_close_range_then_fire_and_report_hits() {
        for (type_id, mass, radius, top, cruise, orbit, falloff, tracking, sig, damage) in [
            (2205, 3_000.0, 15.0, 4_560.0, 780.0, 500.0, 2_000.0, 2.964, 25.0, 10.35),
            (2185, 5_000.0, 20.0, 1_680.0, 480.0, 750.0, 3_000.0, 0.768, 60.0, 23.04),
        ] {
            let mut park = Park::new(30000001, Vec::new(), 0);
            let pilot = |type_id, group_id, char_id| Pilot {
                type_id, group_id, char_id, name: "Pilot".into(), corp_id: char_id,
                alliance_id: None, security: 0.0, bounty: 0.0,
            };
            let stats = ShipStats::new(50.0, 1_000_000.0, 1.0, 300.0, 1.0);
            let target_type = npc::NpcType::with(587, 25, 50.0, 1_000_000.0, &[
                (attr::SHIELD_CAPACITY, 10_000.0), (attr::ARMOR_HP, 1_000.0), (attr::HP, 1_000.0),
                (attr::SIGNATURE_RADIUS, 40.0),
            ]);
            let mut target = Ship::new(10, 1, stats, pilot(587, 25, 100), Vec3::ZERO, Vec3::ZERO);
            target.fight = Some(npc::entity_fighter(&target_type, 60_000.0, 1, 300.0, 0));
            park.ships.insert(10, target);
            let launch_pos = Vec3::new(49_000.0, 0.0, 0.0);
            park.ships.insert(30, Ship::new(30, 2, stats, pilot(587, 25, 200), launch_pos, Vec3::ZERO));
            let drone_type = npc::NpcType::with(type_id, COMBAT_DRONE_GROUP, radius, mass, &[
                (attr::MAX_VELOCITY, top), (attr::AGILITY, 100.0), (attr::ENTITY_CRUISE_SPEED, cruise),
                (attr::ENTITY_FLY_RANGE, orbit), (attr::ENTITY_ATTACK_RANGE, 1_000.0 + falloff),
                (attr::DAMAGE_MULTIPLIER, 1.0), (attr::SPEED, 2_000.0), (attr::THERMAL_DAMAGE, damage),
                (attr::MAX_RANGE, 1_000.0), (attr::FALLOFF, falloff), (attr::TRACKING_SPEED, tracking),
                (attr::OPTIMAL_SIG_RADIUS, sig),
            ]);
            let mut drone = make(&drone_type, 20, pilot(type_id, COMBAT_DRONE_GROUP, 200),
                2, 30, launch_pos, 0, &mut park.rng);
            order_drone(drone.drone.as_mut().unwrap(), &Order::Engage(10), Some(10), false, 0);
            park.ships.insert(20, drone);

            let mut out = Out::default();
            let mut locked_promptly = false;
            let mut fired = false;
            let mut reported_hit = false;
            for now in (0..60_000).step_by(250) {
                park.advance(now / 1_000);
                crate::fight::step(&mut park, now, &mut out);
                if now <= 6_000 {
                    locked_promptly |= park.ships[&20].fight.as_ref().unwrap().is_locked(10);
                }
                if !fired && out.destiny_entries().any(|entry| {
                    let event = &entry.seq().unwrap()[1].seq().unwrap();
                    event[0] == Value::bytes("OnSpecialFX")
                }) {
                    assert!(park.surface_distance(20, 10).unwrap() <= 1_000.0 + falloff);
                    assert_eq!(park.ships[&20].speed_fraction, cruise / top);
                    fired = true;
                }
                reported_hit |= out.notes().iter().any(|(client, event, args)| {
                    *client == 2 && *event == "OnDamageMessage"
                        && args.seq().unwrap()[0].as_bytes().is_some_and(|key| key.starts_with(b"AttackHit"))
                });
                out = Out::default();
            }
            assert!(locked_promptly, "drone {type_id} should lock a frigate-sized target within six seconds");
            assert!(fired && reported_hit, "drone {type_id} should fire and report a hit");
            assert!(park.ships[&10].fight.as_ref().unwrap().vitals.shield.at(60_000) < 10_000.0);
        }
    }

    #[test]
    fn ordered_fighter_locks_and_damages_a_player_ship() {
        let mut park = Park::new(30000001, Vec::new(), 0);
        let stats = ShipStats::new(50.0, 1_000_000.0, 1.0, 300.0, 1.0);
        let pilot = |char_id| Pilot { type_id: 587, group_id: 25, name: "Pilot".into(), char_id,
            corp_id: char_id, alliance_id: None, security: 0.0, bounty: 0.0 };
        let victim_type = npc::NpcType::with(587, 25, 50.0, 1_000_000.0, &[
            (attr::SHIELD_CAPACITY, 1_000.0), (attr::ARMOR_HP, 1_000.0), (attr::HP, 1_000.0),
            (attr::SIGNATURE_RADIUS, 500.0),
        ]);
        let mut victim = Ship::new(10, 1, stats, pilot(100), Vec3::ZERO, Vec3::ZERO);
        victim.fight = Some(npc::entity_fighter(&victim_type, 60_000.0, 1, 300.0, 0));
        park.ships.insert(10, victim);
        park.ships.insert(30, Ship::new(30, 2, stats, pilot(200), Vec3::new(2_000.0, 0.0, 0.0), Vec3::ZERO));
        let fighter_type = npc::NpcType::with(23055, FIGHTER_GROUP, 20.0, 10_000.0, &[
            (attr::MAX_VELOCITY, 500.0), (attr::SHIELD_CAPACITY, 500.0), (attr::HP, 500.0),
            (attr::SCAN_RESOLUTION, 1_000.0), (attr::SIGNATURE_RADIUS, 125.0),
            (attr::DAMAGE_MULTIPLIER, 2.0), (attr::SPEED, 2_000.0),
            (attr::EM_DAMAGE, 100.0), (attr::MAX_RANGE, 5_000.0),
            (attr::TRACKING_SPEED, 1.0), (attr::OPTIMAL_SIG_RADIUS, 125.0),
        ]);
        let mut rng = combat::Rng::seeded(1);
        let mut fighter = make(&fighter_type, 20, Pilot { type_id: 23055, group_id: FIGHTER_GROUP,
            name: "Templar".into(), ..pilot(200) }, 2, 30, Vec3::new(1_000.0, 0.0, 0.0), 0, &mut rng);
        assert_eq!(fighter.drone.as_ref().unwrap().weapons.len(), 1);
        order_drone(fighter.drone.as_mut().unwrap(), &Order::Engage(10), Some(10), false, 0);
        park.ships.insert(20, fighter);
        let mut out = Out::default();
        think(&mut park, 0, &mut out);
        assert_eq!(park.ships[&20].drone.as_ref().unwrap().activity, activity::COMBAT);
        crate::fight::step(&mut park, 60_000, &mut out);
        crate::fight::step(&mut park, 61_000, &mut out);
        assert!(park.ships[&10].fight.as_ref().unwrap().vitals.shield.at(61_000) < 1_000.0);
    }
}
