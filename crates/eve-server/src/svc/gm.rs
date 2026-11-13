//! Game master tools: `slash` (the chat window's /commands, lscchannel.py:541,
//! and the GM menu's "TR me here!"), and the dungeon tools' services, which
//! have no dungeons to work on.
//!
//! The client sends slash commands only from a session with ROLE_GML or
//! ROLE_LEGIONEER (lscchannel.py ROLE_SLASH), and the server holds to the
//! same. The commands:
//! - /create <type> [quantity]: into the hangar, or the hold in space;
//! - /gmships: one of each staff, Jovian and test hull in your station hangar;
//! - /giveisk <amount>;
//! - /giveskill <skill> [level]: trained, straight into the character;
//! - /giveallskills: every skill type at level 5 on your character;
//! - /setstanding <ownerID> <standing>: an NPC's standing toward you;
//! - /maxstandings: maximum standing from every agent, faction, and NPC corporation;
//! - /clearspace: remove every in-space cargo container and its contents;
//! - /spawn: a group of the system's rats, around your ship;
//! - /tr me <station | system | celestial | character>;
//! - /moveme [hub]: choose a market hub, or travel to one by name;
//! - /bots on|off|status: the bot population; /bots event <betrayal |
//!   merger | collapse | coalition | raid>, /bots war <a> | <b>, /bots
//!   density <region> <factor>, /bots follow <name>: steering it;
//! - /help.
//! A type or a skill is its ID or its name.

use eve_marshal::Value;
use std::collections::HashMap;
use tokio_postgres::GenericClient;
use tracing::info;

use crate::ballpark::Vec3;
use crate::items::{ItemTxn, NewItem};
use crate::services::{Call, Ctx, Outcome};
use crate::svc::rows::{float, int, text};
use crate::svc::wallet::{self, Entry};
use crate::world::Move;

pub(crate) const ROLE_GML: i64 = 8;
const ROLE_LEGIONEER: i64 = 262_144;
const FLAG_HANGAR: i64 = 4;
const FLAG_CARGO: i64 = 5;
const FLAG_SKILL: i64 = 7;
const FLAG_SKILL_IN_TRAINING: i64 = 61;
const CATEGORY_SKILL: i32 = 16;
const ATTR_SKILL_TIME_CONSTANT: i32 = 275;
const ATTR_SKILL_POINTS: i32 = 276;
const ATTR_SKILL_LEVEL: i32 = 280;
/// refAgentMiscellaneous, for money a GM conjures.
const REF_GM: i32 = 25;
const OWNER_SYSTEM: i64 = 1;

const HELP: &str = "/create <type> [qty], /gmships, /giveisk <amount>, /giveskill <skill> [level], /giveallskills, /setstanding <ownerID> <standing>, /maxstandings, /clearspace, /spawn, /moveme [hub], /outpostmats [platformID], /rushanchor [structureID], /tr me <place or pilot>, /bots on|off|status|event <kind>|war <a> | <b>|density <region> <x>|follow <name>|battle [here|system] [skirmish|battle|decisive]";

const JOVE_RACE: i32 = 16;
// Jove Frigate, Jove Cruiser, Jove Industrial, Jove Battleship and Polaris.
const SPECIAL_SHIP_SKILLS: &[i32] = &[3755, 3758, 11075, 11078, 9955];
// Developer hulls using ordinary skills in the RMR catalog.
const TEST_HULLS: &[i32] = &[
    11591, 12177, 12403, 20543, // Agility test, The Hammer, Atron much HP, LeMonde's capsule.
    19770, 19780, 19782, 20122, 20191, 20193, 20316, 20318, 20320, 20322, // Pimp variants.
    22212, 22215, 23693, 24298, 24611, // Scorpion and Rook variants, test freighter and titan.
    24688, 24690, 24692, 24694, 24696, 24698, 24700, 24702, // Tier test hulls.
];

// Stocked highsec stations and named nullsec destinations in the RMR map.
// Keep the client dialog's keys in step with this allowlist.
const MOVE_ME_HUBS: &[(&str, i64)] = &[
    ("jita", 60000361),
    ("amarr", 60008494),
    ("rens", 60004588),
    ("dodixie", 60011866),
    ("hek", 60005686),
    ("t22-qi", 60013417),
    ("fd-mlj", 60013360),
    ("x-7omu", 60012595),
    ("vfk-iv", 60014917),
    ("6nj8-v", 60012556),
    ("nol-m9", 60014926),
];

pub async fn slash(call: &Call, ctx: &Ctx<'_>) -> Option<Outcome> {
    if call.method != "SlashCmd" {
        return None;
    }
    if ctx.session.role & (ROLE_GML | ROLE_LEGIONEER) == 0 {
        return Some(Outcome::custom("You are not allowed to use slash commands."));
    }
    let line = call.arg_text(0).unwrap_or_default();
    let mut words = line.trim().trim_start_matches('/').split_whitespace();
    let command = words.next().unwrap_or("").to_lowercase();
    let rest: Vec<&str> = words.collect();
    let me = ctx.session.char_id.unwrap_or(0);
    let result = match command.as_str() {
        "help" | "" => Ok(HELP.to_string()),
        "create" | "createitem" => create(ctx, &rest).await,
        "gmships" => gm_ships(ctx, &rest).await,
        "giveisk" | "givemoney" => give_isk(ctx, &rest).await,
        "giveskill" => give_skill(ctx, &rest).await,
        "giveallskills" => give_all_skills(ctx, &rest).await,
        "setstanding" => set_standing(ctx, &rest).await,
        "maxstandings" => max_standings(ctx, &rest).await,
        "clearspace" => clear_space(ctx, &rest).await,
        "spawn" => spawn(ctx),
        "moveme" => move_me(ctx, &rest).await,
        "rushanchor" => match rest.as_slice() {
            [] => crate::svc::starbase::rush(ctx, None).await,
            [id] => match id.parse::<i64>() {
                Ok(id) => crate::svc::starbase::rush(ctx, Some(id)).await,
                Err(_) => Err("Usage: /rushanchor [structureID]".into()),
            },
            _ => Err("Usage: /rushanchor [structureID]".into()),
        },
        "outpostmats" => match rest.as_slice() {
            [] => crate::svc::outpost::spawn_materials(ctx, None).await,
            [id] => match id.parse::<i64>() {
                Ok(id) => crate::svc::outpost::spawn_materials(ctx, Some(id)).await,
                Err(_) => Err("Usage: /outpostmats [platformID]".into()),
            },
            _ => Err("Usage: /outpostmats [platformID]".into()),
        },
        "tr" => teleport(ctx, &rest).await,
        "bots" => match rest.first().copied() {
            Some("on") => {
                crate::bots::set_running(true);
                Ok("The bots are running.".into())
            }
            Some("off") => {
                crate::bots::set_running(false);
                Ok("The bots are paused.".into())
            }
            Some("status") | None => Ok(crate::bots::status()),
            // `/bots battle here`: the system the GM is in.
            Some("battle") if rest.get(1).is_some_and(|w| w.eq_ignore_ascii_case("here")) => {
                match ctx.session.solar_system_id2.or(ctx.session.solar_system_id) {
                    Some(system) => {
                        let system = system.to_string();
                        let mut words: Vec<&str> = rest.to_vec();
                        words[1] = system.as_str();
                        crate::bots::request(&words)
                    }
                    None => Err("You are not in a solar system.".into()),
                }
            }
            Some(_) => crate::bots::request(&rest),
        },
        other => Err(format!("Unknown command /{other}. {HELP}")),
    };
    info!("{me} /{command} {}: {}", rest.join(" "), match &result { Ok(r) => r.as_str(), Err(e) => e.as_str() });
    Some(match result {
        Ok(reply) => Outcome::Value(Value::text(reply)),
        Err(why) => Outcome::custom(why),
    })
}

/// Remove cargo containers in solar-system space, including their contents,
/// in one transaction. Station hangars, ships, and other inventory locations
/// are outside the selected roots. Then remove the loaded balls from every
/// active park so clients stop displaying them immediately.
async fn clear_space(ctx: &Ctx<'_>, words: &[&str]) -> Result<String, String> {
    if !words.is_empty() { return Err("Usage: /clearspace".into()); }
    let mut db = crate::db::connect(ctx.db_url).await.map_err(|e| e.to_string())?;
    let tx = db.transaction().await.map_err(|e| e.to_string())?;
    let roots: Vec<i32> = tx.query(r#"
        SELECT e."itemID" FROM entity e
          JOIN "invTypes" t ON t."typeID" = e."typeID"
          JOIN "mapSolarSystems" s ON s."solarSystemID" = e."locationID"
         WHERE t."groupID" = 12
         FOR UPDATE OF e
    "#, &[]).await.map_err(|e| e.to_string())?
        .iter().map(|row| row.get(0)).collect();
    if roots.is_empty() { return Ok("Space contains no cargo containers.".into()); }
    let items: Vec<i32> = tx.query(r#"
        WITH RECURSIVE contents("itemID", depth) AS (
            SELECT unnest($1::int[]), 0
            UNION ALL
            SELECT e."itemID", contents.depth + 1
              FROM entity e JOIN contents ON e."locationID" = contents."itemID"
             WHERE contents.depth < 32
        )
        SELECT DISTINCT "itemID" FROM contents
    "#, &[&roots]).await.map_err(|e| e.to_string())?
        .iter().map(|row| row.get(0)).collect();
    tx.execute(r#"DELETE FROM entity_attributes WHERE "itemID" = ANY($1)"#, &[&items])
        .await.map_err(|e| e.to_string())?;
    tx.execute(r#"DELETE FROM entity WHERE "itemID" = ANY($1)"#, &[&items])
        .await.map_err(|e| e.to_string())?;
    tx.commit().await.map_err(|e| e.to_string())?;
    crate::loot::forget_cans(ctx.world, &roots);
    Ok(format!("Cleared {} cargo containers and {} contained items from space in all systems.",
        roots.len(), items.len().saturating_sub(roots.len())))
}

/// A type by ID, or by its name (exactly, else the first that starts so).
async fn find_type(db: &impl GenericClient, words: &[&str], category: Option<i32>) -> Option<(i64, String)> {
    let key = words.join(" ");
    let sql = r#"SELECT t."typeID", t."typeName" FROM "invTypes" t JOIN "invGroups" g ON g."groupID" = t."groupID"
                  WHERE ($3::int IS NULL OR g."categoryID" = $3)
                    AND (t."typeID"::text = $1 OR lower(t."typeName") = lower($1) OR lower(t."typeName") LIKE lower($2))
                  ORDER BY (t."typeID"::text = $1) DESC, (lower(t."typeName") = lower($1)) DESC, length(t."typeName")
                  LIMIT 1"#;
    let r = db.query_opt(sql, &[&key, &format!("{key}%"), &category]).await.ok().flatten()?;
    Some((int(&r, 0)?, text(&r, 1).unwrap_or_default()))
}

/// The trailing number of a command, if there is one, and the words before.
fn trailing_number<'a>(words: &'a [&'a str]) -> (&'a [&'a str], Option<f64>) {
    match words.split_last() {
        Some((last, before)) if !before.is_empty() => match last.replace(',', "").parse::<f64>() {
            Ok(n) => (before, Some(n)),
            Err(_) => (words, None),
        },
        _ => (words, None),
    }
}

async fn create(ctx: &Ctx<'_>, words: &[&str]) -> Result<String, String> {
    let me = ctx.session.char_id.unwrap_or(0);
    let (name, quantity) = trailing_number(words);
    let quantity = quantity.unwrap_or(1.0);
    if !quantity.is_finite() || !(1.0..=i32::MAX as f64).contains(&quantity) {
        return Err(format!("Enter a quantity from 1 to {}. Larger stacks overflow the EVE client.", i32::MAX));
    }
    let quantity = quantity as i64;
    let (type_id, type_name) = find_type(ctx.db, name, None).await.ok_or("No such type.")?;
    let (location, flag) = match (ctx.session.station_id, ctx.session.ship_id) {
        (Some(station), _) => (station, FLAG_HANGAR),
        (None, Some(ship)) => (ship, FLAG_CARGO),
        _ => return Err("You have nowhere to put it.".into()),
    };
    let mut client = crate::db::connect(ctx.db_url).await.map_err(|e| e.to_string())?;
    let mut txn = ItemTxn::begin(&mut client).await.map_err(|e| e.to_string())?;
    txn.create(NewItem { type_id, owner_id: me, location_id: location, flag, quantity, singleton: false })
        .await
        .map_err(|e| e.to_string())?;
    let events = txn.commit().await.map_err(|e| e.to_string())?;
    crate::items::notify(ctx.world, ctx.client_id, events);
    Ok(format!("Created {quantity} x {type_name}"))
}

async fn gm_ships(ctx: &Ctx<'_>, words: &[&str]) -> Result<String, String> {
    if !words.is_empty() { return Err("Usage: /gmships".into()); }
    let owner_id = ctx.session.char_id.ok_or("Select a character first.")?;
    let location_id = ctx.session.station_id.ok_or("Dock at a station to use /gmships.")?;
    let mut client = crate::db::connect(ctx.db_url).await.map_err(|e| e.to_string())?;
    let mut txn = ItemTxn::begin(&mut client).await.map_err(|e| e.to_string())?;
    let types = txn.tx().query(r#"
        SELECT t."typeID" FROM "invTypes" t
          JOIN "invGroups" g ON g."groupID" = t."groupID"
         WHERE g."categoryID" = $1
           AND (t."raceID" = $2 OR t."typeID" = ANY($3) OR EXISTS (
                SELECT 1 FROM "dgmTypeAttributes" a
                 WHERE a."typeID" = t."typeID" AND a."attributeID" = ANY($4)
                   AND COALESCE(a."valueInt"::float8, a."valueFloat") = ANY($5::int[])
           ))
         ORDER BY t."typeID"
    "#, &[&(crate::dogma::category::SHIP as i32), &JOVE_RACE, &TEST_HULLS,
          &&crate::dogma::attr::REQUIRED_SKILLS[..], &SPECIAL_SHIP_SKILLS])
        .await.map_err(|e| e.to_string())?;
    if types.is_empty() { return Err("No GM or Jovian ship types were found in the game data.".into()); }

    for row in &types {
        let type_id = i64::from(row.get::<_, i32>(0));
        txn.create(NewItem { type_id, owner_id, location_id, flag: FLAG_HANGAR, quantity: 1, singleton: false })
            .await.map_err(|e| e.to_string())?;
    }
    let events = txn.commit().await.map_err(|e| e.to_string())?;
    crate::items::notify(ctx.world, ctx.client_id, events);
    Ok(format!("Created {} GM, Jovian and test hulls in your personal ship hangar.", types.len()))
}

async fn give_isk(ctx: &Ctx<'_>, words: &[&str]) -> Result<String, String> {
    let me = ctx.session.char_id.unwrap_or(0);
    let amount: f64 = words.first().and_then(|w| w.replace(',', "").parse().ok()).ok_or("How much?")?;
    let mut client = crate::db::connect(ctx.db_url).await.map_err(|e| e.to_string())?;
    let tx = client.transaction().await.map_err(|e| e.to_string())?;
    let entry = Entry { ref_type: REF_GM, payer: OWNER_SYSTEM, payee: me, arg_id1: "", reason: "GM" };
    let balance = wallet::post(&tx, me, amount, &entry).await.map_err(|_| "The wallet would go below zero.".to_string())?;
    tx.commit().await.map_err(|e| e.to_string())?;
    let w = ctx.world.lock();
    let cash = Value::Tuple(vec![Value::bytes("cash"), Value::Int(me), Value::Double(balance)]);
    w.notify(&w.clients_of(me), "OnAccountChange", cash);
    Ok(format!("Your balance is now {balance:.2} ISK"))
}

/// A trained skill, made in the character (or an existing one raised).
/// The skill list shows it after the next login.
async fn give_skill(ctx: &Ctx<'_>, words: &[&str]) -> Result<String, String> {
    let me = ctx.session.char_id.unwrap_or(0);
    let (name, level) = trailing_number(words);
    let level = level.unwrap_or(5.0).clamp(0.0, 5.0) as i64;
    let (type_id, skill_name) = find_type(ctx.db, name, Some(CATEGORY_SKILL)).await.ok_or("No such skill.")?;
    let stc = ctx
        .query_one(
            r#"SELECT COALESCE("valueFloat", "valueInt"::float8) FROM "dgmTypeAttributes" WHERE "typeID" = $1 AND "attributeID" = $2"#,
            &[&(type_id as i32), &ATTR_SKILL_TIME_CONSTANT],
        )
        .await
        .and_then(|r| float(&r, 0))
        .unwrap_or(1.0)
        * 250.0;
    let points = crate::losses::points_at(stc, level);
    let mut client = crate::db::connect(ctx.db_url).await.map_err(|e| e.to_string())?;
    let mut txn = ItemTxn::begin(&mut client).await.map_err(|e| e.to_string())?;
    let held = r#"SELECT "itemID" FROM entity WHERE "locationID" = $1 AND "typeID" = $2 AND "flag" = $3"#;
    let existing = txn
        .tx()
        .query_opt(held, &[&(me as i32), &(type_id as i32), &(FLAG_SKILL as i32)])
        .await
        .map_err(|e| e.to_string())?
        .and_then(|r| int(&r, 0));
    let id = match existing {
        Some(id) => id,
        None => txn
            .create(NewItem { type_id, owner_id: me, location_id: me, flag: FLAG_SKILL, quantity: 1, singleton: true })
            .await
            .map_err(|e| e.to_string())?,
    };
    let set = r#"INSERT INTO entity_attributes ("itemID", "attributeID", "valueInt", "valueFloat") VALUES ($1, $2, $3, NULL)
                 ON CONFLICT ("itemID", "attributeID") DO UPDATE SET "valueInt" = $3, "valueFloat" = NULL"#;
    for (attr, value) in [(ATTR_SKILL_LEVEL, level), (ATTR_SKILL_POINTS, points.round() as i64)] {
        txn.tx().execute(set, &[&(id as i32), &attr, &value]).await.map_err(|e| e.to_string())?;
    }
    let events = txn.commit().await.map_err(|e| e.to_string())?;
    crate::items::notify(ctx.world, ctx.client_id, events);
    Ok(format!("{skill_name} is now at level {level} ({points:.0} SP); log in again to see it in the skill list"))
}

/// Grant every skill in the installed data set to this GM at level 5.
/// Reuse skills that are already trained or currently training, and commit the
/// complete grant in one transaction so a failure cannot leave a partial set.
async fn give_all_skills(ctx: &Ctx<'_>, words: &[&str]) -> Result<String, String> {
    if !words.is_empty() {
        return Err("Usage: /giveallskills".into());
    }
    let me = ctx.session.char_id.ok_or("Select a character first.")?;
    let mut client = crate::db::connect(ctx.db_url).await.map_err(|e| e.to_string())?;
    let mut txn = ItemTxn::begin(&mut client).await.map_err(|e| e.to_string())?;

    let types = txn
        .tx()
        .query(
            r#"SELECT t."typeID", COALESCE(max(COALESCE(a."valueFloat", a."valueInt"::float8)), 1.0)
                 FROM "invTypes" t
                 JOIN "invGroups" g ON g."groupID" = t."groupID"
                 LEFT JOIN "dgmTypeAttributes" a ON a."typeID" = t."typeID" AND a."attributeID" = $1
                WHERE g."categoryID" = $2
                GROUP BY t."typeID"
                ORDER BY t."typeID""#,
            &[&ATTR_SKILL_TIME_CONSTANT, &CATEGORY_SKILL],
        )
        .await
        .map_err(|e| e.to_string())?;
    if types.is_empty() {
        return Err("No skill types were found in the game data.".into());
    }

    let existing = txn
        .tx()
        .query(
            r#"SELECT e."itemID", e."typeID", e."flag"
                 FROM entity e
                 JOIN "invTypes" t ON t."typeID" = e."typeID"
                 JOIN "invGroups" g ON g."groupID" = t."groupID"
                WHERE e."ownerID" = $1 AND e."locationID" = $1
                  AND e."flag" IN ($2, $3) AND g."categoryID" = $4
                ORDER BY e."typeID", e."itemID"
                FOR UPDATE OF e"#,
            &[&(me as i32), &(FLAG_SKILL as i32), &(FLAG_SKILL_IN_TRAINING as i32), &CATEGORY_SKILL],
        )
        .await
        .map_err(|e| e.to_string())?;
    let mut held: HashMap<i32, Vec<(i64, i32)>> = HashMap::new();
    for row in existing {
        held.entry(row.get(1)).or_default().push((row.get::<_, i32>(0) as i64, row.get(2)));
    }

    let total = types.len();
    let mut added = 0;
    let mut item_ids = Vec::<i32>::new();
    let mut skill_points = Vec::<i64>::new();
    for row in types {
        let type_id: i32 = row.get(0);
        let rank: f64 = row.get(1);
        let rank = if rank.is_finite() && rank > 0.0 { rank } else { 1.0 };
        let points = crate::losses::points_at(rank * 250.0, 5).round() as i64;
        if let Some(items) = held.remove(&type_id) {
            for (id, flag) in items {
                if flag == FLAG_SKILL_IN_TRAINING as i32 {
                    txn.move_to(id, me, FLAG_SKILL).await.map_err(|e| e.to_string())?;
                }
                item_ids.push(id as i32);
                skill_points.push(points);
            }
        } else {
            let id = txn
                .create(NewItem { type_id: type_id as i64, owner_id: me, location_id: me, flag: FLAG_SKILL, quantity: 1, singleton: true })
                .await
                .map_err(|e| e.to_string())?;
            item_ids.push(id as i32);
            skill_points.push(points);
            added += 1;
        }
    }

    txn.tx()
        .execute(
            r#"INSERT INTO entity_attributes ("itemID", "attributeID", "valueInt", "valueFloat")
               SELECT s."itemID", a."attributeID",
                      CASE WHEN a."attributeID" = $3 THEN 5::bigint ELSE s."points" END, NULL
                 FROM unnest($1::int[], $2::bigint[]) AS s("itemID", "points")
                CROSS JOIN (VALUES ($3::int), ($4::int)) AS a("attributeID")
               ON CONFLICT ("itemID", "attributeID")
               DO UPDATE SET "valueInt" = EXCLUDED."valueInt", "valueFloat" = NULL"#,
            &[&item_ids, &skill_points, &ATTR_SKILL_LEVEL, &ATTR_SKILL_POINTS],
        )
        .await
        .map_err(|e| e.to_string())?;
    let events = txn.commit().await.map_err(|e| e.to_string())?;
    crate::items::notify(ctx.world, ctx.client_id, events);
    Ok(format!("All {total} skills are now at level 5 ({added} added). Log in again to refresh your skill list."))
}

async fn set_standing(ctx: &Ctx<'_>, words: &[&str]) -> Result<String, String> {
    let me = ctx.session.char_id.unwrap_or(0);
    let from: i64 = words.first().and_then(|w| w.parse().ok()).ok_or("Whose standing?")?;
    let standing: f64 = words.get(1).and_then(|w| w.parse().ok()).ok_or("What standing?")?;
    let standing = standing.clamp(-10.0, 10.0);
    let sql = r#"INSERT INTO "chrNPCStandings" ("characterID", "fromID", "standing") VALUES ($1, $2, $3)
                 ON CONFLICT ("characterID", "fromID") DO UPDATE SET "standing" = $3"#;
    if !ctx.execute(sql, &[&(me as i32), &(from as i32), &standing]).await {
        return Err("The standing could not be saved.".into());
    }
    let w = ctx.world.lock();
    w.notify(&w.clients_of(me), "OnStandingSet", Value::Tuple(vec![Value::Int(from), Value::Int(me), Value::Double(standing)]));
    Ok(format!("{from}'s standing toward you is now {standing:.2}"))
}

async fn max_standings(ctx: &Ctx<'_>, words: &[&str]) -> Result<String, String> {
    if !words.is_empty() {
        return Err("Usage: /maxstandings".into());
    }
    let me = ctx.session.char_id.ok_or("Select a character first.")? as i32;
    let sql = r#"WITH organizations AS (
                     SELECT "corporationID" AS id FROM "crpNPCCorporations"
                     UNION SELECT "factionID" FROM "chrFactions"
                 ), updated AS (
                     INSERT INTO "chrNPCStandings" ("characterID", "fromID", "standing")
                     SELECT $1, owners.id, 10.0
                       FROM (SELECT "agentID" AS id FROM "agtAgents"
                             UNION SELECT id FROM organizations) owners
                     ON CONFLICT ("characterID", "fromID") DO UPDATE SET "standing" = 10.0
                     RETURNING "fromID"
                 )
                 SELECT count(*), ARRAY(SELECT id FROM organizations ORDER BY id)
                   FROM updated"#;
    let row = ctx.db.query_one(sql, &[&me]).await.map_err(|e| e.to_string())?;
    let count: i64 = row.get(0);
    let organizations: Vec<i32> = row.get(1);

    // Refresh the legacy client's organization cache without loading every agent.
    let world = ctx.world.lock();
    let clients = world.clients_of(i64::from(me));
    for from in organizations {
        world.notify(&clients, "OnStandingSet", Value::Tuple(vec![
            Value::Int(i64::from(from)), Value::Int(i64::from(me)), Value::Double(10.0),
        ]));
    }
    Ok(format!("Maximum standing set for {count} agents, NPC corporations and factions. Reopen the Standings tab to see updated corporations and factions."))
}

fn spawn(ctx: &Ctx<'_>) -> Result<String, String> {
    let (Some(system), Some(ship_id)) = (ctx.session.solar_system_id, ctx.session.ship_id) else {
        return Err("You have to be in space.".into());
    };
    let mut w = ctx.world.lock();
    let (count, nearby) = {
        let park = w.parks.get_mut(&system).ok_or("There is no park here.")?;
        let at = park.ships.get(&ship_id).map(|s| s.pos).ok_or("Your ship is not here.")?;
        let security = park.wildlife.security;
        let rats = crate::npc::spawn_group(park, ship_id, at + Vec3::new(15_000.0, 0.0, 15_000.0), security, crate::combat::now_ms());
        let nearby = rats.iter().flat_map(|id| crate::svc::space::add_ball_entries_for_nearby(park, *id)).collect();
        (rats.len(), nearby)
    };
    crate::svc::space::notify_targeted_updates(&w, nearby);
    Ok(format!("{count} rat(s) spawned"))
}

/// /tr me <where>: a station docks you there; a system, a celestial in one or
/// another pilot's ship puts you in space beside it (from space only).
async fn teleport(ctx: &Ctx<'_>, words: &[&str]) -> Result<String, String> {
    let target = match words {
        ["me", rest @ ..] if !rest.is_empty() => rest.join(" "),
        _ => return Err("Usage: /tr me <station, system, celestial or pilot>".into()),
    };
    let place = r#"SELECT d."itemID", d."groupID", COALESCE(d."solarSystemID", d."itemID"), s."constellationID", s."regionID",
                          d."x", d."y", d."z", COALESCE(d."radius", 0), d."itemName"
                     FROM "mapDenormalize" d
                     JOIN "mapSolarSystems" s ON s."solarSystemID" = COALESCE(d."solarSystemID", d."itemID")
                    WHERE d."itemID"::text = $1 OR lower(d."itemName") = lower($1)
                    ORDER BY (d."itemID"::text = $1) DESC LIMIT 1"#;
    let pilot = r#"SELECT "characterID" FROM "character_" WHERE "characterID"::text = $1 OR lower("characterName") = lower($1) LIMIT 1"#;
    let me_in_space = ctx.session.solar_system_id.is_some();
    let (station, system, constellation, region, pos, name) = if let Some(r) = ctx.query_one(place, &[&target]).await {
        let i = |n| int(&r, n).unwrap_or(0);
        let at = Vec3::new(float(&r, 5).unwrap_or(0.0), float(&r, 6).unwrap_or(0.0), float(&r, 7).unwrap_or(0.0));
        let radius = float(&r, 8).unwrap_or(0.0);
        let station = (i(1) == 15).then_some(i(0));
        // Off a celestial's surface; to a system's sun, an AU out.
        let pos = if i(0) == i(2) { Vec3::new(1.5e11, 0.0, 0.0) } else { at + Vec3::new(radius + 10_000.0, 0.0, 0.0) };
        (station, i(2), i(3), i(4), pos, text(&r, 9).unwrap_or_default())
    } else if let Some(other) = ctx.query_one(pilot, &[&target]).await.and_then(|r| int(&r, 0)) {
        let presence = ctx.world.lock().presence_of(other).ok_or("That pilot is not online.")?;
        let sql = r#"SELECT "constellationID", "regionID" FROM "mapSolarSystems" WHERE "solarSystemID" = $1"#;
        let r = ctx.query_one(sql, &[&(presence.solar_system_id as i32)]).await.ok_or("That pilot is nowhere.")?;
        let pos = {
            let w = ctx.world.lock();
            w.parks
                .get(&presence.solar_system_id)
                .and_then(|p| p.ships.values().find(|s| s.pilot.char_id == other))
                .map_or(Vec3::new(1.5e11, 0.0, 0.0), |s| s.pos + Vec3::new(2_000.0, 0.0, 0.0))
        };
        let station = presence.station_id;
        (station, presence.solar_system_id, int(&r, 0).unwrap_or(0), int(&r, 1).unwrap_or(0), pos, target.clone())
    } else {
        return Err(format!("Nowhere called {target}."));
    };
    if station.is_none() && !me_in_space {
        return Err("Undock first: from a station, /tr only goes to other stations.".into());
    }
    let w = ctx.world.lock();
    w.push_move(ctx.client_id, Move::Teleport { station_id: station, solar_system_id: system, constellation_id: constellation, region_id: region, pos: (pos.x, pos.y, pos.z) });
    Ok(format!("On your way to {name}"))
}

async fn move_me(ctx: &Ctx<'_>, words: &[&str]) -> Result<String, String> {
    if words.is_empty() {
        ctx.world.lock().notify(
            &[ctx.client_id],
            "OnRemoteMessage",
            Value::Tuple(vec![Value::bytes("RmrMoveMe"), Value::Dict(Vec::new())]),
        );
        return Ok("Choose a destination in the Move Me window.".into());
    }
    let key = words.join(" ").to_ascii_lowercase();
    let (_, station) = MOVE_ME_HUBS.iter().find(|(name, _)| *name == key)
        .ok_or("Choose a listed hub in the Move Me window.")?;
    let station_id = station.to_string();
    teleport(ctx, &["me", &station_id]).await
}

/// `keeper` and `dungeon`: the dungeon editor and acceleration gates. There
/// are no dungeons, so there is nothing to edit or activate.
pub async fn dungeon_tools(call: &Call, ctx: &Ctx<'_>) -> Option<Outcome> {
    Some(match call.method.as_str() {
        "GetLevelEditor" => Outcome::Value(Value::None),
        "ActivateAccelerationGate" => crate::dungeons::activate(ctx, call.arg_i64(0).unwrap_or(0)),
        "DEGetDungeons" | "DEGetFactions" | "DEGetRooms" => Outcome::Value(Value::List(Vec::new())),
        "EditObject" | "CopyObject" | "RemoveObject" => Outcome::custom("There are no dungeons to edit."),
        _ => return None,
    })
}

/// `benchmark`: SaveSession(session, dbconn), a client's performance dump
/// (benchmarkSvc.py), noted and dropped.
pub async fn benchmark(call: &Call, ctx: &Ctx<'_>) -> Option<Outcome> {
    if call.method != "SaveSession" {
        return None;
    }
    info!("benchmark session from {:?} not stored", ctx.session.char_id);
    Some(Outcome::Value(Value::None))
}
