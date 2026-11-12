//! `factory`: blueprint and material information for the info window (infosvc.py)
//! and the facility window (facility.py).
//!
//! A blueprint's research and copy state is its invBlueprints row; one without
//! is an original at material and productivity level 0 with unlimited runs,
//! the same defaults the client itself assumes for a blueprint type with no item.

use eve_marshal::Value;

use crate::services::{Call, Ctx, Outcome};
use crate::svc::rows::{float, int, lines, rowset};
use tracing::{debug, info};

const CATEGORY_SKILL: i32 = 16;
const ACTIVITY_MANUFACTURING: i32 = 1;
/// dgmAttributeTypes stationTypeID: the outpost a construction platform becomes.
const ATTR_STATION_TYPE_ID: i32 = 472;

pub async fn factory(call: &Call, ctx: &Ctx<'_>) -> Option<Outcome> {
    let value = match call.method.as_str() {
        "GetBlueprintAttributes" => blueprint_attributes(ctx, call.arg_id(0)).await,
        // Activity -1 is Show Info's request for all activities it can label.
        "GetMaterialsForTypeWithActivity" => {
            let activity = call.arg_i64(1).unwrap_or(-1) as i32;
            let sql = r#"SELECT "activity","requiredTypeID","quantity","damagePerJob"
                           FROM "TL2MaterialsForTypeWithActivity"
                          WHERE "typeID" = $1 AND "quantity" > 0
                            AND ("activity" = $2 OR ($2 = -1 AND "activity" IN (1, 3, 4, 5, 6, 7)))
                          ORDER BY "activity","requiredTypeID""#;
            let header = ["activity", "requiredTypeID", "quantity", "damagePerJob"];
            ctx.rowset(&header, sql, &[&(call.arg_id(0) as i32), &activity]).await
        }
        // What an item is built from: the consumed manufacturing inputs of the
        // blueprint that produces it (no skills, no reusable tools). A
        // construction platform's Material Requirements tab asks for the
        // outpost it becomes (its stationTypeID), which no blueprint makes:
        // that outpost's materials are its platform's.
        "GetMaterialCompositionOfItemType" => {
            let sql = r#"SELECT m."requiredTypeID" AS "typeID", m."quantity"
                           FROM "invBlueprintTypes" b
                           JOIN "TL2MaterialsForTypeWithActivity" m
                             ON m."typeID" = b."blueprintTypeID" AND m."activity" = $2
                           JOIN "invTypes" t ON t."typeID" = m."requiredTypeID"
                           JOIN "invGroups" g ON g."groupID" = t."groupID"
                          WHERE b."productTypeID" = COALESCE(
                                    (SELECT a."typeID" FROM "dgmTypeAttributes" a
                                       JOIN "invBlueprintTypes" pb ON pb."productTypeID" = a."typeID"
                                      WHERE a."attributeID" = $4 AND a."valueInt" = $1
                                      ORDER BY a."typeID" LIMIT 1), $1)
                            AND m."damagePerJob" >= 1
                            AND m."quantity" > 0 AND g."categoryID" <> $3
                          ORDER BY m."requiredTypeID""#;
            let params: [&(dyn tokio_postgres::types::ToSql + Sync); 4] =
                [&(call.arg_id(0) as i32), &ACTIVITY_MANUFACTURING, &CATEGORY_SKILL, &ATTR_STATION_TYPE_ID];
            let type_id = call.arg_id(0);
            let found = ctx.query(sql, &params).await;
            debug!("GetMaterialCompositionOfItemType({type_id}): {} material(s)", found.len());
            rowset(&["typeID", "quantity"], lines(&found))
        }
        // A client hook's one-line diagnostic, for the server log.
        "RmrReport" => {
            info!("client hook: {}", call.arg_text(0).unwrap_or_default());
            Value::None
        }
        // (locationID, flag, isCorp): the blueprints in one hangar.
        "GetBlueprintInformationAtLocationWithFlag" => {
            let sql = r#"SELECT e."itemID", e."typeID", e."quantity", e."singleton",
                                COALESCE(i."copy", 0), COALESCE(i."materialLevel", 0),
                                COALESCE(i."productivityLevel", 0), COALESCE(i."licensedProductionRunsRemaining", -1)
                           FROM entity e
                           JOIN "invBlueprintTypes" b ON b."blueprintTypeID" = e."typeID"
                           LEFT JOIN "invBlueprints" i ON i."itemID" = e."itemID"
                          WHERE e."locationID" = $1 AND e."flag" = $2
                          ORDER BY e."itemID""#;
            let header = [
                "itemID", "typeID", "quantity", "singleton", "copy", "materialLevel", "productivityLevel",
                "licensedProductionRunsRemaining",
            ];
            let location = call.arg_id(0) as i32;
            let flag = call.arg_id(1) as i32;
            ctx.rowset(&header, sql, &[&location, &flag]).await
        }
        _ => return None,
    };
    Some(Outcome::Value(value))
}

/// infosvc reads these keys from the dict (has_key) for a blueprint item.
async fn blueprint_attributes(ctx: &Ctx<'_>, item_id: i64) -> Value {
    let sql = r#"SELECT b."productionTime", b."researchMaterialTime", b."researchCopyTime",
                        b."researchProductivityTime", b."wasteFactor", b."productTypeID",
                        COALESCE(i."materialLevel", 0), COALESCE(i."productivityLevel", 0),
                        COALESCE(i."copy", 0), COALESCE(i."licensedProductionRunsRemaining", -1)
                   FROM entity e
                   JOIN "invBlueprintTypes" b ON b."blueprintTypeID" = e."typeID"
                   LEFT JOIN "invBlueprints" i ON i."itemID" = e."itemID"
                  WHERE e."itemID" = $1"#;
    let Some(r) = ctx.query_one(sql, &[&(item_id as i32)]).await else { return Value::None };
    let num = |i: usize| Value::Int(int(&r, i).unwrap_or(0));
    let pairs = [
        ("manufacturingTime", num(0)),
        ("researchMaterialTime", num(1)),
        ("researchCopyTime", num(2)),
        ("researchProductivityTime", num(3)),
        ("wastageFactor", Value::Double(float(&r, 4).unwrap_or(0.0) / 100.0)),
        ("productTypeID", num(5)),
        ("materialLevel", num(6)),
        ("productivityLevel", num(7)),
        ("copy", num(8)),
        ("licensedProductionRunsRemaining", num(9)),
    ];
    Value::Dict(pairs.into_iter().map(|(k, v)| (Value::bytes(k), v)).collect())
}
