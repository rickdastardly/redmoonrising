//! The medical bay (medical.py): a character's clone and where it waits.
//!
//! The clone is an item of the Clone group the character owns, located on
//! the character. Its type is the grade (how many skill points it keeps);
//! its customInfo names the home station, where the pilot wakes when the
//! capsule is lost. Without one the pilot has an Alpha clone at their
//! corporation's station.

use eve_marshal::Value;
use tokio_postgres::{Client, GenericClient};
use tracing::{error, info};

use crate::combat::attr;
use crate::services::{Ctx, Outcome};

pub const GROUP_CLONE: i32 = 23;
pub const TYPE_CLONE_ALPHA: i64 = 164;
/// market_refTypes: "Clone Activation".
const REF_CLONE: i32 = 8;

const SIGMA_CLONE: i32 = 23734;
const LEGACY_CLONE_PRICES: &[(i32, f64)] = &[
    (164, 1_000.0), (165, 2_000.0), (166, 3_000.0),
    (9917, 4_000.0), (9919, 8_000.0), (9921, 12_000.0),
    (9923, 16_000.0), (9925, 32_000.0), (9927, 48_000.0),
    (9929, 64_000.0), (9931, 128_000.0), (9933, 192_000.0),
    (9935, 256_000.0), (9937, 512_000.0), (9939, 768_000.0),
    (16715, 1_000_000.0), (16718, 4_000_000.0),
    (SIGMA_CLONE, 12_000_000.0),
];

/// The remaining Greek grades extend the original 92,160,000 SP Sigma clone.
/// The final grade holds the skill points for every skill at level V.
const NEW_CLONE_GRADES: &[(i32, &str, i32, f64)] = &[
    (90_000_001, "Clone Grade Tau",     120_000_000, 4.0 / 3.0),
    (90_000_002, "Clone Grade Upsilon", 156_000_000, 7.0 / 4.0),
    (90_000_003, "Clone Grade Phi",     203_000_000, 7.0 / 3.0),
    (90_000_004, "Clone Grade Chi",     264_000_000, 37.0 / 12.0),
    (90_000_005, "Clone Grade Psi",     343_000_000, 49.0 / 12.0),
    (90_000_006, "Clone Grade Omega",   448_386_560, 65.0 / 12.0),
];

/// Keep both dogma tables in sync: the old client reads dgmTypeAttribs in
/// bulk data, while capsule loss uses dgmTypeAttributes on the server.
pub async fn ensure_grades(db: &mut Client) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let tx = db.transaction().await?;
    for &(type_id, price) in LEGACY_CLONE_PRICES {
        tx.execute(r#"UPDATE "invTypes" SET "basePrice" = $2
            WHERE "typeID" = $1 AND "groupID" = $3
              AND COALESCE("basePrice", 0) <= 0"#,
            &[&type_id, &price, &GROUP_CLONE]).await?;
    }
    let sigma_price: f64 = tx.query_one(r#"SELECT "basePrice" FROM "invTypes"
        WHERE "typeID" = $1 AND "groupID" = $2"#,
        &[&SIGMA_CLONE, &GROUP_CLONE]).await?.get(0);
    let sigma_price = sigma_price.max(12_000_000.0);

    for &(type_id, name, points, multiplier) in NEW_CLONE_GRADES {
        let price = (sigma_price * multiplier).ceil();
        let description = format!("This medical clone preserves up to {points} skill points.");
        let changed = tx.execute(r#"INSERT INTO "invTypes"
            ("typeID", "groupID", "typeName", description, "graphicID", radius, mass,
             volume, capacity, "portionSize", "basePrice", published, "chanceOfDuplicating")
            VALUES ($1, $2, $3, $4, 34, 0, 0, 1, 0, 1, $5, 1, 0)
            ON CONFLICT ("typeID") DO UPDATE SET "typeName" = EXCLUDED."typeName",
                description = EXCLUDED.description, "basePrice" = EXCLUDED."basePrice", published = 1
            WHERE "invTypes"."groupID" = $2 AND "invTypes"."typeName" = $3"#,
            &[&type_id, &GROUP_CLONE, &name, &description, &price]).await?;
        if changed != 1 {
            return Err(format!("clone grade type ID {type_id} is already used by another item").into());
        }
        tx.execute(r#"INSERT INTO "dgmTypeAttributes" ("typeID", "attributeID", "valueInt", "valueFloat")
            VALUES ($1, $2, $3, NULL)
            ON CONFLICT ("typeID", "attributeID") DO UPDATE SET
                "valueInt" = EXCLUDED."valueInt", "valueFloat" = NULL"#,
            &[&type_id, &attr::SKILL_POINTS_SAVED, &points]).await?;
        tx.execute(r#"INSERT INTO "dgmTypeAttribs" ("typeID", "attributeID", value)
            VALUES ($1, $2, $3)
            ON CONFLICT ("typeID", "attributeID") DO UPDATE SET value = EXCLUDED.value"#,
            &[&type_id, &attr::SKILL_POINTS_SAVED, &points.to_string()]).await?;
    }
    tx.commit().await?;
    info!(grades = NEW_CLONE_GRADES.len(), maximum_sp = 448_386_560, "medical clone grades ready");
    Ok(())
}

/// The station the clone waits in, if one was chosen.
pub async fn home_station(db: &impl GenericClient, char_id: i64) -> Result<Option<i64>, tokio_postgres::Error> {
    let row = db
        .query_opt(
            r#"SELECT e."customInfo" FROM entity e JOIN "invTypes" t ON t."typeID" = e."typeID"
                WHERE e."ownerID" = $1 AND e."locationID" = $1 AND t."groupID" = $2
                ORDER BY e."itemID" DESC LIMIT 1"#,
            &[&(char_id as i32), &GROUP_CLONE],
        )
        .await?;
    Ok(row.and_then(|r| r.get::<_, Option<String>>(0)).and_then(|s| s.trim().parse().ok()))
}

/// Gives the character a clone of `type_id` waiting at `home`, in place of
/// whatever clone it had.
pub async fn replace_clone(db: &impl GenericClient, char_id: i64, type_id: i64, home: Option<i64>) -> Result<(), tokio_postgres::Error> {
    db.execute(
        r#"DELETE FROM entity e USING "invTypes" t
            WHERE t."typeID" = e."typeID" AND e."ownerID" = $1 AND e."locationID" = $1 AND t."groupID" = $2"#,
        &[&(char_id as i32), &GROUP_CLONE],
    )
    .await?;
    db.execute(
        r#"INSERT INTO entity ("itemName","typeID","ownerID","locationID","flag","singleton","quantity","customInfo")
           VALUES ('', $2, $1, $1, 0, 1, 1, $3)"#,
        &[&(char_id as i32), &(type_id as i32), &home.map(|h| h.to_string())],
    )
    .await?;
    Ok(())
}

pub async fn clone_type(db: &impl GenericClient, char_id: i64) -> Result<Option<i64>, tokio_postgres::Error> {
    let row = db
        .query_opt(
            r#"SELECT e."typeID" FROM entity e JOIN "invTypes" t ON t."typeID" = e."typeID"
                WHERE e."ownerID" = $1 AND e."locationID" = $1 AND t."groupID" = $2 ORDER BY e."itemID" DESC LIMIT 1"#,
            &[&(char_id as i32), &GROUP_CLONE],
        )
        .await?;
    Ok(row.map(|r| r.get::<_, i32>(0) as i64))
}

/// A clone grade the pilot carries as an item (bought on the market, say)
/// that keeps more skill points than the clone in use is used up: it becomes
/// the pilot's clone, keeping the home station, and one leaves the hangar.
/// Returns the clone type now in use, if one was installed.
pub async fn install_carried_clone(ctx: &Ctx<'_>, char_id: i64) -> Result<Option<i64>, tokio_postgres::Error> {
    const SKILL_POINTS_SAVED: i32 = 419;
    let saved = |alias: &str| format!(
        r#"COALESCE((SELECT coalesce(a."valueFloat", a."valueInt"::float8) FROM "dgmTypeAttributes" a
                      WHERE a."typeID" = {alias}."typeID" AND a."attributeID" = {SKILL_POINTS_SAVED}), 0)"#
    );
    let sql = format!(
        r#"SELECT e."itemID", e."typeID", {carried}
             FROM entity e JOIN "invTypes" t ON t."typeID" = e."typeID"
            WHERE e."ownerID" = $1 AND e."locationID" <> $1 AND t."groupID" = $2 AND e."quantity" > 0
              AND {carried} > COALESCE((SELECT max({installed}) FROM entity i JOIN "invTypes" it ON it."typeID" = i."typeID"
                                         WHERE i."ownerID" = $1 AND i."locationID" = $1 AND it."groupID" = $2), 0)
            ORDER BY 3 DESC LIMIT 1"#,
        carried = saved("e"),
        installed = saved("i"),
    );
    let Some(best) = ctx.db.query_opt(sql.as_str(), &[&(char_id as i32), &GROUP_CLONE]).await? else { return Ok(None) };
    let (item_id, type_id) = (best.get::<_, i32>(0) as i64, best.get::<_, i32>(1) as i64);
    let mut client = crate::db::connect(ctx.db_url).await?;
    let mut txn = crate::items::ItemTxn::begin(&mut client).await?;
    let Some(item) = txn.item(item_id).await? else { return Ok(None) };
    if item.quantity > 1 {
        txn.set_quantity(item_id, item.quantity - 1).await?;
    } else {
        txn.destroy(item_id).await?;
    }
    let home = home_station(txn.tx(), char_id).await?;
    replace_clone(txn.tx(), char_id, type_id, home).await?;
    let events = txn.commit().await?;
    crate::items::notify(ctx.world, ctx.client_id, events);
    info!("{char_id} put clone {type_id} (item {item_id}) to use");
    Ok(Some(type_id))
}

/// character.GetHomeStation: the clone's station, else the corporation's.
pub async fn get_home_station(ctx: &Ctx<'_>) -> Value {
    let chosen = match ctx.session.char_id {
        Some(char_id) => home_station(ctx.db, char_id).await.unwrap_or_else(|e| {
            error!("character.GetHomeStation: {}", crate::logging::chain(&e));
            None
        }),
        None => None,
    };
    Value::int_or_none(chosen.or(ctx.session.hq_id).or(ctx.session.station_id))
}

/// corpStationMgr.SetHomeStation(stationID): move the clone to the station
/// the pilot is docked in.
pub async fn set_home_station(ctx: &Ctx<'_>, station_id: i64) -> Outcome {
    let Some(char_id) = ctx.session.char_id else { return Outcome::custom("No character.") };
    if ctx.session.station_id != Some(station_id) {
        return Outcome::custom("You can only move your clone to the station you are in.");
    }
    let result = async {
        let grade = clone_type(ctx.db, char_id).await?.unwrap_or(TYPE_CLONE_ALPHA);
        replace_clone(ctx.db, char_id, grade, Some(station_id)).await
    }
    .await;
    match result {
        Ok(()) => {
            info!("{char_id} moved their clone to {station_id}");
            Outcome::Value(Value::None)
        }
        Err(e) => {
            error!("SetHomeStation({station_id}): {}", crate::logging::chain(&e));
            Outcome::custom("Your clone could not be moved.")
        }
    }
}

/// corpStationMgr.SetCloneTypeID(typeID): buy a better clone; it keeps its
/// home station.
pub async fn set_clone_type(ctx: &Ctx<'_>, type_id: i64) -> Outcome {
    use crate::svc::wallet::{move_cash, Payment, Transfer};
    let Some(char_id) = ctx.session.char_id else { return Outcome::custom("No character.") };
    let sql = r#"SELECT "basePrice" FROM "invTypes" WHERE "typeID" = $1 AND "groupID" = $2"#;
    let Some(price) = ctx.query_one(sql, &[&(type_id as i32), &GROUP_CLONE]).await.map(|r| r.get::<_, f64>(0)) else {
        return Outcome::custom("That is not a clone.");
    };
    let arg = type_id.to_string();
    let payment = Payment { payer: char_id, payee: None, amount: price, ref_type: REF_CLONE, arg_id1: &arg, reason: "" };
    match move_cash(ctx.db_url, payment).await {
        Ok(()) => {}
        Err(Transfer::NotEnough { need, have }) => return Outcome::not_enough_money(need, have),
        Err(Transfer::NoSuchOwner) => return Outcome::custom("There is no wallet to pay from."),
        Err(Transfer::Db(e)) => {
            error!("buying clone {type_id}: {}", crate::logging::chain(&e));
            return Outcome::custom("The clone could not be paid for.");
        }
    }
    let result = async {
        let home = home_station(ctx.db, char_id).await?;
        replace_clone(ctx.db, char_id, type_id, home).await
    }
    .await;
    if let Err(e) = result {
        error!("installing clone {type_id} for {char_id}: {}", crate::logging::chain(&e));
        return Outcome::custom("The clone could not be installed.");
    }
    info!("{char_id} bought clone {type_id} for {price} ISK");
    Outcome::Value(Value::None)
}

#[cfg(test)]
mod tests {
    use super::{LEGACY_CLONE_PRICES, NEW_CLONE_GRADES};

    #[test]
    fn clone_capacity_and_price_increase_to_full_skill_cap() {
        assert!(LEGACY_CLONE_PRICES.windows(2).all(|pair| pair[0].1 < pair[1].1));
        let mut previous_sp = 92_160_000;
        let sigma_price = LEGACY_CLONE_PRICES.last().unwrap().1;
        let mut previous_price = sigma_price;
        for &(_, _, points, multiplier) in NEW_CLONE_GRADES {
            let price = (sigma_price * multiplier).ceil();
            assert!(points > previous_sp);
            assert!(price > previous_price);
            previous_sp = points;
            previous_price = price;
        }
        assert_eq!(previous_sp, 448_386_560);
    }
}
