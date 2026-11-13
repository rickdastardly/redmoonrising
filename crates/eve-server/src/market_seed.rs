//! Make the published inventory catalog browseable and stocked at NPC markets.
//! The imported market has broad regional stock but omits many variants whose
//! invTypes row has no market group, especially faction modules and charges.

use tokio_postgres::{Client, Error};
use tracing::info;

const FIRST_PLAYER_ID: i32 = 100_000_000;
const NPC_MARKET_CORP_ID: i32 = 1_000_002;
const ISOTOPE_STOCK: i32 = 100_000_000;
const ISOTOPE_TYPES: &[i32] = &[16258, 16259, 16260, 16261, 16274, 17887, 17888, 17889];
const CATEGORIES: &[i32] = &[4, 5, 6, 7, 8, 9, 16, 17, 18, 20, 22, 23, 24, 25];

/// The 2007 market tree: (typeID, marketGroupID) for every type the source
/// data placed in the market browser.
fn market_tree_2007() -> Vec<(i32, i32)> {
    include_str!("market_tree_2007.tsv")
        .lines()
        .filter_map(|line| {
            let (t, g) = line.trim().split_once('\t')?;
            Some((t.parse().ok()?, g.parse().ok()?))
        })
        .collect()
}

/// Items Red Moon Rising never released to players, kept off the market:
/// anything named "test" (TEST Smartbomb, Uber Test towers...), the NPC boss
/// placeholders ("Serpentis Bship Boss Rank1 Armor EM Hardener"; the real
/// ones are the Shadow Serpentis modules), dummy, unused and OLD developer
/// copies, GM and "Pimp" modules, CONCORD and Polaris NPC hulls, Jove ships,
/// placeholder hulls and drones, and skills never put into play. Faction,
/// officer and deadspace gear was released (it drops in DED complexes) and
/// stays; so do real items with "test" in the name.
const DEVELOPER_ITEM: &str = r#"(
       (t."typeName" ~* 'test' AND t."typeName" !~* 'protest'
        AND t."typeName" NOT IN ('Test Dummies', 'Serpentis Test Reports'))
    OR t."typeName" ~* 'boss rank ?[0-9]'
    OR t."typeName" ~* '(^|[^a-z])(dummy|unused|placeholder|deprecated|obsolete)([^a-z]|$)'
    OR t."typeName" ~* '^(gm|pimp) '
    OR t."typeName" ~ '(^| )OLD( |$)'
    OR ((SELECT ig."categoryID" FROM "invGroups" ig WHERE ig."groupID" = t."groupID") IN (6, 16, 18)
        AND t."typeName" ~* '(^|[^a-z])(concord|polaris|police|swat|peacekeeper|enforcer|army|special ops)([^a-z]|$)')
    OR ((SELECT ig."categoryID" FROM "invGroups" ig WHERE ig."groupID" = t."groupID") = 6
        AND t."typeName" ~* '^elite |^faction prophecy|^tier3 |bc tier 2|much hp|^immovable|^infested|corvette prototype|^scorpion [0-9]$|^rook [0-9]$|apocalypse the hammer|^soct|arbitrator house|^capsule (home|cp)|^jove ')
    OR ((SELECT ig."categoryID" FROM "invGroups" ig WHERE ig."groupID" = t."groupID") = 18
        AND t."typeName" ~* 'lewt drone|horrible tracking|fighter uno|webscramblifying|^acolyte1$')
    OR ((SELECT ig."categoryID" FROM "invGroups" ig WHERE ig."groupID" = t."groupID") = 16
        AND t."typeName" ~* '^jove |fake skill|^omnipotent$|^(media|chief science officer|public relations|intelligence analyst|amarr tech|caldari tech|gallente tech|minmatar tech|duplicating|hypernet science|black market trading|smuggling|propulsion (optimal range|fall off|ws strength|stasis strength)|structural analyzis|skunk cruise missiles)$'))"#;

pub async fn ensure(db: &mut Client) -> Result<(), Error> {
    // A freshly imported universe has no planner statistics yet, and without
    // them the regional seeding below can run for many minutes. ANALYZE takes
    // seconds and makes it quick.
    info!("market: checking NPC sell orders in every region (a first start takes longer)");
    db.batch_execute(r#"ANALYZE market_orders; ANALYZE "invTypes"; ANALYZE "invGroups"; ANALYZE "invMetaTypes"; ANALYZE "staStations""#).await?;
    let tx = db.transaction().await?;

    // The old catalog marks many real Tech II ships, drones and modules as
    // unpublished. Orders alone cannot make them appear in the client. Expose
    // every inventory item in the marketable categories, including test hulls
    // until the special-ship catalog gets its own policy.
    let published = tx.execute(
        format!(r#"UPDATE "invTypes" t SET "published" = 1
             FROM "invGroups" g
            WHERE g."groupID" = t."groupID" AND t."published" = 0
              AND g."categoryID" = ANY($1) AND NOT {DEVELOPER_ITEM}"#).as_str(),
        &[&CATEGORIES],
    ).await?;

    // Developer items come off the market: unpublished, out of the market
    // browser, and their NPC sell orders withdrawn. Players keep any they own.
    let hidden = tx.execute(
        format!(r#"UPDATE "invTypes" t SET "published" = 0, "marketGroupID" = NULL
             FROM "invGroups" g
            WHERE g."groupID" = t."groupID" AND g."categoryID" = ANY($1)
              AND (t."published" = 1 OR t."marketGroupID" IS NOT NULL) AND {DEVELOPER_ITEM}"#).as_str(),
        &[&CATEGORIES],
    ).await?;
    let mut withdrawn = tx.execute(
        format!(r#"DELETE FROM market_orders o USING "invTypes" t
            WHERE t."typeID" = o."typeID" AND o."charID" < $1 AND {DEVELOPER_ITEM}"#).as_str(),
        &[&FIRST_PLAYER_ID],
    ).await?;
    // The bot traders' orders too, once their table exists.
    let bot_orders: bool = tx.query_one(r#"SELECT to_regclass('public."rmrBotMarketOrders"') IS NOT NULL"#, &[]).await?.get(0);
    if bot_orders {
        withdrawn += tx.execute(
            format!(r#"DELETE FROM market_orders o USING "invTypes" t
                WHERE t."typeID" = o."typeID" AND {DEVELOPER_ITEM}
                  AND o."orderID" IN (SELECT "orderID" FROM "rmrBotMarketOrders")"#).as_str(),
            &[],
        ).await?;
    }
    if hidden > 0 || withdrawn > 0 {
        info!("market: {hidden} unreleased item type(s) taken off the market, {withdrawn} NPC order(s) withdrawn");
    }

    // Every item keeps the market group the 2007 data gives it. Released
    // items the 2007 tree has no place for (faction, officer and deadspace
    // modules, pirate faction ships) go beside their Tech I versions, as
    // before; anything else left over goes to Additional Items below. Items
    // an earlier start put in Additional Items are looked at again.
    let (tree_types, tree_groups): (Vec<i32>, Vec<i32>) = market_tree_2007().into_iter().unzip();
    let mut categorized = tx.execute(
        format!(r#"UPDATE "invTypes" t SET "marketGroupID" = r.mg
             FROM unnest($1::int[], $2::int[]) AS r(id, mg)
            WHERE t."typeID" = r.id AND t."marketGroupID" IS DISTINCT FROM r.mg AND NOT {DEVELOPER_ITEM}"#).as_str(),
        &[&tree_types, &tree_groups],
    ).await?;
    tx.execute(r#"UPDATE "invTypes" SET "marketGroupID" = NULL WHERE "marketGroupID" >= 1000000"#, &[]).await?;

    // Variants belong beside their Tech I parent whenever that parent has a
    // market group. A group with only one market category is also unambiguous.
    let parent_group = r#"UPDATE "invTypes" t SET "marketGroupID" = p."marketGroupID"
                           FROM "invMetaTypes" m
                           JOIN "invTypes" p ON p."typeID" = m."parentTypeID"
                          WHERE t."typeID" = m."typeID" AND t."marketGroupID" IS NULL
                            AND t."published" = 1 AND p."marketGroupID" IS NOT NULL
                            AND (SELECT "categoryID" FROM "invGroups" WHERE "groupID" = t."groupID") = ANY($1)"#;
    categorized += tx.execute(parent_group, &[&CATEGORIES]).await?;
    categorized += tx.execute(
        r#"WITH peer AS (
               SELECT "groupID", min("marketGroupID") AS market_group
                 FROM "invTypes" WHERE "marketGroupID" IS NOT NULL
                GROUP BY "groupID" HAVING count(DISTINCT "marketGroupID") = 1
           )
           UPDATE "invTypes" t SET "marketGroupID" = peer.market_group
             FROM peer JOIN "invGroups" g ON g."groupID" = peer."groupID"
            WHERE t."groupID" = peer."groupID" AND t."marketGroupID" IS NULL
              AND t."published" = 1 AND g."categoryID" = ANY($1)"#,
        &[&CATEGORIES],
    ).await?;
    categorized += tx.execute(parent_group, &[&CATEGORIES]).await?;

    // The source data has no suitable peer for some published types. Stable
    // IDs keep these additional groups unchanged across server restarts.
    tx.execute(
        r#"INSERT INTO "invMarketGroups" ("marketGroupID", "parentGroupID", "marketGroupName", "hasTypes")
           VALUES (1000000, NULL, 'Additional Items', 0)
           ON CONFLICT ("marketGroupID") DO NOTHING"#,
        &[],
    ).await?;
    tx.execute(
        r#"INSERT INTO "invMarketGroups" ("marketGroupID", "parentGroupID", "marketGroupName", "hasTypes")
           SELECT 1100000 + c."categoryID", 1000000, c."categoryName", 0
             FROM "invCategories" c WHERE c."categoryID" = ANY($1)
           ON CONFLICT ("marketGroupID") DO NOTHING"#,
        &[&CATEGORIES],
    ).await?;
    tx.execute(
        r#"INSERT INTO "invMarketGroups" ("marketGroupID", "parentGroupID", "marketGroupName", "hasTypes")
           SELECT 1200000 + g."groupID", 1100000 + g."categoryID", g."groupName", 1
             FROM "invGroups" g
            WHERE g."categoryID" = ANY($1)
              AND EXISTS (SELECT 1 FROM "invTypes" t WHERE t."groupID" = g."groupID"
                            AND t."published" = 1 AND t."marketGroupID" IS NULL)
           ON CONFLICT ("marketGroupID") DO NOTHING"#,
        &[&CATEGORIES],
    ).await?;
    categorized += tx.execute(
        r#"UPDATE "invTypes" t SET "marketGroupID" = 1200000 + t."groupID"
             FROM "invGroups" g
            WHERE g."groupID" = t."groupID" AND g."categoryID" = ANY($1)
              AND t."published" = 1 AND t."marketGroupID" IS NULL"#,
        &[&CATEGORIES],
    ).await?;

    let issued = crate::bulkdata::filetime_now();
    let seeded = tx.execute(
        r#"WITH existing_prices AS (
               SELECT "typeID", min("price") AS price
                 FROM market_orders
                WHERE "charID" < $2 AND "bid" = 0 AND "orderState" = 0 AND "volRemaining" > 0
                GROUP BY "typeID"
           ), group_prices AS (
               SELECT t."groupID", percentile_cont(0.5) WITHIN GROUP (ORDER BY p.price) AS price
                 FROM "invTypes" t JOIN existing_prices p ON p."typeID" = t."typeID"
                GROUP BY t."groupID"
           ), category_prices AS (
               SELECT g."categoryID", percentile_cont(0.5) WITHIN GROUP (ORDER BY p.price) AS price
                 FROM "invTypes" t JOIN "invGroups" g ON g."groupID" = t."groupID"
                 JOIN existing_prices p ON p."typeID" = t."typeID"
                GROUP BY g."categoryID"
           ), station_stock AS (
               SELECT "stationID", count(*) AS listings
                 FROM market_orders
                WHERE "charID" < $2 AND "bid" = 0 AND "orderState" = 0 AND "volRemaining" > 0
                GROUP BY "stationID"
           ), hubs AS (
               SELECT DISTINCT ON (s."regionID") s."regionID", s."stationID", s."solarSystemID", s."corporationID"
                 FROM "staStations" s LEFT JOIN station_stock n ON n."stationID" = s."stationID"
                ORDER BY s."regionID", coalesce(n.listings, 0) DESC,
                         (s."corporationID" < $2) DESC, s."stationID"
           ), catalog AS (
               SELECT t."typeID", g."categoryID", t."groupID", t."basePrice", m."metaGroupID",
                      own.price AS known_price, parent.price AS parent_price,
                      gp.price AS group_price, cp.price AS category_price
                 FROM "invTypes" t JOIN "invGroups" g ON g."groupID" = t."groupID"
                 LEFT JOIN "invMetaTypes" m ON m."typeID" = t."typeID"
                 LEFT JOIN existing_prices own ON own."typeID" = t."typeID"
                 LEFT JOIN existing_prices parent ON parent."typeID" = m."parentTypeID"
                 LEFT JOIN group_prices gp ON gp."groupID" = t."groupID"
                 LEFT JOIN category_prices cp ON cp."categoryID" = g."categoryID"
                WHERE t."published" = 1 AND g."categoryID" = ANY($1)
           )
           INSERT INTO market_orders
               ("typeID", "charID", "regionID", "stationID", "range", "bid", "price",
                "volEntered", "volRemaining", "issued", "orderState", "minVolume",
                "contraband", "accountID", "duration", "isCorp", "solarSystemID", "escrow", "jumps")
           SELECT c."typeID", CASE WHEN h."corporationID" < $2 THEN h."corporationID" ELSE $4 END,
                  h."regionID", h."stationID", -1, 0,
                  greatest(0.01, coalesce(c.known_price,
                      c.parent_price * CASE c."metaGroupID"
                          WHEN 2 THEN 3.0 WHEN 3 THEN 5.0 WHEN 4 THEN 10.0
                          WHEN 5 THEN 25.0 WHEN 6 THEN 15.0 ELSE 1.0 END,
                      CASE WHEN c."basePrice" > 0 THEN c."basePrice" END,
                      c.group_price, c.category_price, 1000000.0)),
                  1000000, 1000000, $3, 0, 1, 0, 1000, 3650, 0, h."solarSystemID", 0, 1
             FROM catalog c CROSS JOIN hubs h
            WHERE NOT EXISTS (
                SELECT 1 FROM market_orders o
                 WHERE o."typeID" = c."typeID" AND o."regionID" = h."regionID"
                   AND o."charID" < $2 AND o."bid" = 0 AND o."orderState" = 0
                   AND o."volRemaining" > 0)"#,
        &[&CATEGORIES, &FIRST_PLAYER_ID, &issued, &NPC_MARKET_CORP_ID],
    ).await?;

    // Capital jump and doomsday fuel must remain buyable even after a pilot
    // empties the original one-million-unit NPC listing. Keep each regional
    // NPC sell order stocked at startup; player orders are never changed.
    let restocked = tx.execute(
        r#"UPDATE market_orders SET "volRemaining" = $1,
                  "volEntered" = greatest("volEntered", $1)
             WHERE "typeID" = ANY($2) AND "charID" < $3 AND "bid" = 0
               AND "orderState" = 0 AND "volRemaining" < $1"#,
        &[&ISOTOPE_STOCK, &ISOTOPE_TYPES, &FIRST_PLAYER_ID],
    ).await?;

    tx.commit().await?;
    info!("market: published {published} requested types, categorized {categorized} types, seeded {seeded} missing regional NPC sell orders, restocked {restocked} isotope orders");

    // Stations chosen on the dashboard (usually freshly built outposts) are
    // kept fully stocked across restarts.
    db.batch_execute(SEEDED_TABLE).await?;
    let chosen: Vec<i32> = db.query(r#"SELECT "stationID" FROM "rmrSeededStations""#, &[]).await?
        .iter().map(|r| r.get(0)).collect();
    if !chosen.is_empty() {
        let (added, topped) = stock_stations(db, &chosen).await?;
        info!("market: {} dashboard-seeded stations: added {added} NPC sell orders, restocked {topped}", chosen.len());
    }
    Ok(())
}

const SEEDED_TABLE: &str = r#"CREATE TABLE IF NOT EXISTS "rmrSeededStations" (
    "stationID" integer PRIMARY KEY,
    seeded_ms bigint NOT NULL
)"#;
const STATION_STOCK: i32 = 1_000_000;

/// Put an NPC sell order for every marketable type at each station (priced
/// like the regional seed) and top existing NPC sell orders there back up.
async fn stock_stations(db: &Client, stations: &[i32]) -> Result<(u64, u64), Error> {
    let issued = crate::bulkdata::filetime_now();
    let added = db.execute(
        r#"WITH existing_prices AS (
               SELECT "typeID", min("price") AS price
                 FROM market_orders
                WHERE "charID" < $2 AND "bid" = 0 AND "orderState" = 0 AND "volRemaining" > 0
                GROUP BY "typeID"
           ), group_prices AS (
               SELECT t."groupID", percentile_cont(0.5) WITHIN GROUP (ORDER BY p.price) AS price
                 FROM "invTypes" t JOIN existing_prices p ON p."typeID" = t."typeID"
                GROUP BY t."groupID"
           ), category_prices AS (
               SELECT g."categoryID", percentile_cont(0.5) WITHIN GROUP (ORDER BY p.price) AS price
                 FROM "invTypes" t JOIN "invGroups" g ON g."groupID" = t."groupID"
                 JOIN existing_prices p ON p."typeID" = t."typeID"
                GROUP BY g."categoryID"
           ), targets AS (
               SELECT s."regionID", s."stationID", s."solarSystemID", s."corporationID"
                 FROM "staStations" s WHERE s."stationID" = ANY($5::int[])
           ), catalog AS (
               SELECT t."typeID", t."basePrice", m."metaGroupID",
                      own.price AS known_price, parent.price AS parent_price,
                      gp.price AS group_price, cp.price AS category_price
                 FROM "invTypes" t JOIN "invGroups" g ON g."groupID" = t."groupID"
                 LEFT JOIN "invMetaTypes" m ON m."typeID" = t."typeID"
                 LEFT JOIN existing_prices own ON own."typeID" = t."typeID"
                 LEFT JOIN existing_prices parent ON parent."typeID" = m."parentTypeID"
                 LEFT JOIN group_prices gp ON gp."groupID" = t."groupID"
                 LEFT JOIN category_prices cp ON cp."categoryID" = g."categoryID"
                WHERE t."published" = 1 AND t."marketGroupID" IS NOT NULL AND g."categoryID" = ANY($1)
           )
           INSERT INTO market_orders
               ("typeID", "charID", "regionID", "stationID", "range", "bid", "price",
                "volEntered", "volRemaining", "issued", "orderState", "minVolume",
                "contraband", "accountID", "duration", "isCorp", "solarSystemID", "escrow", "jumps")
           SELECT c."typeID", CASE WHEN h."corporationID" < $2 THEN h."corporationID" ELSE $4 END,
                  h."regionID", h."stationID", -1, 0,
                  greatest(0.01, coalesce(c.known_price,
                      c.parent_price * CASE c."metaGroupID"
                          WHEN 2 THEN 3.0 WHEN 3 THEN 5.0 WHEN 4 THEN 10.0
                          WHEN 5 THEN 25.0 WHEN 6 THEN 15.0 ELSE 1.0 END,
                      CASE WHEN c."basePrice" > 0 THEN c."basePrice" END,
                      c.group_price, c.category_price, 1000000.0)),
                  1000000, 1000000, $3, 0, 1, 0, 1000, 3650, 0, h."solarSystemID", 0, 1
             FROM catalog c CROSS JOIN targets h
            WHERE NOT EXISTS (
                SELECT 1 FROM market_orders o
                 WHERE o."typeID" = c."typeID" AND o."stationID" = h."stationID"
                   AND o."charID" < $2 AND o."bid" = 0 AND o."orderState" = 0
                   AND o."volRemaining" > 0)"#,
        &[&CATEGORIES, &FIRST_PLAYER_ID, &issued, &NPC_MARKET_CORP_ID, &stations],
    ).await?;
    let topped = db.execute(
        r#"UPDATE market_orders SET "volRemaining" = $1, "volEntered" = greatest("volEntered", $1)
            WHERE "stationID" = ANY($2::int[]) AND "charID" < $3 AND "bid" = 0
              AND "orderState" = 0 AND "volRemaining" < $1"#,
        &[&STATION_STOCK, &stations, &FIRST_PLAYER_ID],
    ).await?;
    Ok((added, topped))
}

/// Dashboard: stock a station's market with NPC sell orders and keep it
/// stocked at every start. Returns (orders added, orders restocked).
pub async fn seed_station(db: &Client, station: i32) -> Result<(u64, u64), Error> {
    db.batch_execute(SEEDED_TABLE).await?;
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64).unwrap_or(0);
    db.execute(
        r#"INSERT INTO "rmrSeededStations" ("stationID", seeded_ms) VALUES ($1, $2)
           ON CONFLICT ("stationID") DO UPDATE SET seeded_ms = EXCLUDED.seeded_ms"#,
        &[&station, &now],
    ).await?;
    let result = stock_stations(db, &[station]).await?;
    info!(station, added = result.0, restocked = result.1, "market: dashboard seeded station");
    Ok(result)
}

/// Dashboard: stop restocking a station. On a player-owned station (an
/// outpost) every NPC sell order came from seeding, so those are removed too.
pub async fn unseed_station(db: &Client, station: i32) -> Result<u64, Error> {
    db.batch_execute(SEEDED_TABLE).await?;
    db.execute(r#"DELETE FROM "rmrSeededStations" WHERE "stationID" = $1"#, &[&station]).await?;
    let removed = db.execute(
        r#"DELETE FROM market_orders o USING "staStations" s
            WHERE s."stationID" = o."stationID" AND o."stationID" = $1
              AND s."corporationID" >= $2 AND o."charID" < $2 AND o."bid" = 0"#,
        &[&station, &FIRST_PLAYER_ID],
    ).await?;
    info!(station, removed, "market: dashboard stopped seeding station");
    Ok(removed)
}

/// Dashboard: the seeded stations.
pub async fn seeded_ids(db: &Client) -> Result<Vec<i32>, Error> {
    db.batch_execute(SEEDED_TABLE).await?;
    Ok(db.query(r#"SELECT "stationID" FROM "rmrSeededStations""#, &[]).await?
        .iter().map(|r| r.get(0)).collect())
}

pub const FIRST_PLAYER: i32 = FIRST_PLAYER_ID;
