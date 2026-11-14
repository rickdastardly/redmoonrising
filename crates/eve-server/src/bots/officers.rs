//! Bot corporations run themselves: their pilots hold the corporation's
//! offices, with the roles that go with them, and do the corporation's work.
//!
//! - Offices (`appoint`, every politics tick): the CEO, a director for about
//!   every 50 pilots (one to four), then a Personnel Manager, Accountant,
//!   Head of Security, Starbase Manager, Station Manager, Factory Manager and
//!   Trader, by seniority, as many as the corporation has pilots for. Each
//!   holds the real corpRole* bits (Members in the Corporation window, Show
//!   Info) and their office as their title. A CEO who left is succeeded by
//!   the most senior director; an officer who left, by the next in line.
//! - Their work: the Personnel Manager answers applications and signs the
//!   letters; the Head of Security declares the side hostile, puts bounties
//!   up and declares wars; the Accountant pays the corporation's and its
//!   alliance's bills (`pay_bills`) and the mercenary contracts; the
//!   Starbase Manager anchors, fuels and onlines towers, the Station Manager
//!   raises outposts, and the CEO answers the alliance (`claim`).

use tokio_postgres::Client;
use tracing::info;

use crate::svc::rows::{float, int, text};

/// An office in a bot corporation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Office {
    Ceo,
    Director,
    Recruiter,
    Accountant,
    Security,
    Starbase,
    Stations,
    Industry,
    Trade,
}

/// eveConstants corpRole*.
const DIRECTOR: i64 = 1;
const PERSONNEL_MANAGER: i64 = 128;
const ACCOUNTANT: i64 = 256;
const SECURITY_OFFICER: i64 = 512;
const FACTORY_MANAGER: i64 = 1024;
const STATION_MANAGER: i64 = 2048;
const EQUIPMENT_CONFIG: i64 = 2_199_023_255_552;
const CAN_RENT_OFFICE: i64 = 562_949_953_421_312;
const CAN_RENT_FACTORY_SLOT: i64 = 1_125_899_906_842_624;
const CAN_RENT_RESEARCH_SLOT: i64 = 2_251_799_813_685_248;
const JUNIOR_ACCOUNTANT: i64 = 4_503_599_627_370_496;
const STARBASE_CONFIG: i64 = 9_007_199_254_740_992;
const TRADER: i64 = 18_014_398_509_481_984;

/// The offices after the directors, in the order they are filled: seat,
/// office, title, roles.
const SEATS: [(i32, Office, &str, i64); 7] = [
    (1, Office::Recruiter, "Personnel Manager", PERSONNEL_MANAGER),
    (2, Office::Accountant, "Accountant", ACCOUNTANT | JUNIOR_ACCOUNTANT),
    (3, Office::Security, "Head of Security", SECURITY_OFFICER),
    (4, Office::Starbase, "Starbase Manager", STARBASE_CONFIG | EQUIPMENT_CONFIG),
    (5, Office::Stations, "Station Manager", STATION_MANAGER | CAN_RENT_OFFICE),
    (6, Office::Industry, "Factory Manager", FACTORY_MANAGER | CAN_RENT_FACTORY_SLOT | CAN_RENT_RESEARCH_SLOT),
    (7, Office::Trade, "Trader", TRADER),
];

impl Office {
    pub fn title(self) -> &'static str {
        match self {
            Office::Ceo => "CEO",
            Office::Director => "Director",
            other => SEATS.iter().find(|s| s.1 == other).map_or("", |s| s.2),
        }
    }
}

/// Every title an office carries, for telling a bot's office from a title a
/// player gave.
fn titles() -> Vec<&'static str> {
    let mut v = vec![Office::Ceo.title(), Office::Director.title()];
    v.extend(SEATS.iter().map(|s| s.2));
    v
}

/// Fills every bot corporation's offices: CEOs where theirs left, then the
/// roles and titles. Returns how many CEOs were succeeded.
pub async fn appoint(db: &Client) -> Result<u64, tokio_postgres::Error> {
    // A CEO no longer in the corporation is succeeded by its most senior
    // director, else its most senior pilot.
    let succeeded = db
        .query(
            r#"WITH heirs AS (
                   SELECT DISTINCT ON (c."corporationID") c."corporationID" AS corp, c."characterID" AS heir
                     FROM "character_" c
                     JOIN "botGroups" g ON g."ownerID" = c."corporationID"
                     JOIN "bots" b ON b."characterID" = c."characterID"
                     LEFT JOIN "chrCorporationRoles" r ON r."characterID" = c."characterID"
                    ORDER BY c."corporationID", (COALESCE(r."rolesAtAll", 0) & 1) DESC, c."corporationDateTime", c."characterID")
               UPDATE "corporation" co SET "ceoID" = h.heir
                 FROM heirs h
                WHERE co."corporationID" = h.corp AND co."deleted" = 0
                  AND NOT EXISTS (SELECT 1 FROM "character_" x WHERE x."characterID" = co."ceoID" AND x."corporationID" = co."corporationID")
               RETURNING co."corporationID""#,
            &[],
        )
        .await?
        .len() as u64;
    if succeeded > 0 {
        info!("bots: {succeeded} bot corporation(s) have a new CEO, their old one gone");
    }
    let all = crate::svc::membership::ALL_ROLES;
    let mut seats = format!("(-1, 'CEO', {all}::bigint, {all}::bigint), (0, 'Director', {DIRECTOR}::bigint, {all}::bigint)");
    for (seat, _, title, roles) in SEATS {
        seats.push_str(&format!(", ({seat}, '{title}', {roles}::bigint, 0::bigint)"));
    }
    let sql = format!(
        r#"WITH m AS (
               SELECT c."characterID" AS id,
                      row_number() OVER (PARTITION BY c."corporationID" ORDER BY c."corporationDateTime", c."characterID") AS n,
                      count(*) OVER (PARTITION BY c."corporationID") AS size
                 FROM "character_" c
                 JOIN "botGroups" g ON g."ownerID" = c."corporationID"
                 JOIN "corporation" co ON co."corporationID" = c."corporationID"
                 JOIN "bots" b ON b."characterID" = c."characterID"
                WHERE c."characterID" <> co."ceoID"),
           want AS (
               SELECT id, CASE WHEN n <= d THEN 0 ELSE n - d END AS seat
                 FROM (SELECT id, n, LEAST(4, GREATEST(1, size / 50)) AS d FROM m) x
               UNION ALL
               SELECT co."ceoID", -1 FROM "corporation" co JOIN "botGroups" g ON g."ownerID" = co."corporationID"
                WHERE co."deleted" = 0 AND co."ceoID" > 0
               UNION ALL
               -- Bots who left for a player's corporation drop the office they held.
               SELECT c."characterID", NULL FROM "character_" c JOIN "bots" b ON b."characterID" = c."characterID"
                WHERE c."title" = ANY($1) AND NOT EXISTS (SELECT 1 FROM "botGroups" g WHERE g."ownerID" = c."corporationID")),
           offices(seat, title, roles, grantable) AS (VALUES {seats}),
           target AS (
               SELECT w.id, COALESCE(o.title, '') AS title, COALESCE(o.roles, 0) AS roles, COALESCE(o.grantable, 0) AS grantable
                 FROM want w LEFT JOIN offices o ON o.seat = w.seat),
           missing AS (
               INSERT INTO "chrCorporationRoles" ("characterID", corprole, "rolesAtAll", "grantableRoles")
               SELECT t.id, t.roles, t.roles, t.grantable FROM target t
                WHERE t.roles <> 0 AND NOT EXISTS (SELECT 1 FROM "chrCorporationRoles" r WHERE r."characterID" = t.id)
               RETURNING 1),
           roles AS (
               UPDATE "chrCorporationRoles" r SET "rolesAtAll" = t.roles, corprole = t.roles, "grantableRoles" = t.grantable
                 FROM target t
                WHERE r."characterID" = t.id AND (r."rolesAtAll" <> t.roles OR r."grantableRoles" <> t.grantable)
               RETURNING 1)
           UPDATE "character_" c SET "title" = t.title
             FROM target t
            WHERE c."characterID" = t.id AND c."title" <> t.title"#
    );
    let titled = db.execute(sql.as_str(), &[&titles()]).await?;
    if titled > 0 {
        info!("bots: {titled} bot pilot(s) took up or left an office in their corporation");
    }
    Ok(succeeded)
}

/// Who holds `office` in a corporation, falling back to its CEO: (character,
/// name, title).
pub async fn holder(db: &Client, corp: i32, office: Office) -> Option<(i32, String, String)> {
    let sql = r#"SELECT * FROM (
                     SELECT c."characterID", c."characterName", c."title", 0 AS k FROM "character_" c
                      WHERE c."corporationID" = $1 AND c."title" = $2
                     UNION ALL
                     SELECT c."characterID", c."characterName", 'CEO', 1 FROM "corporation" co
                       JOIN "character_" c ON c."characterID" = co."ceoID"
                      WHERE co."corporationID" = $1) x
                  ORDER BY k, "characterID" LIMIT 1"#;
    let r = db.query_opt(sql, &[&corp, &office.title()]).await.ok().flatten()?;
    Some((int(&r, 0)? as i32, text(&r, 1).unwrap_or_default(), text(&r, 2).unwrap_or_default()))
}

/// A letter's signature: name, office and corporation.
pub async fn signature(db: &Client, corp: i32, office: Office) -> Option<(i32, String)> {
    let (id, name, title) = holder(db, corp, office).await?;
    let corp_name = db
        .query_opt(r#"SELECT "corporationName" FROM "corporation" WHERE "corporationID" = $1"#, &[&corp])
        .await
        .ok()
        .flatten()
        .and_then(|r| text(&r, 0))
        .unwrap_or_default();
    Some((id, format!("<br><br>{name}<br>{title}, {corp_name}")))
}

/// The Accountants at work: every unpaid bill of a bot corporation, or of a
/// bot alliance (its executor pays), is paid where the wallet can stand it.
/// Returns (bills, ISK) paid.
pub async fn pay_bills(db: &Client) -> Result<(u64, f64), tokio_postgres::Error> {
    let rows = db
        .query(
            r#"SELECT b."billID", b."amount", COALESCE(al."executorCorpID", b."debtorID") AS payer
                 FROM "billsReceivable" b
                 LEFT JOIN alliances al ON al."allianceID" = b."debtorID"
                WHERE b."paid" = 0
                  AND EXISTS (SELECT 1 FROM "botGroups" g WHERE g."ownerID" = COALESCE(al."executorCorpID", b."debtorID"))
                ORDER BY b."billID" LIMIT 500"#,
            &[],
        )
        .await?;
    let (mut bills, mut isk) = (0u64, 0.0f64);
    for r in rows {
        let (bill, amount, payer) = (int(&r, 0).unwrap_or(0) as i32, float(&r, 1).unwrap_or(0.0), int(&r, 2).unwrap_or(0) as i32);
        let paid = db
            .execute(
                r#"WITH paid AS (UPDATE "corporation" SET "balance" = "balance" - $3 WHERE "corporationID" = $2 AND "balance" >= $3 RETURNING 1)
                   UPDATE "billsReceivable" SET "paid" = 1 WHERE "billID" = $1 AND "paid" = 0 AND EXISTS (SELECT 1 FROM paid)"#,
                &[&bill, &payer, &amount],
            )
            .await?;
        if paid > 0 {
            bills += 1;
            isk += amount;
        }
    }
    if bills > 0 {
        info!("bots: accountants paid {bills} bill(s), {:.1}M ISK", isk / 1.0e6);
    }
    Ok((bills, isk))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_office_has_its_own_title() {
        let all = titles();
        let mut unique = all.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), all.len());
        assert_eq!(Office::Starbase.title(), "Starbase Manager");
        assert_eq!(Office::Ceo.title(), "CEO");
    }
}
