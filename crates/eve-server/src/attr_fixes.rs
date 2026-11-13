//! Repairs to the item attributes the client is sent (dgmTypeAttribs, the
//! bulk data behind Show Info and the fitting window).
//!
//! That table was made from a source that stored small negative numbers as
//! unsigned bytes and large ones as unsigned 64-bit numbers, so Gunnery's
//! -2% rate of fire showed as 254% and a -20,000 bonus as 18 quintillion.
//! The server's own table (dgmTypeAttributes) has the signed values; where
//! the two differ by exactly 256, 65,536, 2^32 or 2^64 the client's copy
//! takes the signed value. A few rows only the client's table has, for
//! attributes whose values in this range are all wrapped, are fixed the same
//! way.
//!
//! Skills are then made to agree: the server trains, checks prerequisites and
//! applies bonuses with the values the client shows (ranks, requirements and
//! bonuses as Red Moon Rising had them), and drops requirements the client
//! does not know of.

use tracing::info;

const CATEGORY_SKILL: i32 = 16;

pub async fn repair(db: &mut tokio_postgres::Client) -> Result<(), tokio_postgres::Error> {
    let tx = db.transaction().await?;
    tx.batch_execute(
        r#"CREATE TEMP TABLE rmr_client_attrs ON COMMIT DROP AS
             SELECT "typeID", "attributeID", value::numeric AS v FROM "dgmTypeAttribs"
              WHERE value ~ '^-?[0-9]+(\.[0-9]+)?$';
           CREATE TEMP TABLE rmr_server_attrs ON COMMIT DROP AS
             SELECT "typeID", "attributeID", COALESCE("valueFloat", "valueInt"::float8)::numeric AS v FROM "dgmTypeAttributes"
              WHERE COALESCE("valueFloat", "valueInt"::float8) IS NOT NULL;
           CREATE INDEX ON rmr_client_attrs ("typeID", "attributeID");
           CREATE INDEX ON rmr_server_attrs ("typeID", "attributeID");"#,
    )
    .await?;

    // Rows only the client has, for attributes whose 128-255 values are all
    // wrapped negatives elsewhere.
    let lone = tx
        .execute(
            r#"WITH wrapped AS (SELECT DISTINCT c."attributeID" FROM rmr_client_attrs c JOIN rmr_server_attrs s USING ("typeID", "attributeID") WHERE c.v - s.v = 256),
                    legit AS (SELECT DISTINCT c."attributeID" FROM rmr_client_attrs c JOIN rmr_server_attrs s USING ("typeID", "attributeID") WHERE c.v BETWEEN 128 AND 255 AND c.v = s.v)
               UPDATE "dgmTypeAttribs" t SET value = ((c.v - 256)::bigint)::text
                 FROM rmr_client_attrs c
                WHERE t."typeID" = c."typeID" AND t."attributeID" = c."attributeID"
                  AND c."attributeID" IN (SELECT "attributeID" FROM wrapped)
                  AND c."attributeID" NOT IN (SELECT "attributeID" FROM legit)
                  AND c.v BETWEEN 128 AND 255 AND c.v = trunc(c.v)
                  AND NOT EXISTS (SELECT 1 FROM rmr_server_attrs s WHERE s."typeID" = c."typeID" AND s."attributeID" = c."attributeID")"#,
            &[],
        )
        .await?;
    let wrapped = tx
        .execute(
            r#"UPDATE "dgmTypeAttribs" t
                  SET value = CASE WHEN s.v = trunc(s.v) THEN (s.v::bigint)::text ELSE s.v::text END
                 FROM rmr_client_attrs c JOIN rmr_server_attrs s USING ("typeID", "attributeID")
                WHERE t."typeID" = c."typeID" AND t."attributeID" = c."attributeID" AND s.v < 0
                  AND c.v - s.v IN (256, 65536, 4294967296, 18446744073709551616)"#,
            &[],
        )
        .await?;

    // Skills: the server goes by what the client shows.
    let skills = tx
        .execute(
            r#"UPDATE "dgmTypeAttributes" d
                  SET "valueInt" = CASE WHEN t.value ~ '^-?[0-9]+$' AND abs(t.value::numeric) < 2147483648 THEN t.value::int END,
                      "valueFloat" = CASE WHEN t.value ~ '^-?[0-9]+$' AND abs(t.value::numeric) < 2147483648 THEN NULL
                                       WHEN t.value ~ '^-?[0-9]+(\.[0-9]+)?$' THEN t.value::float8 END
                 FROM (SELECT "typeID", "attributeID", value FROM "dgmTypeAttribs" WHERE value ~ '^-?[0-9]+(\.[0-9]+)?$') t
                 JOIN "invTypes" ty ON ty."typeID" = t."typeID"
                 JOIN "invGroups" g ON g."groupID" = ty."groupID"
                WHERE g."categoryID" = $1 AND d."typeID" = t."typeID" AND d."attributeID" = t."attributeID"
                  AND COALESCE(d."valueFloat", d."valueInt"::float8)::numeric
                      IS DISTINCT FROM (CASE WHEN t.value ~ '^-?[0-9]+(\.[0-9]+)?$' THEN t.value::numeric END)"#,
            &[&CATEGORY_SKILL],
        )
        .await?;
    let dropped = tx
        .execute(
            r#"DELETE FROM "dgmTypeAttributes" d
                USING "invTypes" ty, "invGroups" g
                WHERE ty."typeID" = d."typeID" AND g."groupID" = ty."groupID" AND g."categoryID" = $1
                  AND d."attributeID" IN (182, 183, 184, 277, 278, 279)
                  AND EXISTS (SELECT 1 FROM "dgmTypeAttribs" t WHERE t."typeID" = d."typeID")
                  AND NOT EXISTS (SELECT 1 FROM "dgmTypeAttribs" t WHERE t."typeID" = d."typeID" AND t."attributeID" = d."attributeID")"#,
            &[&CATEGORY_SKILL],
        )
        .await?;
    tx.commit().await?;
    if lone + wrapped + skills + dropped > 0 {
        info!("attributes: {} wrapped value(s) fixed, {skills} skill value(s) matched to the client, {dropped} unknown skill requirement(s) dropped", lone + wrapped);
    }
    Ok(())
}
