//! Bot traders on the market. Every few minutes, while the bots run, trader
//! bots keep sell orders (and some buy orders) at the busiest hubs and at the
//! stations where players are docked, for everything the NPC market there
//! sells: materials, ore, ships, modules, ammunition, drones, implants and
//! deployables. Their prices sit around the NPC asks, a little over or under,
//! so the market looks like players trade on it. Each order stands for one to
//! three days and is then taken down; new ones replace it.
//!
//! Players buy from and sell to these like any other order (`svc::trade`):
//! what a player pays goes to the bot's wallet, and what a player sells into a
//! bot's buy order goes to the bot's hangar. The dashboard sets how busy the
//! bot traders are (`bots::bot_market`); off stops new orders, and the ones up
//! are taken down as they lapse.

use std::sync::Arc;
use std::time::Duration;

use tokio_postgres::Client;
use tracing::{info, warn};

use crate::bots::Rate;
use crate::combat::Rng;
use crate::conn::Server;
use crate::svc::rows::{float, int};

const EVERY: Duration = Duration::from_secs(300);
const ROLE_TRADER: i16 = 3;
/// Characters below this are NPCs, whose orders are the reference prices.
const FIRST_PLAYER_ID: i32 = 100_000_000;
/// The busiest NPC market stations traded at, besides where players are docked.
const HUBS: i64 = 10;
const RANGE_STATION: i32 = -1;
const ORDER_OPEN: i32 = 0;
const ORDER_DAYS: i32 = 14;
/// A day in FILETIME units (100 ns).
const DAY: i64 = 864_000_000_000;
/// What bots trade: materials, ships, modules, charges, commodities, drones,
/// implants, deployables and ore.
const CATEGORIES: [i32; 9] = [4, 6, 7, 8, 17, 18, 20, 22, 25];
const CATEGORY_MATERIAL: i32 = 4;
const CATEGORY_SHIP: i32 = 6;
const CATEGORY_CHARGE: i32 = 8;
const CATEGORY_ASTEROID: i32 = 25;

/// Orders kept per station (sell, buy) and the most added per round.
fn limits(rate: Rate) -> (usize, usize, usize) {
    match rate {
        Rate::Off => (0, 0, 0),
        Rate::Low => (120, 30, 25),
        Rate::Normal => (400, 100, 60),
        Rate::High => (1_000, 250, 150),
    }
}

pub async fn run(server: Arc<Server>) {
    let mut rng = Rng::seeded(crate::bulkdata::filetime_now() as u64 ^ 0x6d61_726b_6574);
    // Let the server and the bots start first.
    tokio::time::sleep(Duration::from_secs(90)).await;
    let mut ready = false;
    loop {
        if crate::bots::running_count() > 0 {
            match crate::db::connect(&server.db_url).await {
                Ok(db) => {
                    if !ready {
                        match ensure_table(&db).await {
                            Ok(()) => ready = true,
                            Err(e) => warn!("bots: market traders' table: {}", crate::logging::chain(&e)),
                        }
                    }
                    if ready {
                        if let Err(e) = round(&server, &db, &mut rng).await {
                            warn!("bots: market traders: {}", crate::logging::chain(&e));
                        }
                    }
                }
                Err(e) => warn!("bots: market traders cannot reach the database: {}", crate::logging::chain(&e)),
            }
        }
        tokio::time::sleep(EVERY).await;
    }
}

async fn ensure_table(db: &Client) -> Result<(), tokio_postgres::Error> {
    db.batch_execute(
        r#"CREATE TABLE IF NOT EXISTS "rmrBotMarketOrders" (
               "orderID" integer PRIMARY KEY,
               "stationID" integer NOT NULL,
               "bid" smallint NOT NULL,
               "expires" bigint NOT NULL
           );
           CREATE INDEX IF NOT EXISTS "rmrBotMarketOrders_station" ON "rmrBotMarketOrders" ("stationID");"#,
    )
    .await
}

async fn round(server: &Server, db: &Client, rng: &mut Rng) -> Result<(), tokio_postgres::Error> {
    let now = crate::bulkdata::filetime_now();
    // Orders that have stood their time come down; filled ones are forgotten.
    let retired = db
        .execute(
            r#"WITH done AS (
                   DELETE FROM "rmrBotMarketOrders" b
                    WHERE b."expires" < $1
                       OR NOT EXISTS (SELECT 1 FROM market_orders o WHERE o."orderID" = b."orderID" AND o."orderState" = 0)
                RETURNING b."orderID"
               )
               DELETE FROM market_orders o USING done WHERE o."orderID" = done."orderID" AND o."orderState" = 0"#,
            &[&now],
        )
        .await?;
    let rate = crate::bots::bot_market();
    let (sell_cap, buy_cap, per_round) = limits(rate);
    if per_round == 0 || !crate::bots::is_running() {
        return Ok(());
    }
    let traders: Vec<i32> = db
        .query(r#"SELECT "characterID" FROM "bots" WHERE "role" = $1 ORDER BY random() LIMIT 60"#, &[&ROLE_TRADER])
        .await?
        .iter()
        .filter_map(|r| int(r, 0))
        .map(|id| id as i32)
        .collect();
    if traders.is_empty() {
        return Ok(());
    }
    let mut stations: Vec<i64> = db
        .query(
            r#"SELECT "stationID" FROM market_orders WHERE "bid" = 0 AND "orderState" = 0 AND "charID" < $1
                GROUP BY "stationID" ORDER BY count(*) DESC LIMIT $2"#,
            &[&FIRST_PLAYER_ID, &HUBS],
        )
        .await?
        .iter()
        .filter_map(|r| int(r, 0))
        .collect();
    let docked: Vec<i64> = server.world.lock().players().iter().filter_map(|p| p.station_id).collect();
    for s in docked {
        if !stations.contains(&s) {
            stations.push(s);
        }
    }
    let mut placed = 0;
    for station in stations {
        placed += stock_station(db, rng, &traders, station, now, sell_cap, buy_cap, per_round).await?;
    }
    if placed > 0 || retired > 0 {
        info!("bots: market traders placed {placed} order(s) and took down {retired}");
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn stock_station(
    db: &Client,
    rng: &mut Rng,
    traders: &[i32],
    station: i64,
    now: i64,
    sell_cap: usize,
    buy_cap: usize,
    per_round: usize,
) -> Result<usize, tokio_postgres::Error> {
    let station = station as i32;
    let Some(place) = db.query_opt(r#"SELECT "solarSystemID", "regionID" FROM "staStations" WHERE "stationID" = $1"#, &[&station]).await? else {
        return Ok(0);
    };
    let (system, region) = (int(&place, 0).unwrap_or(0) as i32, int(&place, 1).unwrap_or(0) as i32);
    // What the bots already have up here: (type, buying).
    let mine: Vec<(i64, bool)> = db
        .query(
            r#"SELECT o."typeID", o."bid" <> 0 FROM "rmrBotMarketOrders" b JOIN market_orders o ON o."orderID" = b."orderID"
                WHERE b."stationID" = $1::int AND o."orderState" = 0"#,
            &[&station],
        )
        .await?
        .iter()
        .map(|r| (int(r, 0).unwrap_or(0), r.get::<_, bool>(1)))
        .collect();
    let selling = mine.iter().filter(|(_, b)| !b).count();
    let buying = mine.len() - selling;
    let room = (sell_cap.saturating_sub(selling) + buy_cap.saturating_sub(buying)).min(per_round);
    if room == 0 {
        return Ok(0);
    }
    // What the NPC market in the region sells, at its lowest ask.
    let categories = CATEGORIES.to_vec();
    let goods = db
        .query(
            r#"SELECT o."typeID", min(o."price")::float8, g."categoryID"
                 FROM market_orders o
                 JOIN "invTypes" t ON t."typeID" = o."typeID"
                 JOIN "invGroups" g ON g."groupID" = t."groupID"
                WHERE o."regionID" = $1::int AND o."bid" = 0 AND o."orderState" = 0 AND o."charID" < $2::int AND o."price" > 0
                  AND t."published" = 1 AND g."categoryID" = ANY($3::int[])
                GROUP BY o."typeID", g."categoryID"
                ORDER BY random() LIMIT $4::bigint"#,
            &[&region, &FIRST_PLAYER_ID, &categories, &((room * 3) as i64)],
        )
        .await?;
    let mut sells_left = sell_cap.saturating_sub(selling);
    let mut buys_left = buy_cap.saturating_sub(buying);
    let mut placed = 0;
    for r in &goods {
        if placed >= room || (sells_left == 0 && buys_left == 0) {
            break;
        }
        let (type_id, reference, category) = (int(r, 0).unwrap_or(0), float(r, 1).unwrap_or(0.0), int(r, 2).unwrap_or(0) as i32);
        if type_id <= 0 || reference <= 0.0 {
            continue;
        }
        // A buy order now and then, where the bots are not buying it already.
        let buy = buys_left > 0 && (sells_left == 0 || rng.unit() < 0.25) && !mine.contains(&(type_id, true));
        if !buy && (sells_left == 0 || mine.contains(&(type_id, false))) {
            continue;
        }
        let price = tidy(reference * if buy { rng.range(0.55, 0.85) } else { rng.range(0.9, 1.12) });
        let volume = volume_for(category, price, buy, rng);
        let escrow = if buy { price * volume as f64 } else { 0.0 };
        let trader = traders[rng.below(traders.len())];
        let bid: i16 = i16::from(buy);
        let row = db
            .query_one(
                r#"INSERT INTO market_orders
                       ("typeID","charID","regionID","stationID","range","bid","price","volEntered","volRemaining","issued",
                        "orderState","minVolume","contraband","accountID","duration","isCorp","solarSystemID","escrow","jumps")
                   VALUES ($1::int, $2::int, $3::int, $4::int, $5::int, $6::smallint, $7::float8, $8::int, $8::int, $9::bigint,
                           $10::int, 1, 0, 1000, $11::int, 0, $12::int, $13::float8, 1)
                   RETURNING "orderID""#,
                &[&(type_id as i32), &trader, &region, &station, &RANGE_STATION, &bid, &price, &volume, &now, &ORDER_OPEN, &ORDER_DAYS, &system, &escrow],
            )
            .await?;
        let order: i32 = row.get(0);
        let expires = now + (DAY as f64 * rng.range(1.0, 3.0)) as i64;
        db.execute(
            r#"INSERT INTO "rmrBotMarketOrders" ("orderID", "stationID", "bid", "expires") VALUES ($1::int, $2::int, $3::smallint, $4::bigint)"#,
            &[&order, &station, &bid, &expires],
        )
        .await?;
        if buy {
            buys_left -= 1;
        } else {
            sells_left -= 1;
        }
        placed += 1;
    }
    Ok(placed)
}

/// How many units an order is for: a few ships, racks of modules, piles of
/// ammunition and mountains of minerals.
fn volume_for(category: i32, price: f64, buy: bool, rng: &mut Rng) -> i32 {
    fn worth(rng: &mut Rng, price: f64, lo: f64, hi: f64, min: f64, max: f64) -> i32 {
        (rng.range(lo, hi) / price.max(0.01)).round().clamp(min, max) as i32
    }
    let n = match category {
        CATEGORY_SHIP => 1 + rng.below(if buy { 2 } else { 4 }) as i32,
        CATEGORY_CHARGE => worth(rng, price, 2.0e6, 8.0e6, 100.0, 50_000.0),
        CATEGORY_MATERIAL | CATEGORY_ASTEROID => worth(rng, price, 5.0e6, 30.0e6, 1_000.0, 2_000_000.0),
        _ => worth(rng, price, 5.0e6, 40.0e6, 1.0, 200.0),
    };
    if buy { (n / 2).max(1) } else { n.max(1) }
}

/// A price as a trader would type it: four significant figures, cents below 100.
fn tidy(price: f64) -> f64 {
    if price < 100.0 {
        return (price * 100.0).round().max(1.0) / 100.0;
    }
    let step = 10f64.powi(price.log10().floor() as i32 - 3);
    (price / step).round() * step
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prices_are_tidied() {
        assert_eq!(tidy(12.3456), 12.35);
        assert_eq!(tidy(1_234_567.0), 1_235_000.0);
    }
}
