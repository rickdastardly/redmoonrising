//! Calls that move the session: entering the game, undocking, docking and
//! jumping. Each also moves the ship into or out of its system's ballpark, so
//! the other players there see it arrive and leave.

use std::time::Duration;

use eve_marshal::Value;
use eve_proto::MachoPacket;
use tracing::{error, info, warn};

use super::{Conn, Result};
use crate::ballpark::{Command, Park, Vec3};
use crate::destiny;
use crate::exceptions::{custom_error, user_error};
use crate::services::Call;
use crate::svc::inventory::FLAG_HANGAR;
use crate::svc::{bound, inflight, space};
use crate::world::{Move, Notification};

/// const.maxStargateJumpingDistance.
const JUMP_RANGE: f64 = 2_500.0;
/// How long the gate's flash and the ship's jump-out play before the ship
/// is gone. Their length is in the effects' models; this is a choice.
const JUMP_OUT_TIME: Duration = Duration::from_secs(3);
/// Maximum distance between the ship and station surfaces for regular docking.
const DOCK_RANGE: f64 = 1_000.0;
/// Dock may be called directly from outside the station's grid.
const DOCK_WARP_DISTANCE: f64 = 1_000_000.0;
/// Time allowed to finish an active warp before reporting a failed warp.
const DOCK_WARP_LIMIT: i64 = 600;

/// Keep the Dock RPC open while the connection continues to deliver flight
/// updates and receive commands.
pub(super) struct PendingDock {
    request: MachoPacket,
    system_id: i64,
    ship_id: i64,
    station_id: i64,
    warp_deadline: i64,
    approaching: bool,
}

#[derive(Debug, PartialEq)]
enum DockStep {
    Wait,
    Approach { gap: f64 },
    Warp,
    Arrived,
}

impl PendingDock {
    fn continues_toward_station(&self, park: &Park, call: &Call) -> bool {
        if call.method == "FollowBall" {
            return call.arg_i64(0) == Some(self.station_id)
                && call.args.get(1).and_then(Value::as_f64).is_none_or(|range| range <= DOCK_RANGE);
        }
        let Some(ship) = park.ships.get(&self.ship_id) else { return false };
        let Some((station, radius)) = park.ball(self.station_id) else { return false };
        let component = |i: usize| call.args.get(i).and_then(Value::as_f64);
        let (Some(x), Some(y), Some(z)) = (component(0), component(1), component(2)) else { return false };
        let point = Vec3::new(x, y, z);
        let direction = match call.method.as_str() {
            "GotoDirection" => point.unit(),
            "GotoPoint" => (point - ship.pos).unit(),
            _ => return false,
        };
        let to_station = station - ship.pos;
        let along = to_station.dot(direction);
        let reach = radius + ship.stats.radius + DOCK_RANGE;
        along > 0.0 && (to_station - direction * along).len_sq() <= reach * reach
    }

    fn step(&mut self, park: &mut Park, towed: bool) -> std::result::Result<DockStep, &'static str> {
        if !space::is_station(park, self.station_id) {
            return Err("That station is not in this solar system.");
        }
        let ship = park.ships.get(&self.ship_id).ok_or("Your ship is no longer in space.")?;
        if towed {
            return Ok(DockStep::Arrived);
        }
        // Let the heartbeat send the warp exit and reset the client's
        // ballpark before sending a sublight command.
        if ship.warping() || park.landed.contains(&self.ship_id) {
            return if park.tick < self.warp_deadline {
                Ok(DockStep::Wait)
            } else {
                Err("Your ship did not finish warping to the station.")
            };
        }

        let (at, radius) = park.ball(self.station_id).ok_or("The station is unavailable.")?;
        let offset = ship.pos - at;
        let gap = (offset.len() - radius - ship.stats.radius).max(0.0);
        if gap <= DOCK_RANGE {
            return Ok(DockStep::Arrived);
        }
        if self.approaching {
            return Ok(DockStep::Wait);
        }

        if gap > DOCK_WARP_DISTANCE {
            if park.warp_disruptor_at(ship.pos).is_some() {
                return Err("Your ship cannot warp to the station from inside a warp disruption field.");
            }
            if ship.fight.as_ref().is_some_and(|fight| fight.afflictions.scrambled >= 1.0) {
                return Err("Your ship cannot warp to the station while warp scrambled.");
            }
            let target = space::station_warp_point(ship, at, radius, 0.0);
            park.set_speed_fraction(self.ship_id, 1.0);
            park.command(self.ship_id, Command::Warp { target, range: 0.0 });
            self.warp_deadline = park.tick + DOCK_WARP_LIMIT;
            return Ok(DockStep::Warp);
        }

        // Aim the ship's centre at the near side of the station hull. A
        // FollowBall goal at the docking perimeter eases thrust much too
        // early on large hulls. The range check intercepts this course at 1 km.
        let course = Command::Point(at + offset.unit() * radius);
        park.set_speed_fraction(self.ship_id, 1.0);
        park.command(self.ship_id, course);
        self.approaching = true;
        Ok(DockStep::Approach { gap })
    }
}

impl Conn {
    pub(super) async fn select_character(&mut self, req: &MachoPacket, char_id: i64) -> Result {
        let before = self.session.clone();

        let sql = r#"SELECT c."characterID", c."stationID", c."solarSystemID",
                            c."constellationID", c."regionID", c."shipID",
                            c."characterName", c."typeID"
                       FROM "character_" c
                      WHERE c."characterID" = $1 AND c."accountID" = $2"#;
        let user_id = self.session.user_id.unwrap_or(0);
        let rows = match self.server.db.query(sql, &[&(char_id as i32), &(user_id as i32)]).await {
            Ok(r) => r,
            Err(e) => {
                error!("loading character {char_id}: {}", crate::logging::chain(&e));
                let err = custom_error("That character could not be loaded.");
                return self.send_error(req, err).await;
            }
        };
        let Some(row) = rows.first() else {
            warn!("character {char_id} is not on account {user_id}");
            let err = custom_error("That character is not on this account.");
            return self.send_error(req, err).await;
        };

        let opt = |i: usize| crate::svc::rows::int(row, i);
        self.session.char_id = Some(char_id);
        self.session.char_name = crate::svc::rows::text(row, 6).unwrap_or_default();
        self.session.char_type_id = opt(7).unwrap_or(0);
        self.session.station_id = opt(1);
        self.session.solar_system_id = opt(2);
        self.session.constellation_id = opt(3);
        self.session.region_id = opt(4);
        self.session.solar_system_id2 = opt(2);
        if self.session.station_id.is_some() {
            self.session.solar_system_id = None;
        }
        let ship = opt(5);
        self.session.ship_id = match self.session.station_id {
            Some(station) => self.resolve_active_ship(char_id, station).await,
            None => ship,
        };
        self.load_affiliation(char_id).await;
        self.span.record("char", char_id);

        self.send_session_change_from(&before).await?;
        info!(
            corp = ?self.session.corp_id,
            station = ?self.session.station_id,
            system = ?self.session.solar_system_id2,
            ship = ?self.session.ship_id,
            "entered the game as {:?} ({char_id})",
            self.session.char_name
        );
        self.reply(req, &Value::None).await?;
        crate::web::notify_joined(&self.server, self.client_id).await;
        Ok(())
    }

    pub(super) async fn undock(&mut self, req: &MachoPacket, bind_params: Option<Vec<i64>>) -> Result {
        let (Some(station_id), Some(system_id)) =
            (self.session.station_id, self.session.solar_system_id2)
        else {
            warn!("undock while not docked");
            return self.send_error(req, custom_error("You are not docked.")).await;
        };
        let Some(ship_id) = self.session.ship_id else {
            warn!("undock with no active ship");
            let err = custom_error("You have no active ship to undock in.");
            return self.send_error(req, err).await;
        };

        let char_id = self.session.char_id.unwrap_or(0);
        let (loaded, exit) = {
            let ctx = self.ctx();
            space::ensure_park(&ctx, system_id).await;
            let loaded = inflight::ship_for_space(&ctx, char_id, ship_id).await;
            let exit = match &loaded {
                Some(ship) => space::undock_point(&ctx, station_id, ship.stats.radius).await,
                None => None,
            };
            (loaded, exit)
        };
        let (Some(mut ship), Some((pos, heading))) = (loaded, exit) else {
            error!("cannot undock ship {ship_id} from {station_id}: ship or station data missing");
            return self.send_error(req, custom_error("Your ship could not be undocked.")).await;
        };
        // Out of the bay at full speed, coasting to a stop clear of the station.
        ship.pos = pos;
        ship.vel = heading * ship.stats.max_velocity;

        if let Some(char_id) = self.session.char_id {
            crate::svc::pvptrade::left_station(&self.server.db_url, &self.server.world, char_id).await;
        }
        let before = self.session.clone();
        self.session.station_id = None;
        self.session.solar_system_id = Some(system_id);
        self.session.solar_system_id2 = Some(system_id);
        self.send_session_change_from(&before).await?;
        space::add_ship(&mut self.server.world.lock(), system_id, ship);
        info!("undocked from station {station_id} into system {system_id} in ship {ship_id}");

        let retval = match bind_params {
            Some(params) => bound::bind_result(self.server.node_id, "ship", &params, Value::None),
            None => Value::None,
        };
        self.reply(req, &retval).await
    }

    /// ship.Board(shipID) while docked: make another of the character's
    /// assembled ships in this station the active one.
    pub(super) async fn board(&mut self, req: &MachoPacket, ship_id: i64, bind_params: Option<Vec<i64>>) -> Result {
        let (Some(char_id), Some(station_id)) = (self.session.char_id, self.session.station_id) else {
            return self.send_error(req, custom_error("Ships can only be boarded in a station.")).await;
        };
        const SQL: &str = r#"
            UPDATE "character_" SET "shipID" = $2
             WHERE "characterID" = $1
               AND EXISTS (SELECT 1 FROM entity e
                             JOIN "invTypes" t ON t."typeID" = e."typeID"
                             JOIN "invGroups" g ON g."groupID" = t."groupID"
                            WHERE e."itemID" = $2 AND e."ownerID" = $1 AND e."locationID" = $3
                              AND e."singleton" = 1 AND g."categoryID" = 6)
            RETURNING "shipID""#;
        let params: [&(dyn tokio_postgres::types::ToSql + Sync); 3] =
            [&(char_id as i32), &(ship_id as i32), &(station_id as i32)];
        match self.server.db.query_opt(SQL, &params).await {
            Ok(Some(_)) => {}
            Ok(None) => {
                warn!("board {ship_id}: not an assembled ship of {char_id} in {station_id}");
                return self.send_error(req, custom_error("You cannot board that ship.")).await;
            }
            Err(e) => {
                error!("boarding {ship_id}: {}", crate::logging::chain(&e));
                return self.send_error(req, custom_error("The ship could not be boarded.")).await;
            }
        }
        let before = self.session.clone();
        self.session.ship_id = Some(ship_id);
        self.send_session_change_from(&before).await?;
        info!("boarded ship {ship_id}");
        let retval = match bind_params {
            Some(params) => bound::bind_result(self.server.node_id, "ship", &params, Value::None),
            None => Value::None,
        };
        self.reply(req, &retval).await
    }

    pub(super) async fn dock(&mut self, req: &MachoPacket, station_id: i64, towed: bool) -> Result {
        let (Some(system_id), Some(ship_id)) = (self.session.solar_system_id, self.session.ship_id) else {
            return self.refuse_dock(req, "You are not in space.").await;
        };
        if let Some(pending) = &self.pending_dock {
            if pending.station_id == station_id && !towed {
                return self.reply(req, &Value::None).await;
            }
            return self.refuse_dock(req, "A docking request is already in progress.").await;
        }
        // The owner's Station Management docking rule (outposts).
        if !towed {
            if let Some(char_id) = self.session.char_id {
                if let Some(why) = crate::svc::outpost::docking_refusal(&self.server.db, station_id, char_id).await {
                    return self.refuse_dock(req, &why).await;
                }
            }
        }
        let pending = PendingDock {
            request: req.clone(),
            system_id,
            ship_id,
            station_id,
            warp_deadline: destiny::tick_now() + DOCK_WARP_LIMIT,
            approaching: false,
        };
        self.advance_dock(pending, towed).await
    }

    pub(super) async fn poll_dock(&mut self) -> Result {
        if let Some(pending) = self.pending_dock.take() {
            self.advance_dock(pending, false).await?;
        }
        Ok(())
    }

    pub(super) async fn handle_dock_movement(&mut self, call: &Call) -> Result<bool> {
        let Some(pending) = &self.pending_dock else { return Ok(false) };
        let keep_course = {
            let mut world = self.server.world.lock();
            world.parks.get_mut(&pending.system_id).is_some_and(|park| {
                park.advance(destiny::tick_now());
                park.ships.get(&pending.ship_id).is_some_and(|ship| ship.in_warp())
                    || pending.continues_toward_station(park, call)
            })
        };
        // A warp ignores new courses, and a manual click toward the same
        // station is already covered by the queued approach.
        if keep_course {
            return Ok(true);
        }
        if call.method != "SetSpeedFraction"
            || call.args.first().and_then(Value::as_f64).is_some_and(|speed| speed <= 0.0)
        {
            self.cancel_dock("Docking cancelled by a new flight command.").await?;
        }
        Ok(false)
    }

    pub(super) async fn cancel_dock(&mut self, reason: &str) -> Result {
        if let Some(pending) = self.pending_dock.take() {
            self.refuse_dock(&pending.request, reason).await?;
        }
        Ok(())
    }

    async fn refuse_dock(&mut self, req: &MachoPacket, reason: &str) -> Result {
        self.send_error(req, user_error("DockingRequestDenied", &[("reason", Value::bytes(reason))])).await
    }

    async fn advance_dock(&mut self, mut pending: PendingDock, towed: bool) -> Result {
        let (system_id, ship_id, station_id) = (pending.system_id, pending.ship_id, pending.station_id);
        if self.session.solar_system_id != Some(system_id) || self.session.ship_id != Some(ship_id) {
            return self.refuse_dock(&pending.request, "Your ship has left the docking approach.").await;
        }
        // Check range and take the ship out of space under the same world lock.
        let result = (|| {
            let mut world = self.server.world.lock();
            let park = world.parks.get_mut(&system_id).ok_or("You are not in space.")?;
            park.advance(destiny::tick_now());
            match pending.step(park, towed)? {
                DockStep::Wait => Ok(None),
                step @ (DockStep::Approach { .. } | DockStep::Warp) => {
                    let ship = &park.ships[&ship_id];
                    let course = space::course_entries(ship, park.tick);
                    let mut viewers = space::ship_viewers(park, ship_id);
                    if step == DockStep::Warp {
                        viewers.retain(|&client| client != self.client_id);
                        let mut own_course = Vec::from(space::ship_motion_entries(ship, park.tick));
                        own_course.extend(course.iter().cloned());
                        space::broadcast(&world, system_id, Some(self.client_id), own_course);
                    } else if !viewers.contains(&self.client_id) {
                        viewers.push(self.client_id);
                    }
                    world.notify(&viewers, "DoDestinyUpdate", Value::Tuple(vec![Value::List(course), Value::Int(0)]));
                    if let DockStep::Approach { gap } = step {
                        let message = Value::Tuple(vec![Value::bytes("DockingApproach"), Value::Dict(Vec::new())]);
                        world.notify(&[self.client_id], "OnRemoteMessage", message);
                        info!("approaching station {station_id} from {gap:.0} m");
                    }
                    Ok(None)
                }
                DockStep::Arrived => {
                    let start = park.ships[&ship_id].pos;
                    let end = park.ball(station_id).map_or(start, |(pos, _)| pos);
                    let ship = space::remove_ship(&mut world, system_id, ship_id)
                        .ok_or("Your ship is no longer in space.")?;
                    Ok(Some((start, end, ship)))
                }
            }
        })();
        let (start, end, ship) = match result {
            Ok(Some(arrival)) => arrival,
            Ok(None) => {
                self.pending_dock = Some(pending);
                return Ok(());
            }
            Err(reason) => return self.refuse_dock(&pending.request, reason).await,
        };
        if let Some(fight) = &ship.fight {
            inflight::save_vitals(&self.server.db, ship_id, fight, true).await;
        }

        let before = self.session.clone();
        self.session.station_id = Some(station_id);
        self.session.solar_system_id = None;
        self.session.solar_system_id2 = Some(system_id);
        if let Some(char_id) = self.session.char_id {
            self.session.ship_id = self.store_docked(char_id, ship_id, station_id).await;
        }
        let point = |v: Vec3| Value::Tuple(vec![Value::Double(v.x), Value::Double(v.y), Value::Double(v.z)]);
        self.send_notification(&Notification {
            event: "OnDockingAccepted",
            args: Value::Tuple(vec![point(start), point(end), Value::Int(station_id)]),
        }).await?;
        self.send_session_change_from(&before).await?;

        info!(ship = ?self.session.ship_id, "docked at station {station_id}");
        self.reply(&pending.request, &Value::None).await
    }

    pub(super) async fn stargate_jump(&mut self, req: &MachoPacket, from_gate: i64, to_gate: i64) -> Result {
        let (Some(system_id), Some(ship_id)) = (self.session.solar_system_id, self.session.ship_id) else {
            warn!("stargate jump while not in space in a ship");
            return self.send_error(req, custom_error("You are not in space.")).await;
        };

        let gap = {
            let mut world = self.server.world.lock();
            world.parks.get_mut(&system_id).and_then(|park| {
                park.advance(destiny::tick_now());
                park.surface_distance(ship_id, from_gate)
            })
        };
        match gap {
            Some(gap) if gap <= JUMP_RANGE => {}
            Some(gap) => {
                info!("jump refused: {gap:.0} m from gate {from_gate}");
                let maxdist = [("maxdist", Value::Int(JUMP_RANGE as i64))];
                return self.send_error(req, user_error("NotCloseEnoughToJump", &maxdist)).await;
            }
            None => {
                warn!("jump: ship {ship_id} or gate {from_gate} is not in the ballpark of {system_id}");
                return self.send_error(req, custom_error("You are not in space.")).await;
            }
        }

        let jump = {
            let ctx = self.ctx();
            match space::resolve_jump(&ctx, system_id, from_gate, to_gate).await {
                Some(jump) => {
                    space::ensure_park(&ctx, jump.solar_system_id).await;
                    Some(jump)
                }
                None => None,
            }
        };
        let Some(jump) = jump else {
            warn!("gate {from_gate} -> {to_gate} is not a jump from system {system_id}");
            let err = custom_error("That stargate does not lead there.");
            return self.send_error(req, err).await;
        };

        // Leave: the gate lights up and the ship jumps out, for everyone here
        // and for the pilot ("Jumping", spaceMgr.py:123); it goes once that
        // has played.
        {
            let world = self.server.world.lock();
            let tick = world.parks.get(&system_id).map_or(destiny::tick_now(), |p| p.tick);
            let gate = space::special_fx(tick, from_gate, None, "effects.GateActivity");
            let out = space::special_fx(tick, ship_id, Some(from_gate), "effects.JumpOut");
            space::broadcast(&world, system_id, None, vec![gate, out]);
        }
        tokio::time::sleep(JUMP_OUT_TIME).await;
        let ship = space::remove_ship(&mut self.server.world.lock(), system_id, ship_id);
        let Some(ship) = ship else {
            warn!("jump: ship {ship_id} was lost before it could jump");
            return self.send_error(req, custom_error("Your ship is no longer in space.")).await;
        };
        let ship = Some(ship);

        let before = self.session.clone();
        self.session.solar_system_id = Some(jump.solar_system_id);
        self.session.solar_system_id2 = Some(jump.solar_system_id);
        self.session.constellation_id = Some(jump.constellation_id);
        self.session.region_id = Some(jump.region_id);
        self.send_session_change_from(&before).await?;

        // Arrive: out of the destination gate, standing still, with whatever
        // the modules that just stopped did to the ship undone.
        if let Some(mut ship) = ship {
            if ship.fight.as_ref().is_some_and(|f| !f.applied.is_empty()) {
                let now = crate::combat::now_ms();
                let char_id = self.session.char_id.unwrap_or(0);
                let nothing = Default::default();
                let fresh = inflight::profile(&self.server.db, &self.server.dogma, char_id, ship_id, &nothing, now);
                match fresh.await {
                    Ok(Some(p)) => inflight::apply_profile(&mut ship, p, now),
                    Ok(None) => {}
                    Err(e) => error!("refreshing ship {ship_id} after the jump: {}", crate::logging::chain(&e)),
                }
            }
            ship.pos = jump.arrival;
            ship.vel = Vec3::ZERO;
            ship.command = Command::Stop;
            ship.arrived_through = Some(to_gate);
            let mut world = self.server.world.lock();
            space::add_ship(&mut world, jump.solar_system_id, ship);
            let tick = world.parks.get(&jump.solar_system_id).map_or(destiny::tick_now(), |p| p.tick);
            let flash = space::special_fx(tick, ship_id, Some(to_gate), "effects.JumpIn");
            let others: Vec<i64> =
                world.in_space(jump.solar_system_id).into_iter().filter(|c| *c != self.client_id).collect();
            for client in others {
                space::broadcast(&world, jump.solar_system_id, Some(client), vec![flash.clone()]);
            }
        }

        info!(
            "ship {ship_id} jumped from gate {from_gate} in {system_id} to gate {to_gate} in {}",
            jump.solar_system_id
        );
        self.reply(req, &Value::None).await
    }

    /// A session move the world decided on: the ship blew up, or the capsule did.
    pub(super) async fn forced_move(&mut self, m: Move) -> Result {
        let before = self.session.clone();
        match m {
            Move::ShipLost { ship_id } => {
                self.session.ship_id = Some(ship_id);
                self.send_session_change_from(&before).await?;
                info!("ship destroyed; ejected in capsule {ship_id}");
            }
            Move::Podded { station_id, solar_system_id, constellation_id, region_id, ship_id } => {
                self.session.station_id = Some(station_id);
                self.session.solar_system_id = None;
                self.session.solar_system_id2 = Some(solar_system_id);
                self.session.constellation_id = Some(constellation_id);
                self.session.region_id = Some(region_id);
                self.session.ship_id = ship_id;
                self.send_session_change_from(&before).await?;
                info!(ship = ?ship_id, "capsule destroyed; woke up in a clone at station {station_id}");
            }
            Move::Gang { gang_id, leader } => {
                self.session.gang_id = gang_id;
                self.session.gang_role = gang_id.map(|_| if leader { crate::svc::gang::GANG_LEADER_ROLE } else { 0 });
                if self.session.change_from(&before) != Value::Dict(Vec::new()) {
                    self.send_session_change_from(&before).await?;
                    info!(gang = ?gang_id, leader, "gang changed");
                }
            }
            Move::Teleport { station_id, solar_system_id, constellation_id, region_id, pos } => {
                self.teleport(station_id, solar_system_id, constellation_id, region_id, Vec3::new(pos.0, pos.1, pos.2)).await?;
            }
            Move::Affiliation => {
                let Some(char_id) = self.session.char_id else { return Ok(()) };
                self.load_affiliation(char_id).await;
                if self.session.change_from(&before) != Value::Dict(Vec::new()) {
                    self.send_session_change_from(&before).await?;
                    info!(corp = ?self.session.corp_id, alliance = ?self.session.alliance_id, "affiliation changed");
                }
            }
        }
        Ok(())
    }

    /// /tr: out of where the ship is, and into the station or the spot given.
    async fn teleport(&mut self, station_id: Option<i64>, system: i64, constellation: i64, region: i64, pos: Vec3) -> Result {
        let (Some(char_id), Some(ship_id)) = (self.session.char_id, self.session.ship_id) else { return Ok(()) };
        let from = self.session.solar_system_id;
        if station_id.is_none() && from.is_none() {
            warn!("teleport into space from a station; ignored");
            return Ok(());
        }
        let ship = from.and_then(|s| space::remove_ship(&mut self.server.world.lock(), s, ship_id));
        if let Some(fight) = ship.as_ref().and_then(|s| s.fight.as_ref()) {
            inflight::save_vitals(&self.server.db, ship_id, fight, true).await;
        }
        let before = self.session.clone();
        self.session.solar_system_id2 = Some(system);
        self.session.constellation_id = Some(constellation);
        self.session.region_id = Some(region);
        match station_id {
            Some(station) => {
                self.session.station_id = Some(station);
                self.session.solar_system_id = None;
                self.session.ship_id = self.store_docked(char_id, ship_id, station).await;
                self.send_session_change_from(&before).await?;
                info!("teleported into station {station}");
            }
            None => {
                space::ensure_park(&self.ctx(), system).await;
                self.session.solar_system_id = Some(system);
                self.send_session_change_from(&before).await?;
                if let Some(mut ship) = ship {
                    ship.pos = pos;
                    ship.vel = Vec3::ZERO;
                    ship.command = Command::Stop;
                    space::add_ship(&mut self.server.world.lock(), system, ship);
                }
                info!("teleported to {pos:?} in {system}");
            }
        }
        Ok(())
    }

    /// Reads the character's corporation, its alliance and HQ, and the
    /// character's roles in it into the session.
    async fn load_affiliation(&mut self, char_id: i64) {
        const SQL: &str = r#"
            SELECT c."corporationID", co."allianceID", co."stationID",
                   COALESCE(r."rolesAtAll", 0), COALESCE(r."rolesAtHQ", 0),
                   COALESCE(r."rolesAtBase", 0), COALESCE(r."rolesAtOther", 0)
              FROM "character_" c
              LEFT JOIN "corporation" co ON co."corporationID" = c."corporationID"
              LEFT JOIN "chrCorporationRoles" r ON r."characterID" = c."characterID"
             WHERE c."characterID" = $1"#;
        let row = match self.server.db.query_opt(SQL, &[&(char_id as i32)]).await {
            Ok(Some(row)) => row,
            Ok(None) => return,
            Err(e) => {
                error!("loading the corporation of {char_id}: {}", crate::logging::chain(&e));
                return;
            }
        };
        let opt = |i: usize| crate::svc::rows::int(&row, i);
        let s = &mut self.session;
        s.corp_id = opt(0);
        s.alliance_id = opt(1).filter(|v| *v != 0);
        s.hq_id = opt(2).filter(|v| *v != 0);
        s.base_id = s.hq_id;
        s.roles_at_all = opt(3).unwrap_or(0);
        s.roles_at_hq = opt(4).unwrap_or(0);
        s.roles_at_base = opt(5).unwrap_or(0);
        s.roles_at_other = opt(6).unwrap_or(0);
    }

    /// Record a dock: the ship goes into the station hangar, the pilot into the
    /// station, and the ship stays the active one. Returns the active ship.
    async fn store_docked(&self, char_id: i64, ship_id: i64, station_id: i64) -> Option<i64> {
        const SQL: &str = r#"
            WITH station AS (
                SELECT "stationID","solarSystemID","constellationID","regionID"
                  FROM "staStations" WHERE "stationID" = $3
            ),
            ship AS (
                UPDATE entity SET "locationID" = $3, "flag" = $4
                 WHERE "itemID" = $2 AND "ownerID" = $1
                RETURNING "itemID"
            ),
            pilot AS (
                UPDATE entity SET "locationID" = $3 WHERE "itemID" = $1
            )
            UPDATE "character_" c
               SET "stationID" = s."stationID", "solarSystemID" = s."solarSystemID",
                   "constellationID" = s."constellationID", "regionID" = s."regionID",
                   "shipID" = (SELECT "itemID" FROM ship)
              FROM station s
             WHERE c."characterID" = $1
            RETURNING c."shipID""#;
        let params: [&(dyn tokio_postgres::types::ToSql + Sync); 4] =
            [&(char_id as i32), &(ship_id as i32), &(station_id as i32), &FLAG_HANGAR];
        match self.server.db.query_opt(SQL, &params).await {
            Ok(row) => row.and_then(|r| crate::svc::rows::int(&r, 0)),
            Err(e) => {
                error!("recording {char_id} docked at {station_id}: {}", crate::logging::chain(&e));
                None
            }
        }
    }

    /// Pick the character's ship in `station_id` (preferring the recorded one) and
    /// store it as their active ship.
    async fn resolve_active_ship(&self, char_id: i64, station_id: i64) -> Option<i64> {
        const SQL: &str = r#"
            WITH candidate AS (
                SELECT e."itemID"
                  FROM entity e
                  JOIN "invTypes"  t ON t."typeID"  = e."typeID"
                  JOIN "invGroups" g ON g."groupID" = t."groupID"
                 WHERE e."ownerID"    = $1
                   AND e."locationID" = $2
                   AND g."categoryID" = 6
                   AND e."singleton"  = 1
                 ORDER BY (e."itemID" = (SELECT c."shipID" FROM "character_" c
                                          WHERE c."characterID" = $1)) DESC,
                          e."itemID"
                 LIMIT 1
            )
            UPDATE "character_" SET "shipID" = (SELECT "itemID" FROM candidate)
             WHERE "characterID" = $1
             RETURNING "shipID""#;
        match self.server.db.query_opt(SQL, &[&(char_id as i32), &(station_id as i32)]).await {
            Ok(row) => row.and_then(|r| crate::svc::rows::int(&r, 0)),
            Err(e) => {
                error!("resolving the active ship for {char_id}: {}", crate::logging::chain(&e));
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ballpark::{Celestial, Pilot, Ship, ShipStats};
    use eve_proto::MessageType;

    fn station_and_ship(stats: ShipStats, pos: Vec3) -> (Park, PendingDock) {
        let station = Celestial {
            id: 60_013_417,
            type_id: 3869,
            group_id: 15,
            name: "Station".into(),
            pos: Vec3::ZERO,
            radius: 43_259.0,
            undock: None,
            jump: None,
        };
        let dock = PendingDock {
            request: MachoPacket::new(MessageType::CallReq, Vec::new()),
            system_id: 30_003_287,
            ship_id: 1,
            station_id: station.id,
            warp_deadline: DOCK_WARP_LIMIT,
            approaching: false,
        };
        let mut park = Park::new(dock.system_id, vec![station], 0);
        let pilot = Pilot {
            type_id: 587,
            group_id: 25,
            name: "Pilot".into(),
            char_id: 2,
            corp_id: 3,
            alliance_id: None,
            security: 0.0,
            bounty: 0.0,
        };
        let ship = Ship::new(dock.ship_id, 10, stats, pilot, pos, Vec3::ZERO);
        park.ships.insert(dock.ship_id, ship);
        (park, dock)
    }

    #[test]
    fn docking_waits_for_warp_then_approaches_from_seventeen_km() {
        let stats = ShipStats::new(37.0, 1_100_000.0, 3.1, 320.0, 2.0);
        let (mut park, mut dock) = station_and_ship(stats, Vec3::new(149_597_870_700.0, 0.0, 0.0));
        let exit = Vec3::new(43_259.0 + stats.radius + 17_000.0, 0.0, 0.0);
        let warp = Command::Warp { target: exit, range: 0.0 };
        park.command(dock.ship_id, warp.clone());
        finish_warp(&mut dock, &mut park);
        park.set_speed_fraction(dock.ship_id, 0.0);
        let Ok(DockStep::Approach { gap }) = dock.step(&mut park, false) else {
            panic!("Dock should start a sublight approach after landing");
        };
        assert!((16_000.0..=18_000.0).contains(&gap));
        assert_eq!(park.ships[&dock.ship_id].speed_fraction, 1.0);
        let mut click = Call {
            service: "beyonce".into(),
            method: "GotoDirection".into(),
            args: vec![Value::Double(-1.0), Value::Double(0.0), Value::Double(0.0)],
            kwargs: Vec::new(),
        };
        assert!(dock.continues_toward_station(&park, &click));
        click.args[0] = Value::Double(1.0);
        assert!(!dock.continues_toward_station(&park, &click));
        assert!(finish_approach(&mut dock, &mut park, 120) > 1);
    }

    #[test]
    fn docking_keeps_a_large_ship_approaching_past_the_old_timeout() {
        let stats = ShipStats::new(9_000.0, 1_800_000_000.0, 0.05, 87.5, 1.0);
        let pos = Vec3::new(43_259.0 + stats.radius + 100_000.0, 0.0, 0.0);
        let (mut park, mut dock) = station_and_ship(stats, pos);
        assert_eq!(dock.step(&mut park, false), Ok(DockStep::Approach { gap: 100_000.0 }));
        assert!(finish_approach(&mut dock, &mut park, 2_000) > 600);
    }

    #[test]
    fn distant_dock_warps_before_the_sublight_approach() {
        let stats = ShipStats::new(37.0, 1_100_000.0, 3.1, 320.0, 2.0);
        let (mut park, mut dock) = station_and_ship(stats, Vec3::new(149_597_870_700.0, 0.0, 0.0));
        assert_eq!(dock.step(&mut park, false), Ok(DockStep::Warp));
        assert!(park.ships[&dock.ship_id].warping());
        finish_warp(&mut dock, &mut park);
        let Ok(DockStep::Approach { gap }) = dock.step(&mut park, false) else {
            panic!("Dock should approach automatically after the station warp");
        };
        assert!((1_000.0..10_000.0).contains(&gap));
        finish_approach(&mut dock, &mut park, 120);
    }

    fn finish_warp(dock: &mut PendingDock, park: &mut Park) {
        let warp = park.ships[&dock.ship_id].command.clone();
        for tick in park.tick + 1..=park.tick + DOCK_WARP_LIMIT {
            assert_eq!(dock.step(park, false), Ok(DockStep::Wait));
            assert_eq!(park.ships[&dock.ship_id].command, warp);
            park.advance(tick);
            if !park.ships[&dock.ship_id].warping() {
                break;
            }
        }
        assert!(!park.ships[&dock.ship_id].warping());
        assert!(park.landed.contains(&dock.ship_id));
        assert_eq!(dock.step(park, false), Ok(DockStep::Wait));
        park.landed.clear(); // The heartbeat delivers the landing state.
    }

    fn finish_approach(dock: &mut PendingDock, park: &mut Park, max_ticks: i64) -> i64 {
        let start = park.tick;
        for tick in start + 1..=start + max_ticks {
            park.advance(tick);
            let gap = park.surface_distance(dock.ship_id, dock.station_id).unwrap();
            let step = dock.step(park, false).unwrap();
            if step == DockStep::Arrived {
                assert!(gap > 0.0 && gap <= 1_000.0, "arrived at {gap} m from the station");
                return tick - start;
            }
            assert!(gap > 1_000.0 && step == DockStep::Wait, "unexpected docking step {step:?} at {gap} m");
        }
        panic!("ship did not reach the station within {max_ticks}s");
    }
}
