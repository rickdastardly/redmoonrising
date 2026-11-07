//! `standing2`: standings between characters, corporations and NPCs (standingsvc.py).

use eve_marshal::Value;
use tracing::info;

use crate::services::{Call, Ctx, Outcome};
use crate::session::CORP_ROLE_DIRECTOR;
use crate::svc::rows::{cell, float, rowset};

const OWNER_HEADER: &[&str] = &["ownerID", "ownerName", "typeID"];
// /maxstandings stores every agent relationship for mission access. The
// build 3913 standings window eagerly creates a row for each result; sending
// over ten thousand untouched agents freezes it. Keep the full server data,
// while listing only NPC organizations and agents with mission history.
const VISIBLE_NPC_STANDINGS: &str = r#"SELECT s."fromID",s."standing" FROM "chrNPCStandings" s
    WHERE s."characterID" = $1 AND (
        NOT EXISTS (SELECT 1 FROM "agtAgents" a WHERE a."agentID" = s."fromID")
        OR EXISTS (SELECT 1 FROM "agtCharMissions" m
                    WHERE m."characterID" = $1 AND m."agentID" = s."fromID")
    )"#;

pub async fn standing(call: &Call, ctx: &Ctx<'_>) -> Option<Outcome> {
    let value = match call.method.as_str() {
        "GetMyKillRights" => my_kill_rights(ctx).await,
        "GetNPCNPCStandings" => {
            let sql = r#"SELECT "fromID","toID","standing" FROM "npcStandings""#;
            ctx.rowset(&["fromID", "toID", "standing"], sql, &[]).await
        }
        "GetMyStandings" => my_standings(ctx).await,
        "GetStandingTransactions" => standing_transactions(ctx).await,
        "GetSecurityRating" => {
            let sql = r#"SELECT "securityRating" FROM "character_" WHERE "characterID" = $1"#;
            let rating = ctx.query_one(sql, &[&(call.arg_id(0) as i32)]).await;
            Value::Double(rating.and_then(|r| float(&r, 0)).unwrap_or(0.0))
        }
        "GetNPCStandingsTo" => {
            ctx.rowset(&["fromID", "standing"], VISIBLE_NPC_STANDINGS, &[&(call.arg_id(0) as i32)]).await
        }
        "GetAgentStandingsPage" => {
            agent_standings_page(ctx, call.arg_i64(0).unwrap_or(0), call.arg_i64(1).unwrap_or(1) != 0).await
        }
        "GetStandingCompositions" => {
            standing_compositions(ctx, call.arg_id(0), call.arg_id(1)).await
        }
        // (toID, standing, reason); a standing of 0 clears it, as the client's cache does.
        "SetPlayerStanding" => {
            let Some(me) = ctx.session.char_id else { return Some(Outcome::Value(Value::None)) };
            set_standing(ctx, Standing::Character(me), call).await
        }
        "SetCorpStanding" => {
            let Some(corp) = ctx.session.corp_id else { return Some(Outcome::Value(Value::None)) };
            if !ctx.session.has_corp_role(CORP_ROLE_DIRECTOR) {
                return Some(Outcome::UserError {
                    msg: "CrpAccessDenied".into(),
                    args: vec![("reason".into(), Value::bytes("you are not a director"))],
                });
            }
            set_standing(ctx, Standing::Corporation(corp), call).await
        }
        _ => return None,
    };
    Some(Outcome::Value(value))
}

/// The old Character Sheet constructs every visible standing entry eagerly.
/// Serve its agent section in bounded pages while keeping the ordinary
/// standings cache small enough for login and mission conversations.
async fn agent_standings_page(ctx: &Ctx<'_>, offset: i64, positive: bool) -> Value {
    const HEADER: &[&str] = &["agentID", "standing"];
    let Some(character_id) = ctx.session.char_id else {
        return rowset(HEADER, Vec::new());
    };
    let polarity = if positive { ">= 0.05" } else { "<= -0.05" };
    let sql = format!(
        r#"SELECT a."agentID", s."standing"
             FROM "agtAgents" a
             JOIN "chrNPCStandings" s ON s."fromID" = a."agentID" AND s."characterID" = $1
             JOIN "character_" c ON c."characterID" = a."agentID"
            WHERE s."standing" {polarity}
              AND NOT EXISTS (SELECT 1 FROM "agtCharMissions" m
                               WHERE m."characterID" = $1 AND m."agentID" = a."agentID")
            ORDER BY c."characterName", a."agentID"
            LIMIT 200 OFFSET $2"#
    );
    let offset = offset.clamp(0, 100_000);
    ctx.rowset(HEADER, &sql, &[&(character_id as i32), &offset]).await
}

enum Standing {
    Character(i64),
    Corporation(i64),
}

async fn set_standing(ctx: &Ctx<'_>, from: Standing, call: &Call) -> Value {
    let to_id = call.arg_id(0) as i32;
    let standing = call.args.get(1).and_then(|v| v.as_f64()).unwrap_or(0.0).clamp(-10.0, 10.0);
    let reason = call.arg_text(2).unwrap_or_default();
    let (from_id, upsert, delete) = match from {
        Standing::Character(id) => (
            id as i32,
            r#"INSERT INTO "chrStandings" ("characterID","toID","standing") VALUES ($1,$2,$3)
               ON CONFLICT ("characterID","toID") DO UPDATE SET "standing" = EXCLUDED."standing""#,
            r#"DELETE FROM "chrStandings" WHERE "characterID" = $1 AND "toID" = $2"#,
        ),
        Standing::Corporation(id) => (
            id as i32,
            r#"INSERT INTO "ownerStandings" ("fromID","toID","standing","reason","setAt")
               VALUES ($1,$2,$3,$4,$5)
               ON CONFLICT ("fromID","toID") DO UPDATE
                  SET "standing" = EXCLUDED."standing", "reason" = EXCLUDED."reason",
                      "setAt" = EXCLUDED."setAt""#,
            r#"DELETE FROM "ownerStandings" WHERE "fromID" = $1 AND "toID" = $2"#,
        ),
    };
    if standing == 0.0 {
        ctx.execute(delete, &[&from_id, &to_id]).await;
    } else if let Standing::Corporation(_) = from {
        let now = crate::bulkdata::filetime_now();
        ctx.execute(upsert, &[&from_id, &to_id, &standing, &reason, &now]).await;
    } else {
        ctx.execute(upsert, &[&from_id, &to_id, &standing]).await;
    }
    info!("standing {from_id} -> {to_id} set to {standing}");
    Value::None
}

/// What fromID's standing towards toID is made of. Standings here are set
/// directly, never derived, so the only component is the standing itself
/// (the window labels the fromID entry "Current Value").
async fn standing_compositions(ctx: &Ctx<'_>, from_id: i64, to_id: i64) -> Value {
    let sql = r#"SELECT $1::int AS "ownerID", "standing" FROM "chrNPCStandings"
                  WHERE "fromID" = $1 AND "characterID" = $2
                 UNION ALL
                 SELECT $1::int, "standing" FROM "chrStandings"
                  WHERE "characterID" = $1 AND "toID" = $2
                 UNION ALL
                 SELECT $1::int, "standing" FROM "ownerStandings"
                  WHERE "fromID" = $1 AND "toID" = $2"#;
    ctx.rowset(&["ownerID", "standing"], sql, &[&(from_id as i32), &(to_id as i32)]).await
}

async fn standing_transactions(ctx: &Ctx<'_>) -> Value {
    const HEADER: &[&str] = &[
        "eventID", "eventTypeID", "eventDateTime", "fromID", "toID", "originalFromID",
        "originalToID", "modification", "int_1", "int_2", "int_3", "msg",
    ];
    let Some(character) = ctx.session.char_id else { return rowset(HEADER, Vec::new()) };
    ctx.rowset(HEADER, r#"SELECT "eventID", "eventTypeID", "eventDateTime", "fromID", "toID",
                             "originalFromID", "originalToID", "modification", "int_1", "int_2", "int_3", "msg"
                        FROM "standingTransactions" WHERE "toID" = $1
                       ORDER BY "eventDateTime" DESC, "eventID" DESC LIMIT 500"#,
               &[&(character as i32)]).await
}

/// standingsvc.__Init unpacks (charstandings, charprime, npccharstandings,
/// corpstandings, corpprime, npccorpstandings). The primes go to
/// cfg.eveowners.Hint(None, prime), which needs owner rows, not bare IDs.
async fn my_standings(ctx: &Ctx<'_>) -> Value {
    const TO_HEADER: &[&str] = &["toID", "standing"];
    const FROM_HEADER: &[&str] = &["fromID", "standing"];
    let char_id = ctx.session.char_id.unwrap_or(0) as i32;
    let corp_id = ctx.session.corp_id.unwrap_or(0) as i32;

    let char_standings = ctx
        .rowset(TO_HEADER, r#"SELECT "toID","standing" FROM "chrStandings" WHERE "characterID" = $1"#, &[&char_id])
        .await;
    let char_prime = owners_of(ctx, r#"SELECT "toID" FROM "chrStandings" WHERE "characterID" = $1"#, char_id).await;
    let npc_standings = ctx
        .rowset(FROM_HEADER, VISIBLE_NPC_STANDINGS, &[&char_id])
        .await;
    let corp_standings = ctx
        .rowset(TO_HEADER, r#"SELECT "toID","standing" FROM "ownerStandings" WHERE "fromID" = $1"#, &[&corp_id])
        .await;
    let corp_prime = owners_of(ctx, r#"SELECT "toID" FROM "ownerStandings" WHERE "fromID" = $1"#, corp_id).await;

    Value::Tuple(vec![
        char_standings,
        char_prime,
        npc_standings,
        corp_standings,
        corp_prime,
        Value::Dict(Vec::new()),
    ])
}

/// Owner rows for the IDs `ids_sql` selects, for cfg.eveowners.Hint.
async fn owners_of(ctx: &Ctx<'_>, ids_sql: &str, id: i32) -> Value {
    let sql = format!(
        r#"SELECT "ownerID","ownerName","typeID" FROM "eveStaticOwners" WHERE "ownerID" IN ({ids_sql})"#
    );
    ctx.rowset(OWNER_HEADER, &sql, &[&id]).await
}

/// (rights I hold over others, rights others hold over me): {charID: expiry}.
async fn my_kill_rights(ctx: &Ctx<'_>) -> Value {
    let char_id = ctx.session.char_id.unwrap_or(0) as i32;
    let dict = |rows: Vec<tokio_postgres::Row>| {
        Value::Dict(rows.iter().map(|r| (cell(r, 0), cell(r, 1))).collect())
    };
    let held = ctx
        .query(r#"SELECT "toID","whenTimestamp" FROM "chrKillRights" WHERE "fromID" = $1"#, &[&char_id])
        .await;
    let against = ctx
        .query(r#"SELECT "fromID","whenTimestamp" FROM "chrKillRights" WHERE "toID" = $1"#, &[&char_id])
        .await;
    Value::Tuple(vec![dict(held), dict(against)])
}
