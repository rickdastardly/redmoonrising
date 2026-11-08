mod ballpark;
mod bots;
mod bulkdata;
mod combat;
mod concord;
mod config;
mod conn;
mod db;
mod destiny;
mod drones;
mod dogma;
mod exceptions;
mod fight;
mod gates;
mod guard;
mod heartbeat;
mod items;
mod logging;
mod market_seed;
mod losses;
mod loot;
mod npc;
mod pos;
mod services;
mod session;
mod sites;
mod svc;
mod web;
mod world;

use std::sync::Arc;
use std::time::Instant;

use eve_marshal::StringTable;
use tokio::net::TcpListener;
use tracing::{debug, error, info, warn};

use config::Config;
use conn::{Conn, Server};

/// Bulk objects this close to the 1 MB frame cap are worth a warning.
const BULK_WARN_BYTES: usize = 900_000;

#[tokio::main]
async fn main() {
    // The config says how to log, so it is read before there is a log.
    let (config, note) = match Config::load() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };
    if let Err(e) = logging::init(&config.log_level, config.log_file.as_deref()) {
        eprintln!("{e}");
        std::process::exit(2);
    }
    info!("{note}");
    if let Err(e) = run(config).await {
        error!("{}", logging::chain(&*e));
        std::process::exit(1);
    }
}

async fn run(config: Config) -> Result<(), Box<dyn std::error::Error>> {

    let table = std::fs::read(&config.strings)
        .map(|raw| StringTable::parse(&raw))
        .map_err(|e| {
            format!("cannot read the shared string table at {}: {e}", config.strings.display())
        })?;
    info!("string table: {} entries from {}", table.len(), config.strings.display());

    let mut client = db::connect(&config.database).await.map_err(|e| {
        format!("cannot reach PostgreSQL at {}: {}", config.database_redacted(), logging::chain(&e))
    })?;
    let accounts: i64 = client
        .query_one(r#"SELECT count(*) FROM "account""#, &[])
        .await
        .map(|r| r.get(0))
        .unwrap_or(0);
    info!("database: {} ({accounts} accounts)", config.database_redacted());
    let ids = db::sync_accounts(&client, &config.accounts)
        .await
        .map_err(|e| format!("setting up the config file's accounts: {}", logging::chain(&e)))?;
    if !ids.is_empty() {
        info!("{} account(s) from the config file", ids.len());
    }
    config::set_rules(config::Rules {
        starting_isk: config.starting_isk,
        account_isk: ids.iter().zip(&config.accounts).filter_map(|(id, a)| Some((*id, a.starting_isk?))).collect(),
        auto_create_accounts: config.auto_create_accounts,
    });

    svc::medical::ensure_grades(&mut client).await.map_err(|e| format!("preparing medical clone grades: {e}"))?;
    market_seed::ensure(&mut client).await.map_err(|e| format!("seeding the market: {}", logging::chain(&e)))?;
    svc::moves::ensure_capital_storage(&client).await.map_err(|e| format!("preparing capital hangars: {}", logging::chain(&e)))?;
    svc::fitting::ensure_doomsday_fitting(&client).await.map_err(|e| format!("preparing doomsday fittings: {}", logging::chain(&e)))?;
    svc::agent_missions::ensure(&client).await.map_err(|e| format!("preparing period missions: {}", logging::chain(&e)))?;
    svc::bookmark::ensure_coordinate_format(&mut client).await.map_err(|e| format!("repairing coordinate bookmarks: {}", logging::chain(&e)))?;
    losses::ensure_kill_items(&client).await.map_err(|e| format!("preparing kill item records: {}", logging::chain(&e)))?;
    web::ensure_storage(&client).await.map_err(|e| format!("preparing dashboard news and downtime: {}", logging::chain(&e)))?;

    let started = Instant::now();
    let bulk =
        bulkdata::BulkData::load(&client, &table, config.node_id, bulkdata::filetime_now()).await?;
    report_bulk(&bulk, started);

    let dogma = dogma::Catalog::load(&client).await.map_err(|e| format!("loading dogma: {}", logging::chain(&e)))?;

    let services = svc::registry();
    info!("{} services registered", services.len());

    let server = Arc::new(Server {
        table,
        db: client,
        db_url: config.database.clone(),
        node_id: config.node_id,
        bulk,
        dogma,
        services,
        world: world::World::default(),
        dump_dir: config.dump_dir.clone(),
        alliance_join_delay_secs: config.alliance_join_delay_secs,
        sov_claim_delay_secs: config.sov_claim_delay_secs,
    });
    pos::set_speedup(config.starbase_speedup);
    tokio::spawn(heartbeat::run(Arc::clone(&server)));
    bots::configure(config.bots_pirates_attack, config.bots_controls);
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let bot_task = tokio::spawn(bots::run(Arc::clone(&server), config.bots, config.bots_speed, config.bots_shown, shutdown_rx));
    tokio::spawn(web::run(Arc::clone(&server), config.web.clone()));
    let listener = TcpListener::bind(&config.bind).await?;
    info!("listening on {} as node {}", config.bind, config.node_id);

    let mut seq = 0i64;
    let shutdown_file = std::env::var_os("RMR_SHUTDOWN_FILE").map(std::path::PathBuf::from);
    let mut shutdown_check = tokio::time::interval(std::time::Duration::from_millis(500));
    loop {
        let accepted = tokio::select! {
            result = listener.accept() => result,
            _ = shutdown_check.tick() => {
                if shutdown_file.as_ref().is_some_and(|path| path.exists()) {
                    info!("graceful shutdown requested");
                    break;
                }
                match web::take_due_downtime(&server.db).await {
                    Ok(true) => {
                        info!("scheduled downtime reached zero; saving and stopping");
                        if let Some(path) = &shutdown_file {
                            if let Err(e) = std::fs::write(path, b"downtime") {
                                warn!("recording planned downtime for launcher: {e}");
                            }
                        }
                        break;
                    }
                    Err(e) => warn!("checking scheduled downtime: {}", logging::chain(&e)),
                    _ => {}
                }
                continue;
            }
        };
        let (stream, peer) = match accepted {
            Ok(v) => v,
            Err(e) => {
                warn!("accept failed: {err}", err = crate::logging::chain(&e));
                continue;
            }
        };
        let _ = stream.set_nodelay(true);
        seq += 1;
        tokio::spawn(Conn::new(stream, peer, Arc::clone(&server), seq).run());
    }
    let _ = shutdown_tx.send(true);
    match tokio::time::timeout(std::time::Duration::from_secs(20), bot_task).await {
        Ok(Ok(())) => info!("bot state saved"),
        Ok(Err(e)) => warn!("bot shutdown task failed: {e}"),
        Err(_) => warn!("timed out saving bot state"),
    }
    save_live_ships(&server).await;
    Ok(())
}

async fn save_live_ships(server: &Server) {
    let ships: Vec<_> = {
        let world = server.world.lock();
        world.parks.values().flat_map(|park| park.ships.values()).filter(|ship| ship.is_player() || ship.drone.is_some())
            .map(|ship| (ship.id, ship.pos, ship.fight.clone())).collect()
    };
    for (id, pos, fight) in &ships {
        if let Err(e) = server.db.execute(r#"UPDATE entity SET "x"=$2,"y"=$3,"z"=$4 WHERE "itemID"=$1"#,
            &[&(*id as i32), &pos.x, &pos.y, &pos.z]).await {
            warn!("saving ship {id} position: {}", logging::chain(&e));
        }
        if let Some(fight) = fight {
            svc::inflight::save_vitals(&server.db, *id, fight, false).await;
        }
    }
    info!("saved {} live ships and drones", ships.len());
}

fn report_bulk(bulk: &bulkdata::BulkData, started: Instant) {
    let mut bytes = 0usize;
    let mut empty = Vec::new();
    for spec in bulkdata::SPECS {
        let e = &bulk.entries[spec.key];
        bytes += e.pickle.len();
        if e.rows == 0 {
            empty.push(spec.key);
        }
        if e.pickle.len() > BULK_WARN_BYTES {
            warn!("bulk object {} is {} bytes, near the 1 MB frame cap", spec.key, e.pickle.len());
        }
    }
    info!(
        "bulk data: {} objects, {} rows, {:.1} MB on the wire, built in {:.1}s",
        bulk.entries.len(),
        bulk.entries.values().map(|e| e.rows).sum::<usize>(),
        bytes as f64 / 1e6,
        started.elapsed().as_secs_f64()
    );
    if !empty.is_empty() {
        debug!("{} bulk object(s) served with no rows: {}", empty.len(), empty.join(", "));
    }
}
