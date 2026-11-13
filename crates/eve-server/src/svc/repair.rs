//! `repairSvc`: the station repair shop (base_repairshop.py), bound per station,
//! and repackaging from the item menu (menusvc.py UnasembleItems), unbound.
//!
//! A quote covers an item's armor (`armorDamage`) and structure (`damage`).
//! There is no repair pricing data, so repairs cost nothing.

use eve_marshal::Value;
use tracing::info;

use crate::services::{Call, Ctx, Outcome};
use crate::svc::bound;
use crate::svc::inventory::{invitem_cells, item_change, INVITEM_SELECT};
use crate::svc::rows::{float, int, key_val};

const ATTR_DAMAGE: i32 = 3;
const ATTR_HP: i32 = 9;
const ATTR_ARMOR_DAMAGE: i32 = 266;
const ATTR_ARMOR_HP: i32 = 265;
/// Column position of `singleton` in an item row (const.ixSingleton).
const IX_SINGLETON: i64 = 6;

pub async fn repair_svc(call: &Call, ctx: &Ctx<'_>) -> Option<Outcome> {
    let station = bound::parse(&call.service).and_then(|b| b.params.first().copied());
    let value = match call.method.as_str() {
        "GetDamageReports" => damage_reports(ctx, &ids(call.args.first())).await,
        "RepairItems" => {
            repair(ctx, &ids(call.args.first())).await;
            Value::None
        }
        // UnasembleItem(itemID), on the station's repair shop.
        "UnasembleItem" => {
            let station = station.or(ctx.session.station_id)?;
            return Some(repackage(ctx, station, &[call.arg_id(0)]).await);
        }
        // UnasembleItems({stationID: [itemID]}), from the item menu.
        "UnasembleItems" => {
            let Some(Value::Dict(by_station)) = call.args.first() else { return Some(Outcome::Value(Value::None)) };
            for (station, items) in by_station {
                let outcome = repackage(ctx, station.as_i64().unwrap_or(0), &ids(Some(items))).await;
                if !matches!(outcome, Outcome::Value(_)) {
                    return Some(outcome);
                }
            }
            Value::None
        }
        _ => return None,
    };
    Some(Outcome::Value(value))
}

fn ids(v: Option<&Value>) -> Vec<i64> {
    v.and_then(|v| v.seq()).unwrap_or(&[]).iter().filter_map(|v| v.as_i64()).collect()
}

/// {itemID: KeyVal(quote=[KeyVal(itemID, typeID, damage, maxHealth,
/// costToRepairOneUnitOfDamage)])} for the caller's items
/// (base_repairshop.py:176-187). The shop shows one line per item, so a
/// ship's armor and structure damage are quoted together.
async fn damage_reports(ctx: &Ctx<'_>, item_ids: &[i64]) -> Value {
    let Some(me) = ctx.session.char_id else { return Value::Dict(Vec::new()) };
    let ids: Vec<i32> = item_ids.iter().map(|i| *i as i32).collect();
    let sql = r#"
        SELECT e."itemID", e."typeID",
               coalesce(d."valueFloat", d."valueInt"::float8, 0) + coalesce(ad."valueFloat", ad."valueInt"::float8, 0),
               coalesce(hp."value"::float8, 0) + coalesce(ahp."value"::float8, 0)
          FROM entity e
          LEFT JOIN entity_attributes d ON d."itemID" = e."itemID" AND d."attributeID" = $3
          LEFT JOIN entity_attributes ad ON ad."itemID" = e."itemID" AND ad."attributeID" = $5
          LEFT JOIN "dgmTypeAttribs" hp ON hp."typeID" = e."typeID" AND hp."attributeID" = $4
          LEFT JOIN "dgmTypeAttribs" ahp ON ahp."typeID" = e."typeID" AND ahp."attributeID" = $6
         WHERE e."itemID" = ANY($1) AND e."ownerID" = $2"#;
    let rows = ctx.query(sql, &[&ids, &(me as i32), &ATTR_DAMAGE, &ATTR_HP, &ATTR_ARMOR_DAMAGE, &ATTR_ARMOR_HP]).await;
    let reports = rows
        .iter()
        .map(|r| {
            let item_id = int(r, 0).unwrap_or(0);
            let quote = key_val(vec![
                ("itemID", Value::Int(item_id)),
                ("typeID", Value::int_or_none(int(r, 1))),
                ("damage", Value::Double(float(r, 2).unwrap_or(0.0).max(0.0))),
                ("maxHealth", Value::Double(float(r, 3).unwrap_or(0.0))),
                ("costToRepairOneUnitOfDamage", Value::Double(0.0)),
            ]);
            (Value::Int(item_id), key_val(vec![("quote", Value::List(vec![quote]))]))
        })
        .collect();
    Value::Dict(reports)
}

/// Repairs the caller's items in full: armor and structure. The client's
/// gauges are told, so the ship shows repaired without undocking.
async fn repair(ctx: &Ctx<'_>, item_ids: &[i64]) {
    let Some(me) = ctx.session.char_id else { return };
    let ids: Vec<i32> = item_ids.iter().map(|i| *i as i32).collect();
    let attrs = [ATTR_DAMAGE, ATTR_ARMOR_DAMAGE];
    let sql = r#"DELETE FROM entity_attributes a USING entity e
                  WHERE a."itemID" = e."itemID" AND a."attributeID" = ANY($3)
                    AND e."itemID" = ANY($1) AND e."ownerID" = $2
              RETURNING a."itemID", a."attributeID""#;
    let rows = ctx.query(sql, &[&ids, &(me as i32), &attrs.to_vec()]).await;
    if rows.is_empty() {
        return;
    }
    let now = crate::combat::filetime(crate::combat::now_ms());
    let changes: Vec<Value> = rows
        .iter()
        .map(|r| {
            Value::Tuple(vec![
                Value::bytes("OnModuleAttributeChange"),
                Value::Int(me),
                Value::Int(int(r, 0).unwrap_or(0)),
                Value::Int(int(r, 1).unwrap_or(0)),
                Value::Long(now),
                Value::Double(0.0),
                Value::Double(0.0),
            ])
        })
        .collect();
    info!("{me} repaired {} item(s): {} damage value(s) cleared", ids.len(), changes.len());
    ctx.world.lock().notify(&[ctx.client_id], "OnModuleAttributeChanges", Value::Tuple(vec![Value::List(changes)]));
}

/// Repackage assembled items the caller owns in `station`: never the active
/// ship, and only once they are empty.
async fn repackage(ctx: &Ctx<'_>, station: i64, item_ids: &[i64]) -> Outcome {
    let Some(me) = ctx.session.char_id else { return Outcome::Value(Value::None) };
    let ids: Vec<i32> = item_ids.iter().map(|i| *i as i32).collect();
    let active_ship = ctx.session.ship_id.map(|s| s as i32);

    let sql = r#"SELECT count(*) FROM entity WHERE "locationID" = ANY($1)"#;
    let holding = ctx.query_one(sql, &[&ids]).await.and_then(|r| int(&r, 0)).unwrap_or(0);
    if holding > 0 {
        return Outcome::custom("Remove everything from an item before repackaging it.");
    }
    let sql = r#"UPDATE entity SET "singleton" = 0
                  WHERE "itemID" = ANY($1) AND "ownerID" = $2 AND "locationID" = $3
                    AND "singleton" = 1 AND "itemID" IS DISTINCT FROM $4
                RETURNING "itemID""#;
    let done: Vec<i32> = ctx
        .query(sql, &[&ids, &(me as i32), &(station as i32), &active_ship])
        .await
        .iter()
        .filter_map(|r| int(r, 0))
        .map(|i| i as i32)
        .collect();
    if done.is_empty() {
        return Outcome::Value(Value::None);
    }

    let sql = format!(r#"{INVITEM_SELECT} e."itemID" = ANY($1)"#);
    let rows = ctx.query(&sql, &[&done]).await;
    let world = ctx.world.lock();
    for r in &rows {
        let event = item_change(invitem_cells(r), vec![(IX_SINGLETON, Value::Int(1))]);
        world.notify(&[ctx.client_id], "OnItemChange", event);
    }
    info!("repackaged {} item(s) at station {station}", done.len());
    Outcome::Value(Value::None)
}
