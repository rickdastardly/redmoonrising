//! Make the published inventory catalog browseable and stocked at NPC markets.
//! The imported market has broad regional stock but omits many variants whose
//! invTypes row has no market group, especially faction modules and charges.

use tokio_postgres::{Client, Error};
use tracing::info;

const FIRST_PLAYER_ID: i32 = 100_000_000;
const NPC_MARKET_CORP_ID: i32 = 1_000_002;
const CATEGORIES: &[i32] = &[4, 5, 6, 7, 8, 9, 16, 17, 18, 20, 22, 23, 24, 25];

pub async fn ensure(db: &mut Client) -> Result<(), Error> {
    let tx = db.transaction().await?;

    // The old catalog marks many real Tech II ships, drones and modules as
    // unpublished. Orders alone cannot make them appear in the client. Expose
    // every inventory item in the marketable categories, including test hulls
    // until the special-ship catalog gets its own policy.
    let published = tx.execute(
        r#"UPDATE "invTypes" t SET "published" = 1
             FROM "invGroups" g
            WHERE g."groupID" = t."groupID" AND t."published" = 0
              AND g."categoryID" = ANY($1)"#,
        &[&CATEGORIES],
    ).await?;

    // Variants belong beside their Tech I parent whenever that parent has a
    // market group. A group with only one market category is also unambiguous.
    let parent_group = r#"UPDATE "invTypes" t SET "marketGroupID" = p."marketGroupID"
                           FROM "invMetaTypes" m
                           JOIN "invTypes" p ON p."typeID" = m."parentTypeID"
                          WHERE t."typeID" = m."typeID" AND t."marketGroupID" IS NULL
                            AND t."published" = 1 AND p."marketGroupID" IS NOT NULL
                            AND (SELECT "categoryID" FROM "invGroups" WHERE "groupID" = t."groupID") = ANY($1)"#;
    let mut categorized = tx.execute(parent_group, &[&CATEGORIES]).await?;
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

    tx.commit().await?;
    info!("market: published {published} requested types, categorized {categorized} types, seeded {seeded} missing regional NPC sell orders");
    Ok(())
}
