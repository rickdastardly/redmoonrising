//! Skills: training (skillMgr), what a skill or module needs, and the
//! prerequisites every character must hold.
//!
//! One skill trains at a time. Starting it moves the skill to the training
//! flag and records when it started, from how many points and when it ends
//! ("rmrSkillTraining"); `run_training` finishes it at that time, online or
//! not. Stopping early keeps the points earned so far. The client hears of
//! each step as EVE told it (OnSkillStartTraining, OnSkillTrainingStopped,
//! OnSkillTrained), with the item and attribute changes.

use std::collections::HashMap;
use std::sync::Arc;

use eve_marshal::Value;
use tokio_postgres::GenericClient;
use tracing::{error, info, warn};

use crate::items::ItemTxn;
use crate::services::{Call, Ctx, Outcome};

const SERVICE: &str = "skillMgr";

const FLAG_SKILL: i32 = 7;
const FLAG_SKILL_IN_TRAINING: i32 = 61;
const FLAG_BOOSTER: i32 = 88;
const FLAG_IMPLANT: i32 = 89;

const CATEGORY_SKILL: i32 = 16;
const CATEGORY_IMPLANT: i32 = 20;
const GROUP_BOOSTER: i32 = 303;

const ATTR_PRIMARY: i32 = 180;
const ATTR_SECONDARY: i32 = 181;
const ATTR_SKILL_TIME_CONSTANT: i32 = 275;
const ATTR_SKILL_POINTS: i32 = 276;
const ATTR_SKILL_LEVEL: i32 = 280;
/// (requiredSkillN, requiredSkillNLevel).
const REQUIREMENTS: [(i32, i32); 6] = [(182, 277), (183, 278), (184, 279), (1285, 1286), (1289, 1287), (1290, 1288)];
/// A FILETIME minute.
const MINUTE: f64 = 600_000_000.0;

/// The character's trained level in each of `types` (missing ones are 0).
/// Works on a client or inside a transaction.
pub async fn levels(
    db: &impl tokio_postgres::GenericClient,
    char_id: i64,
    types: &[i32],
) -> Result<std::collections::HashMap<i32, i64>, tokio_postgres::Error> {
    let rows = db
        .query(
            r#"SELECT e."typeID", max(coalesce(a."valueInt", a."valueFloat"::bigint))
                 FROM entity e
                 JOIN entity_attributes a ON a."itemID" = e."itemID" AND a."attributeID" = $3
                WHERE e."ownerID" = $1 AND e."flag" IN ($4, $5) AND e."typeID" = ANY($2)
                GROUP BY e."typeID""#,
            &[&(char_id as i32), &types, &ATTR_SKILL_LEVEL, &FLAG_SKILL, &FLAG_SKILL_IN_TRAINING],
        )
        .await?;
    let mut out: std::collections::HashMap<i32, i64> = types.iter().map(|t| (*t, 0)).collect();
    for r in rows {
        out.insert(r.get(0), r.get::<_, Option<i64>>(1).unwrap_or(0));
    }
    Ok(out)
}

pub async fn skill_mgr(call: &Call, ctx: &Ctx<'_>) -> Option<Outcome> {
    Some(method_call(&call.method, &call.args, ctx).await)
}

async fn method_call(method: &str, args: &[Value], ctx: &Ctx<'_>) -> Outcome {
    let item_id = args.first().and_then(|v| v.as_i64()).unwrap_or(0);
    match method {
        "GetEndOfTraining" => Outcome::Value(end_of_training(ctx, item_id).await),
        "CharStartTrainingSkill" => match start_training(ctx, item_id).await {
            Ok(()) => Outcome::Value(Value::None),
            Err(why) => Outcome::custom(why),
        },
        "CharStopTrainingSkill" => match stop_training(ctx, item_id).await {
            Ok(()) => Outcome::Value(Value::None),
            Err(why) => Outcome::custom(why),
        },
        "CharAddImplant" => Outcome::Value(add_implant(ctx, item_id).await),
        "RemoveImplantFromCharacter" => Outcome::Value(remove_implant(ctx, item_id).await),
        _ => Outcome::Unknown(format!("{}::{}", SERVICE, method)),
    }
}

async fn end_of_training(ctx: &Ctx<'_>, skill_id: i64) -> Value {
    let Some(char_id) = ctx.session.char_id else { return Value::None };
    let sql = r#"SELECT "endTime" FROM "rmrSkillTraining" WHERE "characterID" = $1 AND "itemID" = $2"#;
    match ctx.db.query_opt(sql, &[&(char_id as i32), &(skill_id as i32)]).await {
        Ok(Some(r)) => Value::Long(r.get(0)),
        Ok(None) => Value::None,
        Err(e) => {
            error!("GetEndOfTraining: {}", crate::logging::chain(&e));
            Value::None
        }
    }
}

/// What training a skill needs to know of it.
struct Progress {
    type_id: i64,
    level: i64,
    points: f64,
    /// Points at the next level.
    next: f64,
}

async fn progress(ctx: &Ctx<'_>, skill_id: i64, type_id: i64) -> Progress {
    let rank = attr(ctx, skill_id, type_id, ATTR_SKILL_TIME_CONSTANT).await.unwrap_or(1.0).max(0.01);
    let level = attr(ctx, skill_id, type_id, ATTR_SKILL_LEVEL).await.unwrap_or(0.0).clamp(0.0, 5.0) as i64;
    let points = attr(ctx, skill_id, type_id, ATTR_SKILL_POINTS).await.unwrap_or(0.0).max(0.0);
    let next = crate::losses::points_at(rank * 250.0, (level + 1).min(5));
    Progress { type_id, level, points, next }
}

/// Skill points a minute on `type_id`: the primary attribute and half the
/// secondary, as the character has them now (implants and learning skills
/// included).
async fn points_per_minute(ctx: &Ctx<'_>, char_id: i64, skill_id: i64, type_id: i64) -> f64 {
    let computed = crate::svc::fitting::snapshot(ctx.db, ctx.dogma, char_id, None).await.ok().and_then(|s| s.get(&char_id).cloned());
    let mut rate = 0.0;
    for (attribute, share) in [(ATTR_PRIMARY, 1.0), (ATTR_SECONDARY, 0.5)] {
        let which = attr(ctx, skill_id, type_id, attribute).await.unwrap_or(0.0) as i32;
        if which == 0 {
            continue;
        }
        let value = match computed.as_ref().and_then(|c| c.get(&which)).copied().filter(|v| *v > 0.0) {
            Some(v) => v,
            None => char_attr(ctx, char_id, which).await,
        };
        rate += value * share;
    }
    if rate.is_finite() && rate > 0.0 { rate } else { 1.0 }
}

async fn start_training(ctx: &Ctx<'_>, skill_id: i64) -> Result<(), String> {
    let Some(char_id) = ctx.session.char_id else { return Err("Select a character first.".into()) };
    let Some(it) = item(ctx, skill_id).await else {
        warn!("CharStartTrainingSkill: no item {skill_id}");
        return Err("That skill could not be found.".into());
    };
    if it.owner_id != char_id || it.category_id != CATEGORY_SKILL || (it.flag != FLAG_SKILL && it.flag != FLAG_SKILL_IN_TRAINING) {
        warn!("CharStartTrainingSkill: {skill_id} is not {char_id}'s skill");
        return Err("That is not one of your skills.".into());
    }
    let p = progress(ctx, skill_id, it.type_id).await;
    if p.level >= 5 {
        return Err("That skill is already at level V.".into());
    }
    let missing = missing_for(ctx.db, char_id, it.type_id).await.map_err(|e| {
        error!("CharStartTrainingSkill (requirements): {}", crate::logging::chain(&e));
        "The skill's requirements could not be read.".to_string()
    })?;
    if !missing.is_empty() {
        return Err(format!("You need {} before you can train that skill.", describe(ctx.db, &missing).await));
    }
    let current = ctx
        .db
        .query_opt(r#"SELECT "itemID" FROM "rmrSkillTraining" WHERE "characterID" = $1"#, &[&(char_id as i32)])
        .await
        .map_err(|e| e.to_string())?
        .map(|r| r.get::<_, i32>(0) as i64);
    if current == Some(skill_id) {
        return Ok(());
    }
    if let Some(other) = current {
        stop_training(ctx, other).await?;
    }

    let rate = points_per_minute(ctx, char_id, skill_id, it.type_id).await;
    let now = crate::bulkdata::filetime_now();
    let end = now + (((p.next - p.points).max(0.0) / rate) * MINUTE) as i64;
    let result: Result<Vec<Value>, tokio_postgres::Error> = async {
        let mut client = crate::db::connect(ctx.db_url).await?;
        let mut txn = ItemTxn::begin(&mut client).await?;
        // Anything else left in training (by an older version) stops.
        txn.tx()
            .execute(
                r#"UPDATE entity SET "flag" = $1 WHERE "ownerID" = $2 AND "flag" = $3 AND "itemID" <> $4"#,
                &[&FLAG_SKILL, &(char_id as i32), &FLAG_SKILL_IN_TRAINING, &(skill_id as i32)],
            )
            .await?;
        txn.move_to(skill_id, char_id, FLAG_SKILL_IN_TRAINING as i64).await?;
        txn.tx()
            .execute(
                r#"INSERT INTO "rmrSkillTraining" ("characterID", "itemID", "startTime", "startPoints", "endTime")
                   VALUES ($1, $2, $3, $4, $5)
                   ON CONFLICT ("characterID") DO UPDATE SET "itemID" = $2, "startTime" = $3, "startPoints" = $4, "endTime" = $5"#,
                &[&(char_id as i32), &(skill_id as i32), &now, &p.points, &end],
            )
            .await?;
        txn.commit().await
    }
    .await;
    let events = result.map_err(|e| {
        error!("CharStartTrainingSkill: {}", crate::logging::chain(&e));
        "Training could not be started.".to_string()
    })?;
    crate::items::notify(ctx.world, ctx.client_id, events);
    let world = ctx.world.lock();
    world.notify(&[ctx.client_id], "OnSkillStartTraining", Value::Tuple(vec![Value::Int(skill_id), Value::Long(end)]));
    info!("{char_id} started training skill {} (item {skill_id}) to level {}, {:.0} SP a minute", p.type_id, p.level + 1, rate);
    Ok(())
}

async fn stop_training(ctx: &Ctx<'_>, skill_id: i64) -> Result<(), String> {
    let Some(char_id) = ctx.session.char_id else { return Err("Select a character first.".into()) };
    let row = ctx
        .db
        .query_opt(
            r#"SELECT "startTime", "startPoints", "endTime" FROM "rmrSkillTraining" WHERE "characterID" = $1 AND "itemID" = $2"#,
            &[&(char_id as i32), &(skill_id as i32)],
        )
        .await
        .map_err(|e| e.to_string())?;
    let Some(it) = item(ctx, skill_id).await else { return Ok(()) };
    if it.owner_id != char_id {
        return Err("That is not one of your skills.".into());
    }
    let p = progress(ctx, skill_id, it.type_id).await;
    // The points earned so far: the share of the training time that has passed.
    let points = match &row {
        Some(r) => {
            let (start, from, end): (i64, f64, i64) = (r.get(0), r.get(1), r.get(2));
            let now = crate::bulkdata::filetime_now();
            let done = if end > start { ((now - start) as f64 / (end - start) as f64).clamp(0.0, 1.0) } else { 1.0 };
            (from + (p.next - from) * done).min(p.next - 1.0).max(p.points).floor()
        }
        None => p.points,
    };
    let result: Result<Vec<Value>, tokio_postgres::Error> = async {
        let mut client = crate::db::connect(ctx.db_url).await?;
        let mut txn = ItemTxn::begin(&mut client).await?;
        set_attribute(txn.tx(), skill_id, ATTR_SKILL_POINTS, points as i64).await?;
        if it.flag == FLAG_SKILL_IN_TRAINING {
            txn.move_to(skill_id, char_id, FLAG_SKILL as i64).await?;
        }
        txn.tx().execute(r#"DELETE FROM "rmrSkillTraining" WHERE "characterID" = $1"#, &[&(char_id as i32)]).await?;
        txn.commit().await
    }
    .await;
    let events = result.map_err(|e| {
        error!("CharStopTrainingSkill: {}", crate::logging::chain(&e));
        "Training could not be stopped.".to_string()
    })?;
    crate::items::notify(ctx.world, ctx.client_id, events);
    let world = ctx.world.lock();
    let change = attribute_change(char_id, skill_id, ATTR_SKILL_POINTS, points, p.points);
    world.notify(&[ctx.client_id], "OnModuleAttributeChanges", Value::Tuple(vec![Value::List(vec![change])]));
    world.notify(&[ctx.client_id], "OnSkillTrainingStopped", Value::Tuple(vec![Value::Int(skill_id), Value::Int(0)]));
    info!("{char_id} stopped training skill {} (item {skill_id}) at {points:.0} SP", p.type_id);
    Ok(())
}

fn attribute_change(owner: i64, item_id: i64, attribute: i32, new: f64, old: f64) -> Value {
    Value::Tuple(vec![
        Value::bytes("OnModuleAttributeChange"),
        Value::Int(owner),
        Value::Int(item_id),
        Value::Int(attribute as i64),
        Value::Long(crate::bulkdata::filetime_now()),
        Value::Int(new.round() as i64),
        Value::Int(old.round() as i64),
    ])
}

async fn set_attribute(db: &impl GenericClient, item_id: i64, attribute: i32, value: i64) -> Result<(), tokio_postgres::Error> {
    db.execute(
        r#"INSERT INTO entity_attributes ("itemID", "attributeID", "valueInt", "valueFloat") VALUES ($1, $2, $3, NULL)
           ON CONFLICT ("itemID", "attributeID") DO UPDATE SET "valueInt" = $3, "valueFloat" = NULL"#,
        &[&(item_id as i32), &attribute, &value],
    )
    .await
    .map(|_| ())
}

/// The training table; skills an older version left on the training flag,
/// with no record of when they started, go back to the skill list.
pub async fn ensure_training(db: &tokio_postgres::Client) -> Result<(), tokio_postgres::Error> {
    db.batch_execute(
        r#"CREATE TABLE IF NOT EXISTS "rmrSkillTraining" (
               "characterID" integer PRIMARY KEY,
               "itemID" integer NOT NULL,
               "startTime" bigint NOT NULL,
               "startPoints" double precision NOT NULL,
               "endTime" bigint NOT NULL
           );
           CREATE INDEX IF NOT EXISTS "rmrSkillTraining_end" ON "rmrSkillTraining" ("endTime");"#,
    )
    .await?;
    let reset = db
        .execute(
            r#"UPDATE entity e SET "flag" = $1
                WHERE e."flag" = $2 AND NOT EXISTS (SELECT 1 FROM "rmrSkillTraining" t WHERE t."itemID" = e."itemID")"#,
            &[&FLAG_SKILL, &FLAG_SKILL_IN_TRAINING],
        )
        .await?;
    if reset > 0 {
        info!("skills: {reset} skill(s) left marked as training with no start time went back to the skill list");
    }
    Ok(())
}

/// Finishes training as it falls due, whether the pilot is online or not.
pub async fn run_training(server: Arc<crate::conn::Server>) {
    loop {
        if let Err(e) = finish_due(&server).await {
            warn!("skills: finishing training: {}", crate::logging::chain(&e));
        }
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    }
}

async fn finish_due(server: &crate::conn::Server) -> Result<(), tokio_postgres::Error> {
    let now = crate::bulkdata::filetime_now();
    let due = server
        .db
        .query(r#"SELECT "characterID", "itemID" FROM "rmrSkillTraining" WHERE "endTime" <= $1 LIMIT 200"#, &[&now])
        .await?;
    for r in due {
        let (char_id, skill_id) = (r.get::<_, i32>(0) as i64, r.get::<_, i32>(1) as i64);
        let mut client = crate::db::connect(&server.db_url).await?;
        let mut txn = ItemTxn::begin(&mut client).await?;
        let found = txn
            .tx()
            .query_opt(
                r#"SELECT COALESCE((SELECT coalesce(a."valueInt", a."valueFloat"::bigint) FROM entity_attributes a WHERE a."itemID" = e."itemID" AND a."attributeID" = $2), 0),
                          COALESCE((SELECT coalesce(a."valueFloat", a."valueInt"::float8) FROM entity_attributes a WHERE a."itemID" = e."itemID" AND a."attributeID" = $3), 0),
                          COALESCE((SELECT coalesce(d."valueFloat", d."valueInt"::float8) FROM "dgmTypeAttributes" d WHERE d."typeID" = e."typeID" AND d."attributeID" = $4), 1),
                          e."ownerID", e."typeID"
                     FROM entity e WHERE e."itemID" = $1"#,
                &[&(skill_id as i32), &ATTR_SKILL_LEVEL, &ATTR_SKILL_POINTS, &ATTR_SKILL_TIME_CONSTANT],
            )
            .await?;
        txn.tx().execute(r#"DELETE FROM "rmrSkillTraining" WHERE "characterID" = $1"#, &[&(char_id as i32)]).await?;
        let Some(found) = found.filter(|f| f.get::<_, i32>(3) as i64 == char_id) else {
            txn.commit().await?;
            continue;
        };
        let (level, points, rank, type_id): (i64, f64, f64, i32) = (found.get(0), found.get(1), found.get(2), found.get(4));
        let level_now = (level + 1).min(5);
        let points_now = crate::losses::points_at(rank.max(0.01) * 250.0, level_now).round();
        set_attribute(txn.tx(), skill_id, ATTR_SKILL_LEVEL, level_now).await?;
        set_attribute(txn.tx(), skill_id, ATTR_SKILL_POINTS, points_now as i64).await?;
        txn.move_to(skill_id, char_id, FLAG_SKILL as i64).await?;
        let events = txn.commit().await?;
        let clients = server.world.lock().clients_of(char_id);
        for client_id in clients {
            crate::items::notify(&server.world, client_id, events.clone());
            let world = server.world.lock();
            let changes = vec![
                attribute_change(char_id, skill_id, ATTR_SKILL_LEVEL, level_now as f64, level as f64),
                attribute_change(char_id, skill_id, ATTR_SKILL_POINTS, points_now, points),
            ];
            world.notify(&[client_id], "OnModuleAttributeChanges", Value::Tuple(vec![Value::List(changes)]));
            world.notify(&[client_id], "OnSkillTrained", Value::Tuple(vec![Value::Int(skill_id)]));
        }
        info!("{char_id} finished training skill {type_id} (item {skill_id}) to level {level_now}");
    }
    Ok(())
}

// ---- what skills and modules need ------------------------------------------------------

/// Each type's required skills, with the level each is needed at.
pub async fn requirements(db: &impl GenericClient, types: &[i32]) -> Result<HashMap<i32, Vec<(i32, i64)>>, tokio_postgres::Error> {
    let rows = db
        .query(
            r#"SELECT s."typeID", COALESCE(s."valueInt", s."valueFloat"::int)::int,
                      COALESCE(l."valueInt", l."valueFloat"::int, 1)::bigint
                 FROM "dgmTypeAttributes" s
                 LEFT JOIN "dgmTypeAttributes" l ON l."typeID" = s."typeID" AND l."attributeID" = CASE s."attributeID"
                      WHEN 182 THEN 277 WHEN 183 THEN 278 WHEN 184 THEN 279
                      WHEN 1285 THEN 1286 WHEN 1289 THEN 1287 WHEN 1290 THEN 1288 END
                WHERE s."typeID" = ANY($1) AND s."attributeID" = ANY($2)"#,
            &[&types, &REQUIREMENTS.iter().map(|(s, _)| *s).collect::<Vec<i32>>()],
        )
        .await?;
    let mut out: HashMap<i32, Vec<(i32, i64)>> = HashMap::new();
    for r in rows {
        let (type_id, skill, level): (i32, i32, i64) = (r.get(0), r.get(1), r.get(2));
        if skill > 0 && skill != type_id {
            out.entry(type_id).or_default().push((skill, level.clamp(1, 5)));
        }
    }
    Ok(out)
}

/// `skills` with every skill they need, at the highest level any needs it.
pub async fn with_prerequisites(db: &impl GenericClient, skills: &[(i32, i32)]) -> Result<Vec<(i32, i32)>, tokio_postgres::Error> {
    let mut out: Vec<(i32, i32)> = skills.to_vec();
    let mut frontier: Vec<i32> = skills.iter().map(|(t, _)| *t).collect();
    for _ in 0..10 {
        if frontier.is_empty() {
            break;
        }
        let needs = requirements(db, &frontier).await?;
        let mut next = Vec::new();
        for list in needs.values() {
            for &(skill, level) in list {
                let level = level as i32;
                match out.iter_mut().find(|(t, _)| *t == skill) {
                    Some((_, held)) => *held = (*held).max(level),
                    None => {
                        out.push((skill, level));
                        next.push(skill);
                    }
                }
            }
        }
        frontier = next;
    }
    Ok(out)
}

/// The skills (and levels) `char_id` lacks to use `type_id`.
pub async fn missing_for(db: &impl GenericClient, char_id: i64, type_id: i64) -> Result<Vec<(i32, i64)>, tokio_postgres::Error> {
    let needs = requirements(db, &[type_id as i32]).await?.remove(&(type_id as i32)).unwrap_or_default();
    if needs.is_empty() {
        return Ok(needs);
    }
    let skills: Vec<i32> = needs.iter().map(|(s, _)| *s).collect();
    let held = levels(db, char_id, &skills).await?;
    Ok(needs.into_iter().filter(|(s, l)| held.get(s).copied().unwrap_or(0) < *l).collect())
}

/// "Gunnery level I and Small Hybrid Turret level II".
pub async fn describe(db: &impl GenericClient, missing: &[(i32, i64)]) -> String {
    const ROMAN: [&str; 6] = ["0", "I", "II", "III", "IV", "V"];
    let ids: Vec<i32> = missing.iter().map(|(s, _)| *s).collect();
    let names: HashMap<i32, String> = db
        .query(r#"SELECT "typeID", "typeName" FROM "invTypes" WHERE "typeID" = ANY($1)"#, &[&ids])
        .await
        .map(|rows| rows.iter().map(|r| (r.get(0), r.get(1))).collect())
        .unwrap_or_default();
    let parts: Vec<String> = missing
        .iter()
        .map(|(s, l)| format!("{} level {}", names.get(s).cloned().unwrap_or_else(|| format!("skill {s}")), ROMAN[(*l).clamp(0, 5) as usize]))
        .collect();
    match parts.len() {
        0 => String::new(),
        1 => parts[0].clone(),
        n => format!("{} and {}", parts[..n - 1].join(", "), parts[n - 1]),
    }
}

/// Gives every player character the skills its skills need, at the levels
/// they need: characters made before new pilots got their prerequisites.
pub async fn grant_missing_prerequisites(db: &tokio_postgres::Client) -> Result<usize, tokio_postgres::Error> {
    let rows = db
        .query(
            r#"SELECT e."ownerID", e."itemID", e."typeID", COALESCE((SELECT coalesce(a."valueInt", a."valueFloat"::bigint)
                        FROM entity_attributes a WHERE a."itemID" = e."itemID" AND a."attributeID" = $3), 0)
                 FROM entity e
                 JOIN "character_" c ON c."characterID" = e."ownerID"
                 JOIN "account" acc ON acc."accountID" = c."accountID"
                WHERE e."flag" IN ($1, $2) AND e."locationID" = e."ownerID" AND acc."accountName" <> 'rmr-bots'"#,
            &[&FLAG_SKILL, &FLAG_SKILL_IN_TRAINING, &ATTR_SKILL_LEVEL],
        )
        .await?;
    let mut by_char: HashMap<i32, Vec<(i32, i32, i64)>> = HashMap::new();
    for r in rows {
        by_char.entry(r.get(0)).or_default().push((r.get(1), r.get(2), r.get(3)));
    }
    let mut granted = 0;
    for (char_id, held) in by_char {
        let trained: Vec<(i32, i32)> = held.iter().filter(|(_, _, l)| *l >= 1).map(|(_, t, l)| (*t, *l as i32)).collect();
        let wanted = with_prerequisites(db, &trained).await?;
        for (type_id, level) in wanted {
            let have = held.iter().filter(|(_, t, _)| *t == type_id).map(|(i, _, l)| (*i, *l)).max_by_key(|(_, l)| *l);
            if have.is_some_and(|(_, l)| l >= level as i64) {
                continue;
            }
            let rank: f64 = db
                .query_opt(
                    r#"SELECT coalesce("valueFloat", "valueInt"::float8) FROM "dgmTypeAttributes" WHERE "typeID" = $1 AND "attributeID" = $2"#,
                    &[&type_id, &ATTR_SKILL_TIME_CONSTANT],
                )
                .await?
                .and_then(|r| r.get::<_, Option<f64>>(0))
                .unwrap_or(1.0);
            let points = crate::losses::points_at(rank * 250.0, level as i64).round() as i64;
            let item_id = match have {
                Some((id, _)) => id,
                None => db
                    .query_one(
                        r#"INSERT INTO entity ("itemName","typeID","ownerID","locationID","flag","singleton","quantity")
                           VALUES ('', $1, $2, $2, $3, 1, 1) RETURNING "itemID""#,
                        &[&type_id, &char_id, &FLAG_SKILL],
                    )
                    .await?
                    .get(0),
            };
            set_attribute(db, item_id as i64, ATTR_SKILL_LEVEL, level as i64).await?;
            // Points never go down: a partly trained skill keeps what it has.
            db.execute(
                r#"INSERT INTO entity_attributes ("itemID", "attributeID", "valueInt", "valueFloat") VALUES ($1, $2, $3, NULL)
                   ON CONFLICT ("itemID", "attributeID") DO UPDATE
                   SET "valueInt" = GREATEST(COALESCE(entity_attributes."valueInt", entity_attributes."valueFloat"::bigint, 0), $3), "valueFloat" = NULL"#,
                &[&item_id, &ATTR_SKILL_POINTS, &points],
            )
            .await?;
            granted += 1;
        }
    }
    Ok(granted)
}

async fn add_implant(ctx: &Ctx<'_>, item_id: i64) -> Value {
    let Some(char_id) = ctx.session.char_id else { return Value::None };
    let Some(it) = item(ctx, item_id).await else {
        warn!("CharAddImplant: no item {item_id}");
        return Value::None;
    };
    if it.owner_id != char_id || it.category_id != CATEGORY_IMPLANT {
        warn!("CharAddImplant: {item_id} is not {char_id}'s implant");
        return Value::None;
    }
    let flag = if it.group_id == GROUP_BOOSTER { FLAG_BOOSTER } else { FLAG_IMPLANT };
    let sql = r#"UPDATE "entity" SET "locationID" = $1, "flag" = $2
                   WHERE "itemID" = $3 AND "ownerID" = $4"#;
    if let Err(e) =
        ctx.db.execute(sql, &[&(char_id as i32), &flag, &(item_id as i32), &(char_id as i32)]).await
    {
        error!("CharAddImplant: {err}", err = crate::logging::chain(&e));
    }
    Value::None
}

async fn remove_implant(ctx: &Ctx<'_>, item_id: i64) -> Value {
    let Some(char_id) = ctx.session.char_id else { return Value::None };
    let Some(it) = item(ctx, item_id).await else { return Value::None };
    if it.owner_id != char_id || it.flag != FLAG_IMPLANT {
        warn!("RemoveImplantFromCharacter: {item_id} is not a plugged implant");
        return Value::None;
    }
    let attrs = r#"DELETE FROM "entity_attributes" WHERE "itemID" = $1"#;
    if let Err(e) = ctx.db.execute(attrs, &[&(item_id as i32)]).await {
        error!("RemoveImplantFromCharacter (attributes): {err}", err = crate::logging::chain(&e));
        return Value::None;
    }
    let row = r#"DELETE FROM "entity" WHERE "itemID" = $1 AND "ownerID" = $2 AND "flag" = $3"#;
    if let Err(e) = ctx.db.execute(row, &[&(item_id as i32), &(char_id as i32), &FLAG_IMPLANT]).await
    {
        error!("RemoveImplantFromCharacter: {err}", err = crate::logging::chain(&e));
    }
    Value::None
}

struct Item {
    type_id: i64,
    owner_id: i64,
    flag: i32,
    group_id: i32,
    category_id: i32,
}

async fn item(ctx: &Ctx<'_>, item_id: i64) -> Option<Item> {
    let sql = r#"SELECT e."typeID", e."ownerID", e."flag", g."groupID", g."categoryID"
                   FROM "entity" e
                   JOIN "invTypes" t ON t."typeID" = e."typeID"
                   JOIN "invGroups" g ON g."groupID" = t."groupID"
                  WHERE e."itemID" = $1"#;
    match ctx.db.query_opt(sql, &[&(item_id as i32)]).await {
        Ok(Some(r)) => Some(Item {
            type_id: r.get::<_, i32>(0) as i64,
            owner_id: r.get::<_, i32>(1) as i64,
            flag: r.get::<_, i32>(2),
            group_id: r.get::<_, i32>(3),
            category_id: r.get::<_, i32>(4),
        }),
        Ok(None) => None,
        Err(e) => {
            error!("item {item_id}: {err}", err = crate::logging::chain(&e));
            None
        }
    }
}

async fn attr(ctx: &Ctx<'_>, item_id: i64, type_id: i64, attribute_id: i32) -> Option<f64> {
    let sql = r#"SELECT COALESCE(
                     (SELECT COALESCE(ea."valueFloat", ea."valueInt"::double precision)
                        FROM "entity_attributes" ea
                       WHERE ea."itemID" = $1 AND ea."attributeID" = $3),
                     (SELECT COALESCE(ta."valueFloat", ta."valueInt"::double precision)
                        FROM "dgmTypeAttributes" ta
                       WHERE ta."typeID" = $2 AND ta."attributeID" = $3),
                     (SELECT at."defaultValue"::double precision
                        FROM "dgmAttributeTypes" at
                       WHERE at."attributeID" = $3))"#;
    match ctx.db.query_opt(sql, &[&(item_id as i32), &(type_id as i32), &attribute_id]).await {
        Ok(Some(r)) => r.get::<_, Option<f64>>(0),
        Ok(None) => None,
        Err(e) => {
            error!("attribute {attribute_id} of {item_id}: {err}", err = crate::logging::chain(&e));
            None
        }
    }
}

async fn char_attr(ctx: &Ctx<'_>, char_id: i64, attribute_id: i32) -> f64 {
    let own = r#"SELECT COALESCE(ea."valueFloat", ea."valueInt"::double precision)
                   FROM "entity_attributes" ea
                  WHERE ea."itemID" = $1 AND ea."attributeID" = $2"#;
    if let Ok(Some(r)) = ctx.db.query_opt(own, &[&(char_id as i32), &attribute_id]).await {
        if let Some(v) = r.get::<_, Option<f64>>(0) {
            return v;
        }
    }

    let column = match attribute_id {
        164 => "charisma",
        165 => "intelligence",
        166 => "memory",
        167 => "perception",
        168 => "willpower",
        _ => return 0.0,
    };
    let sql = r#"SELECT b."charisma", b."intelligence", b."memory", b."perception", b."willpower"
                   FROM "character_" c
                   JOIN "chrBloodlines" b ON b."bloodlineID" = c."bloodlineID"
                  WHERE c."characterID" = $1"#;
    match ctx.db.query_opt(sql, &[&(char_id as i32)]).await {
        Ok(Some(r)) => {
            let idx = match column {
                "charisma" => 0,
                "intelligence" => 1,
                "memory" => 2,
                "perception" => 3,
                _ => 4,
            };
            r.get::<_, Option<i32>>(idx).map(|v| v as f64).unwrap_or(0.0)
        }
        Ok(None) => 0.0,
        Err(e) => {
            error!("character attribute {attribute_id} of {char_id}: {err}", err = crate::logging::chain(&e));
            0.0
        }
    }
}
