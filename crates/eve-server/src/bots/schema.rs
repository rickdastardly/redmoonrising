//! The tables the bot simulation keeps beyond the characters themselves,
//! made on start where an older database lacks them (the dumps in `db/`
//! hold them too, for a fresh one).

use tokio_postgres::Client;

const STATEMENTS: &[&str] = &[
    // The ship class a bot has worked its way up to (`ships`).
    r#"ALTER TABLE "bots" ADD COLUMN IF NOT EXISTS "tier" smallint NOT NULL DEFAULT 0"#,
    // What the bots hold in each station, between them (`economy`).
    r#"CREATE TABLE IF NOT EXISTS "botStock" (
           "stationID" integer NOT NULL,
           "typeID" integer NOT NULL,
           "quantity" bigint NOT NULL DEFAULT 0,
           PRIMARY KEY ("stationID", "typeID"))"#,
    r#"COMMENT ON TABLE "botStock" IS 'MADE: goods the bot simulation holds in each station: ore mined, minerals refined, hulls built (bots::economy).'"#,
    // How each bot side feels about each player (`relations`).
    r#"CREATE TABLE IF NOT EXISTS "botRelations" (
           "sideID" integer NOT NULL,
           "charID" integer NOT NULL,
           "heat" real NOT NULL DEFAULT 0,
           "favor" real NOT NULL DEFAULT 0,
           "flags" integer NOT NULL DEFAULT 0,
           "updatedAt" bigint NOT NULL DEFAULT 0,
           PRIMARY KEY ("sideID", "charID"))"#,
    r#"COMMENT ON TABLE "botRelations" IS 'MADE: what each bot alliance (or corporation outside one) holds against or for each player: heat from losses, favor from help, and what it has done about it (bots::relations).'"#,
    // How busy each region is, against the usual (`control`).
    r#"CREATE TABLE IF NOT EXISTS "botRegionDensity" (
           "regionID" integer PRIMARY KEY,
           "factor" real NOT NULL DEFAULT 1)"#,
    r#"COMMENT ON TABLE "botRegionDensity" IS 'MADE: how much more or less the bots living in a region play than usual, set from the dashboard or /bots density (bots::control).'"#,
    // The bots' sovereignty battles, for the killboard's battle reports (`battle`).
    r#"CREATE TABLE IF NOT EXISTS "rmrBattleReports" (
           "battleID" bigserial PRIMARY KEY,
           "solarSystemID" integer NOT NULL,
           "systemName" text NOT NULL DEFAULT '',
           "regionName" text NOT NULL DEFAULT '',
           "stakes" text NOT NULL DEFAULT '',
           "attackerID" integer NOT NULL,
           "attackerName" text NOT NULL DEFAULT '',
           "attackerAllies" text NOT NULL DEFAULT '',
           "defenderID" integer NOT NULL,
           "defenderName" text NOT NULL DEFAULT '',
           "defenderAllies" text NOT NULL DEFAULT '',
           "attackAlliances" integer[] NOT NULL DEFAULT '{}',
           "defendAlliances" integer[] NOT NULL DEFAULT '{}',
           "startedAt" bigint NOT NULL,
           "endedAt" bigint,
           "attackBrought" integer NOT NULL DEFAULT 0,
           "defendBrought" integer NOT NULL DEFAULT 0,
           "attackLost" integer NOT NULL DEFAULT 0,
           "defendLost" integer NOT NULL DEFAULT 0,
           "attackCapitals" text NOT NULL DEFAULT '',
           "defendCapitals" text NOT NULL DEFAULT '',
           "attackCapitalsLost" text NOT NULL DEFAULT '',
           "defendCapitalsLost" text NOT NULL DEFAULT '',
           "outcome" text NOT NULL DEFAULT '',
           "attackersWon" boolean)"#,
    r#"CREATE INDEX IF NOT EXISTS "rmrBattleReports_started" ON "rmrBattleReports" ("startedAt" DESC)"#,
    r#"COMMENT ON TABLE "rmrBattleReports" IS 'MADE: the bots'' sovereignty battles: who fought, what they brought (capitals too), what they lost and how it ended, for the dashboard killboard''s battle reports (bots::battle).'"#,
    // A battle cut short by a restart never ends: close it.
    r#"UPDATE "rmrBattleReports" SET "endedAt" = "startedAt", "outcome" = 'Cut short when the server stopped.' WHERE "endedAt" IS NULL"#,
    // Kill reports share one sequence: losses.rs and the bots both take from it.
    r#"SELECT setval(pg_get_serial_sequence('"killLog"', 'killID'), GREATEST((SELECT COALESCE(MAX("killID"), 0) FROM "killLog"), 1))"#,
];

pub async fn ensure(db: &Client) -> Result<(), tokio_postgres::Error> {
    for sql in STATEMENTS {
        db.execute(*sql, &[]).await?;
    }
    Ok(())
}
