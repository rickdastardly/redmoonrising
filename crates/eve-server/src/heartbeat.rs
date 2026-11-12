//! The server's pulse: four times a second, every solar system with a player
//! in space moves on (see `fight`). The pulse only decides, under the world
//! lock; what it decided that needs the database is done after the lock is
//! released, on the heartbeat's own connection: charges used, ore mined,
//! bounties paid, ships and capsules lost.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

use eve_marshal::Value;
use tokio_postgres::Client;
use tracing::{error, info, warn};

use crate::ballpark::Vec3;
use crate::combat;
use crate::conn::Server;
use crate::items::{self, ItemTxn, NewItem};
use crate::svc::{inflight, space};
use crate::world::{World, WorldState};

pub(crate) const PULSE_MS: u64 = 250;
/// Pulses between looks at slow corporate business (alliance applications
/// coming due): once a minute.
const AFFAIRS_EVERY: u64 = 60_000 / PULSE_MS;
const SECURITY_EVERY: u64 = 20 * 60_000 / PULSE_MS;
const FLAG_CARGO: i64 = 5;
/// market_refTypes: "Bounty Prize".
const REF_BOUNTY_PRIZE: i32 = 17;
const OWNER_CONCORD: i64 = 1000125;

/// What a pulse decided, to be told or done once the world lock is released.
#[derive(Default)]
pub struct Out {
    /// Destiny entries in order, for everyone in a system or one pilot.
    destiny: Vec<(i64, Option<i64>, Value)>,
    notes: Vec<(i64, &'static str, Value)>,
    pub jobs: Vec<Job>,
    /// Ships whose hit points changed this pulse.
    hurt: BTreeSet<i64>,
    /// Ships whose capacitor changed this pulse.
    drained: BTreeSet<i64>,
}

impl Out {
    pub fn destiny(&mut self, system: i64, entry: Value) {
        self.destiny.push((system, None, entry));
    }

    pub fn destiny_to(&mut self, system: i64, client_id: i64, entry: Value) {
        self.destiny.push((system, Some(client_id), entry));
    }

    pub fn note(&mut self, client_id: i64, event: &'static str, args: Value) {
        self.notes.push((client_id, event, args));
    }

    pub fn hurt(&mut self, ship_id: i64) {
        self.hurt.insert(ship_id);
    }

    pub fn drained(&mut self, ship_id: i64) {
        self.drained.insert(ship_id);
    }

    /// The hurt and drained marks for ships `here` says belong to the caller.
    pub fn take_marks(&mut self, here: impl Fn(i64) -> bool) -> (BTreeSet<i64>, BTreeSet<i64>) {
        let (hurt, rest): (BTreeSet<i64>, BTreeSet<i64>) = std::mem::take(&mut self.hurt).into_iter().partition(|id| here(*id));
        self.hurt = rest;
        let (drained, rest): (BTreeSet<i64>, BTreeSet<i64>) =
            std::mem::take(&mut self.drained).into_iter().partition(|id| here(*id));
        self.drained = rest;
        (hurt, drained)
    }

    #[cfg(test)]
    pub fn notes(&self) -> &[(i64, &'static str, Value)] {
        &self.notes
    }

    #[cfg(test)]
    pub fn destiny_entries(&self) -> impl Iterator<Item = &Value> {
        self.destiny.iter().map(|(_, _, entry)| entry)
    }

    /// Sends what was decided; returns the jobs left to do.
    pub fn deliver(self, world: &WorldState) -> Vec<Job> {
        let mut system_clients = BTreeMap::new();
        let mut by_client: BTreeMap<(i64, i64), Vec<Value>> = BTreeMap::new();
        for (system, client, entry) in self.destiny {
            if let Some(client) = client {
                by_client.entry((system, client)).or_default().push(entry);
            } else {
                let clients = system_clients.entry(system).or_insert_with(|| world.in_space(system));
                for &client in clients.iter() {
                    by_client.entry((system, client)).or_default().push(entry.clone());
                }
            }
        }
        for ((_system, client), entries) in by_client {
            let args = Value::Tuple(vec![Value::List(entries), Value::Int(0)]);
            world.notify(&[client], "DoDestinyUpdate", args);
        }
        for (client, event, args) in self.notes {
            world.notify(&[client], event, args);
        }
        self.jobs
    }
}

/// Work a pulse leaves for after the lock.
#[derive(Debug, Clone)]
pub enum Job {
    /// A charge stack burned down to `quantity`.
    Ammo { client_id: i64, charge_id: i64, quantity: i64 },
    /// Ore into `ship_id`'s hold, mined by `module_id` on `miner_id` (the ship, or a drone).
    Ore { client_id: i64, char_id: i64, ship_id: i64, miner_id: i64, module_id: i64, system: i64, type_id: i64, quantity: i64 },
    /// A rat's bounty, and what killing it does for security status: (share
    /// of the way to 10, highest it raises it to).
    Bounty { client_id: Option<i64>, char_id: i64, amount: f64, npc_type: i64, security: (f64, f64) },
    /// A module that changes attributes stopped.
    Refit { client_id: i64, char_id: i64, ship_id: i64, system: i64 },
    /// A player's ship (or capsule) was destroyed.
    Lost(Box<crate::losses::Loss>),
    /// A bot's ship was destroyed.
    BotLost(Box<crate::losses::Loss>),
    /// Something was killed with loot on it.
    Loot { system: i64, pos: Vec3, owner_id: i64, stash: Vec<crate::loot::Stash> },
    /// A can's time ran out.
    Scrap { can_id: i64 },
    /// A drone flew home: back into its ship's drone bay.
    DroneHome { client_id: i64, drone_id: i64, ship_id: i64 },
    /// A drone was destroyed.
    DroneLost { client_id: Option<i64>, drone_id: i64 },
    /// A fighter reached the grid it followed its target to.
    DronePosition { drone_id: i64, pos: Vec3 },
    /// The last rat of a mission site died.
    SiteCleared { mission_id: i64, char_id: i64 },
    /// A crime, or a kill, changed a pilot's security status by this much.
    Security { char_id: i64, delta: f64 },
    /// A starbase structure's health after a fight.
    StructureDamage { id: i64, shield: f64, armor: f64, hull: f64 },
    /// A tower fell below a quarter shield and may go reinforced.
    Reinforce { system: i64, tower: i64 },
    /// A battery fired a round from its bay.
    StructureAmmo { item_id: i64, quantity: i64 },
    /// A starbase structure was destroyed.
    StructureLost { id: i64, tower: bool },
    /// A starbase structure finished a timed step.
    Structure { id: i64, state: i16, completed_stamp: i64, tower: Option<i64> },
}

pub async fn run(server: Arc<Server>) {
    let mut db = loop {
        match crate::db::connect(&server.db_url).await {
            Ok(c) => break c,
            Err(e) => {
                error!("heartbeat cannot reach the database: {}", crate::logging::chain(&e));
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        }
    };
    match crate::drones::recover_stale_nonfighters(&db).await {
        Ok(count) if count > 0 => info!("returned {count} stranded non-fighter drones to their owners' active ships"),
        Err(e) => error!("recovering stranded drones: {}", crate::logging::chain(&e)),
        _ => {}
    }
    crate::svc::pvptrade::recover(&db).await;
    if let Err(e) = ensure_security_history(&db).await {
        error!("security history setup: {}", crate::logging::chain(&e));
    }
    info!("heartbeat every {PULSE_MS} ms");
    let mut interval = tokio::time::interval(Duration::from_millis(PULSE_MS));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut pulses = 0u64;
    loop {
        interval.tick().await;
        let work = std::time::Instant::now();
        for job in pulse(&server.world, combat::now_ms()) {
            perform(&server, &mut db, job).await;
        }
        crate::health::record_pulse(work.elapsed());
        pulses += 1;
        if pulses % AFFAIRS_EVERY == 1 {
            crate::svc::alliance::take_effect(&server.world, &mut db, server.alliance_join_delay_secs).await;
            crate::svc::alliance::bill_alliances(&db).await;
            crate::svc::war::bill_wars(&server.world, &db).await;
            let loaded: Vec<i32> = server.world.lock().parks.keys().map(|id| *id as i32).collect();
            if let Err(e) = crate::pos::finish_unloaded(&db, &loaded).await {
                error!("updating starbase timers: {}", crate::logging::chain(&e));
            }
            crate::svc::starbase::burn_fuel(&server.world, &mut db).await;
            crate::svc::starbase::sovereignty(&mut db, server.sov_claim_delay_secs).await;
            crate::svc::votes::close_votes(&server.world, &db).await;
            crate::guard::reload(&server.world, &db).await;
            crate::guard::refresh_standings(&db).await;
        }
        if pulses % SECURITY_EVERY == 1 {
            if let Err(e) = apply_security_tick(&mut db).await {
                error!("security status tick: {}", crate::logging::chain(&e));
            }
        }
    }
}

/// One pulse of every occupied system.
fn pulse(world: &World, now: i64) -> Vec<Job> {
    let mut w = world.lock();
    let mut out = Out::default();
    out.jobs.append(&mut w.pending_jobs);
    let tick = crate::destiny::tick_now();
    for park in w.parks.values_mut() {
        crate::pos::step(park, &mut out);
        if park.players().next().is_none() {
            park.entered_warp.clear();
            park.landed.clear();
            continue;
        }
        park.advance(tick);
        crate::drones::recover_distant_nonfighters(park, now, &mut out);
        let entered_warp = std::mem::take(&mut park.entered_warp);
        for ship_id in entered_warp {
            let effect = space::warp_fx(park.tick, ship_id, true);
            let mut viewers = space::ship_viewers(park, ship_id);
            if let Some(ship) = park.ships.get(&ship_id).filter(|ship| ship.is_player()) {
                if !viewers.contains(&ship.client_id) { viewers.push(ship.client_id); }
                info!("player warp entry: ship {ship_id} at {:?}, {:?}, {} viewer(s)", ship.pos, ship.command, viewers.len());
            }
            for client in viewers {
                out.destiny_to(park.solar_system_id, client, effect.clone());
            }
            crate::drones::on_warp_entered(park, ship_id, now, &mut out);
        }
        let landed = std::mem::take(&mut park.landed);
        for ship_id in landed {
            let Some(ship) = park.ships.get(&ship_id).filter(|ship| !ship.in_warp()) else { continue };
            let mut viewers = space::ship_viewers(park, ship_id);
            if ship.is_player() {
                if !viewers.contains(&ship.client_id) { viewers.push(ship.client_id); }
                let nearest = park.belts.iter().map(|belt| (belt.id, (ship.pos - belt.pos).len()))
                    .min_by(|a, b| a.1.total_cmp(&b.1));
                let rocks = park.rocks.values().filter(|rock| (ship.pos - rock.pos).len() <= 1_000_000.0).count();
                info!("player warp landing: ship {ship_id} at {:?}, nearest belt {nearest:?}, {rocks} nearby asteroid(s), {} viewer(s)", ship.pos, viewers.len());
            }
            let entries = space::warp_landing_entries(ship, park.tick);
            for client in viewers {
                for entry in &entries {
                    out.destiny_to(park.solar_system_id, client, entry.clone());
                }
            }
            crate::drones::on_warp_landed(park, ship_id, now);
        }
        crate::drones::arrive_following_fighters(park, now, &mut out);
        crate::fight::step(park, now, &mut out);
        for (client, entry) in space::sync_ship_visibility(park) {
            out.destiny_to(park.solar_system_id, client, entry);
        }
        for (client, entry) in space::sync_object_visibility(park) {
            out.destiny_to(park.solar_system_id, client, entry);
        }
    }
    out.deliver(&w)
}

async fn perform(server: &Server, db: &mut Client, job: Job) {
    match job {
        Job::Ammo { client_id, charge_id, quantity } => ammo(server, db, client_id, charge_id, quantity).await,
        Job::Ore { client_id, char_id, ship_id, miner_id, module_id, system, type_id, quantity } => {
            ore(server, db, client_id, char_id, (ship_id, miner_id, module_id), system, type_id, quantity).await
        }
        Job::Bounty { client_id, char_id, amount, npc_type, security } => {
            bounty(server, db, client_id, char_id, amount, npc_type).await;
            queue_security(db, char_id, npc_type, security).await;
        }
        Job::Refit { client_id, char_id, ship_id, system } => {
            inflight::refit(db, &server.dogma, &server.world, client_id, char_id, ship_id, system).await
        }
        Job::Lost(loss) => crate::losses::handle(server, db, *loss).await,
        Job::BotLost(loss) => crate::losses::bot_lost(server, db, *loss).await,
        Job::Loot { system, pos, owner_id, stash } => crate::loot::drop_can(db, &server.world, system, pos, owner_id, stash).await,
        Job::Scrap { can_id } => crate::loot::scrap(db, can_id).await,
        Job::DroneHome { client_id, drone_id, ship_id } => crate::drones::home(server, db, client_id, drone_id, ship_id).await,
        Job::DroneLost { client_id, drone_id } => crate::drones::lost(server, db, client_id, drone_id).await,
        Job::DronePosition { drone_id, pos } => {
            if let Err(e) = db.execute(r#"UPDATE "entity" SET "x"=$2,"y"=$3,"z"=$4 WHERE "itemID"=$1"#,
                &[&(drone_id as i32), &pos.x, &pos.y, &pos.z]).await {
                warn!("saving fighter {drone_id} position: {}", crate::logging::chain(&e));
            }
        }
        Job::Security { char_id, delta } => change_security(db, char_id, delta).await,
        Job::SiteCleared { mission_id, char_id } => crate::svc::agent_missions::site_cleared(&server.world, db, mission_id, char_id).await,
        Job::Structure { id, state, completed_stamp, tower } => save_structure(db, id, state, completed_stamp, tower).await,
        Job::StructureDamage { id, shield, armor, hull } => crate::svc::starbase::save_damage(db, id, shield, armor, hull).await,
        Job::Reinforce { system, tower } => crate::svc::starbase::reinforce(&server.world, db, system, tower).await,
        Job::StructureLost { id, tower } => crate::svc::starbase::lost(db, id, tower).await,
        Job::StructureAmmo { item_id, quantity } => crate::guard::save_ammo(db, item_id, quantity).await,
    }
}

async fn ammo(server: &Server, db: &mut Client, client_id: i64, charge_id: i64, quantity: i64) {
    let result = async {
        let mut txn = ItemTxn::begin(db).await?;
        txn.set_quantity(charge_id, quantity).await?;
        txn.commit().await
    }
    .await;
    match result {
        Ok(events) => items::notify(&server.world, client_id, events),
        Err(e) => error!("using charge {charge_id}: {}", crate::logging::chain(&e)),
    }
}

/// Mined ore goes into the cargo hold, as much as fits; a full hold stops
/// the laser.
#[allow(clippy::too_many_arguments)]
async fn ore(server: &Server, db: &mut Client, client_id: i64, char_id: i64, (ship_id, miner_id, module_id): (i64, i64, i64), system: i64, type_id: i64, quantity: i64) {
    let result = async {
        let room = db
            .query_one(
                r#"SELECT COALESCE(st."capacity", 0)
                          - COALESCE((SELECT SUM(e."quantity" * COALESCE(t."volume", 0))
                                        FROM entity e JOIN "invTypes" t ON t."typeID" = e."typeID"
                                       WHERE e."locationID" = $1 AND e."flag" = $2), 0),
                          (SELECT COALESCE("volume", 1) FROM "invTypes" WHERE "typeID" = $3)
                     FROM entity s JOIN "invTypes" st ON st."typeID" = s."typeID"
                    WHERE s."itemID" = $1"#,
                &[&(ship_id as i32), &(FLAG_CARGO as i32), &(type_id as i32)],
            )
            .await?;
        let (room, volume): (f64, f64) = (room.get(0), room.get(1));
        let fits = ((room / volume.max(1e-6)).floor() as i64).clamp(0, quantity);
        if fits == 0 {
            return Ok((0, Vec::new()));
        }
        let mut txn = ItemTxn::begin(db).await?;
        txn.stack_onto(NewItem { type_id, owner_id: char_id, location_id: ship_id, flag: FLAG_CARGO, quantity: fits, singleton: false })
            .await?;
        Ok::<_, tokio_postgres::Error>((fits, txn.commit().await?))
    }
    .await;
    match result {
        Ok((fits, events)) => {
            items::notify(&server.world, client_id, events);
            if fits < quantity {
                let mut w = server.world.lock();
                let mut out = Out::default();
                if let Some(park) = w.parks.get_mut(&system) {
                    let message = crate::fight::notify("Your cargo hold is full; the mining laser shuts down.");
                    crate::fight::stop(park, miner_id, module_id, combat::now_ms(), &mut out, Some(message));
                }
                out.deliver(&w);
            }
        }
        Err(e) => error!("mining {quantity} of {type_id} into {ship_id}: {}", crate::logging::chain(&e)),
    }
}

async fn bounty(server: &Server, db: &mut Client, client_id: Option<i64>, char_id: i64, amount: f64, npc_type: i64) {
    use crate::svc::wallet::{post, Entry, Transfer};
    let arg = npc_type.to_string();
    let result = async {
        let tx = db.transaction().await.map_err(Transfer::Db)?;
        let entry = Entry { ref_type: REF_BOUNTY_PRIZE, payer: OWNER_CONCORD, payee: char_id, arg_id1: &arg, reason: "" };
        let balance = post(&tx, char_id, amount, &entry).await?;
        tx.commit().await.map_err(Transfer::Db)?;
        Ok::<_, Transfer>(balance)
    }
    .await;
    match result {
        Ok(balance) => {
            info!("bounty of {amount} ISK to {char_id} for a {npc_type}");
            if let Some(client) = client_id {
                let w = server.world.lock();
                let cash = Value::Tuple(vec![Value::bytes("cash"), Value::Int(char_id), Value::Double(balance)]);
                w.notify(&[client], "OnAccountChange", cash);
                let text = format!("A bounty of {} ISK has been paid into your wallet.", fmt_isk(amount));
                let (key, args) = crate::fight::notify(&text);
                w.notify(&[client], "OnRemoteMessage", Value::Tuple(vec![Value::bytes(key), args]));
            }
        }
        Err(Transfer::Db(e)) => error!("paying a bounty to {char_id}: {}", crate::logging::chain(&e)),
        Err(_) => warn!("bounty for {char_id}: no wallet"),
    }
}

async fn ensure_security_history(db: &Client) -> Result<(), tokio_postgres::Error> {
    db.execute(r#"CREATE TABLE IF NOT EXISTS "securityPending" (
        "eventID" bigserial PRIMARY KEY, "characterID" integer NOT NULL,
        "npcTypeID" integer NOT NULL, "bonus" float8 NOT NULL, "killedAt" bigint NOT NULL)"#, &[]).await?;
    db.execute(r#"CREATE SEQUENCE IF NOT EXISTS "standingTransactions_eventID_seq""#, &[]).await?;
    db.query_one(r#"SELECT setval('"standingTransactions_eventID_seq"',
        GREATEST((SELECT COALESCE(MAX("eventID"), 0) FROM "standingTransactions"), 1))"#, &[]).await?;
    Ok(())
}

/// Queue every pirate kill so the 20-minute tick includes all of them.
async fn queue_security(db: &Client, char_id: i64, npc_type: i64, (bonus, _): (f64, f64)) {
    if bonus <= 0.0 {
        return;
    }
    let sql = r#"INSERT INTO "securityPending" ("characterID", "npcTypeID", "bonus", "killedAt") VALUES ($1,$2,$3,$4)"#;
    if let Err(e) = db.execute(sql, &[&(char_id as i32), &(npc_type as i32), &bonus, &crate::bulkdata::filetime_now()]).await {
        error!("queuing security gain for {char_id}: {}", crate::logging::chain(&e));
    }
}

async fn apply_security_tick(db: &mut Client) -> Result<(), tokio_postgres::Error> {
    let tx = db.transaction().await?;
    let pending = tx.query(r#"SELECT "eventID", "characterID", "npcTypeID", "bonus", "killedAt"
                              FROM "securityPending" ORDER BY "eventID" FOR UPDATE"#, &[]).await?;
    for kill in pending {
        let (id, character, npc, bonus, killed_at): (i64, i32, i32, f64, i64) =
            (kill.get(0), kill.get(1), kill.get(2), kill.get(3), kill.get(4));
        let changed = tx.query_opt(r#"UPDATE "character_" SET "securityRating" =
                 LEAST(10.0, "securityRating" + (10.0 - "securityRating") * $2)
                 WHERE "characterID" = $1 RETURNING "securityRating""#, &[&character, &bonus]).await?;
        if changed.is_some() {
            tx.execute(r#"INSERT INTO "standingTransactions"
                ("eventID", "eventTypeID", "eventDateTime", "fromID", "toID",
                 "originalFromID", "originalToID", "modification", "int_1", "msg")
                VALUES (nextval('"standingTransactions_eventID_seq"'), 89, $1, $2, $3,
                        $2, $3, $4, $5, 'Pirate ship destroyed')"#,
                &[&killed_at, &(OWNER_CONCORD as i32), &character, &bonus, &npc]).await?;
        }
        tx.execute(r#"DELETE FROM "securityPending" WHERE "eventID" = $1"#, &[&id]).await?;
    }
    tx.commit().await
}

async fn save_structure(db: &mut Client, id: i64, state: i16, completed_stamp: i64, tower: Option<i64>) {
    let sql = r#"UPDATE "posStructures" SET "state" = $2, "stateTimestamp" = NULL, "controlTowerID" = $4,
                        "moonID" = CASE WHEN $2 = 0 THEN NULL ELSE "moonID" END,
                        "lastFuel" = CASE WHEN state IN (2, 3) AND $2 = 4 AND "moonID" IS NOT NULL THEN $5 ELSE "lastFuel" END
                  WHERE "itemID" = $1 AND "stateTimestamp" = $3"#;
    if let Err(e) = db.execute(sql, &[&(id as i32), &state, &completed_stamp, &tower.map(|t| t as i32), &crate::bulkdata::filetime_now()]).await {
        error!("saving structure {id}: {}", crate::logging::chain(&e));
    }
}

async fn change_security(db: &mut Client, char_id: i64, delta: f64) {
    let sql = r#"UPDATE "character_" SET "securityRating" = GREATEST(-10, LEAST(10, "securityRating" + $2))
                  WHERE "characterID" = $1 RETURNING "securityRating""#;
    match db.query_opt(sql, &[&(char_id as i32), &delta]).await {
        Ok(Some(r)) => info!("security status of {char_id} {delta:+.3}, now {:.2}", r.get::<_, f64>(0)),
        Ok(None) => {}
        Err(e) => error!("changing the security status of {char_id}: {}", crate::logging::chain(&e)),
    }
}

pub fn fmt_isk(amount: f64) -> String {
    let whole = amount.round() as i64;
    let digits = whole.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ship_departure_jobs_survive_until_the_next_pulse() {
        let world = World::default();
        world.lock().pending_jobs.push(Job::DroneHome { client_id: 7, drone_id: 20, ship_id: 30 });
        let jobs = pulse(&world, combat::now_ms());
        assert!(matches!(jobs.as_slice(), [Job::DroneHome { drone_id: 20, ship_id: 30, .. }]));
        assert!(world.lock().pending_jobs.is_empty());
    }

    #[test]
    fn grid_updates_keep_add_before_effect_and_removal() {
        use crate::world::{Presence, Push};

        let system = 30_000_001;
        let world = World::default();
        let mut state = world.lock();
        let mut rx = state.connect(7);
        state.set_presence(7, Some(Presence { char_id: 100, corp_id: 2, solar_system_id: system,
            docked: false, station_id: None, role: 0 }));
        state.sees(7, system);
        let mut out = Out::default();
        let add = space::entry(10, "AddBalls", Vec::new());
        let effect = space::entry(10, "TerminalExplosion", vec![Value::Int(8)]);
        let remove = space::entry(10, "RemoveBalls", vec![Value::List(vec![Value::Int(8)])]);
        out.destiny_to(system, 7, add.clone());
        out.destiny(system, effect.clone());
        out.destiny(system, remove.clone());
        out.deliver(&state);

        let Ok(Push::Notify(note)) = rx.try_recv() else { panic!("missing grid update") };
        assert_eq!(note.args.seq().unwrap()[0], Value::List(vec![add, effect, remove]));
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn warp_landing_updates_the_ship_and_adds_only_new_grid_objects() {
        use crate::ballpark::{Command, Park, Pilot, Ship, ShipStats};
        use crate::world::{Presence, Push};

        let world = World::default();
        let mut rx = {
            let mut state = world.lock();
            let rx = state.connect(7);
            state.set_presence(7, Some(Presence { char_id: 100, corp_id: 2, solar_system_id: 30000001,
                docked: false, station_id: None, role: 0 }));
            state.sees(7, 30000001);
            let tick = crate::destiny::tick_now();
            let mut park = Park::new(30000001, Vec::new(), tick - 100);
            let stats = ShipStats::new(37.0, 1_100_000.0, 3.1, 320.0, 2.0);
            let pilot = Pilot { type_id: 587, group_id: 25, name: "Rifter".into(), char_id: 100,
                corp_id: 2, alliance_id: None, security: 0.0, bounty: 0.0 };
            park.ships.insert(10, Ship::new(10, 7, stats, pilot.clone(), Vec3::ZERO, Vec3::ZERO));
            let target = Vec3::new(5.0e12, 0.0, 0.0);
            let mut neighbor = Ship::new(11, 0, stats, pilot, target + Vec3::new(10_000.0, 0.0, 0.0), Vec3::ZERO);
            neighbor.bot = true;
            park.ships.insert(11, neighbor);
            park.visible_ships.insert(7, [10].into());
            park.command(10, Command::Warp { target, range: 0.0 });
            park.advance(tick - 1);
            assert_eq!(park.landed, vec![10]);
            park.entered_warp.clear();
            state.parks.insert(30000001, park);
            rx
        };
        pulse(&world, combat::now_ms());
        let mut batches = Vec::new();
        while let Ok(Push::Notify(note)) = rx.try_recv() {
            if note.event != "DoDestinyUpdate" { continue; }
            if let Some(entries) = note.args.seq().and_then(|args| args.first()).and_then(Value::seq) {
                batches.push(entries.to_vec());
            }
        }
        assert_eq!(batches.len(), 1, "landing should update the existing park");
        let landing = &batches[0];
        let payload = |i: usize| landing[i].seq().unwrap()[1].seq().unwrap();
        let expected = ["Stop", "SetBallPosition", "SetBallVelocity", "SetBallMassive", "OnSpecialFX", "AddBalls"];
        assert_eq!(landing.len(), expected.len());
        for (i, opcode) in expected.into_iter().enumerate() {
            assert_eq!(payload(i)[0], Value::bytes(opcode));
        }
        let world = world.lock();
        let park = &world.parks[&30000001];
        let ship = &park.ships[&10];
        let position = payload(1)[1].seq().unwrap();
        let velocity = payload(2)[1].seq().unwrap();
        assert_eq!(position, &[Value::Int(10), Value::Double(ship.pos.x), Value::Double(ship.pos.y), Value::Double(ship.pos.z)]);
        assert_eq!(velocity, &[Value::Int(10), Value::Double(ship.vel.x), Value::Double(ship.vel.y), Value::Double(ship.vel.z)]);
        let fx = payload(4)[1].seq().unwrap();
        assert_eq!(fx[6], Value::bytes("effects.Warping"));
        assert_eq!(fx[8], Value::Int(0), "the landing state must stop the warp effect");
        assert_eq!(park.visible_ships[&7], [10, 11].into());
    }

    #[test]
    fn isk_is_grouped_in_thousands() {
        assert_eq!(fmt_isk(10_000.0), "10,000");
        assert_eq!(fmt_isk(999.4), "999");
        assert_eq!(fmt_isk(1_234_567.0), "1,234,567");
    }

    #[test]
    fn marks_split_by_park() {
        let mut out = Out::default();
        out.hurt(1);
        out.hurt(2);
        out.drained(2);
        let (hurt, drained) = out.take_marks(|id| id == 2);
        assert_eq!(hurt.into_iter().collect::<Vec<_>>(), vec![2]);
        assert_eq!(drained.into_iter().collect::<Vec<_>>(), vec![2]);
        assert_eq!(out.hurt.into_iter().collect::<Vec<_>>(), vec![1]);
    }
}
