use eve_marshal::Value;
use tokio_postgres::types::ToSql;
use tracing::{error, info};

use crate::services::{Call, Ctx, Outcome};
use crate::session::CORP_ROLE_ACCOUNTANT;
use crate::svc::rows::{cell, float, int, rowset, text};

const JOURNAL_BATCH: i64 = 25;

const REF_PLAYER_DONATION: i32 = 10;
const REF_CORPORATION_ACCOUNT_WITHDRAWAL: i32 = 37;
pub const REF_BOUNTY: i32 = 16;

const ACCOUNT_KEY_PERSONAL: i32 = 1000;

const REASON_MAX: usize = 40;

pub async fn account(call: &Call, ctx: &Ctx<'_>) -> Option<Outcome> {
    let value = match call.method.as_str() {
        "GiveCash" => return Some(give_cash(call, ctx, false).await),
        "GiveCashFromCorpAccount" => return Some(give_cash(call, ctx, true).await),
        "GetCashBalance" => {
            let corp = call.arg_id(0) != 0;
            cash_balance(ctx, corp).await
        }
        "GetKeyMap" => key_map(ctx).await,
        "GetRefTypes" => ref_types(ctx).await,
        "GetJournal" => journal(call, ctx).await,
        _ => return None,
    };
    Some(Outcome::Value(value))
}

pub async fn bill_mgr(call: &Call, ctx: &Ctx<'_>) -> Option<Outcome> {
    let value = match call.method.as_str() {
        "GetBillTypes" => bill_types(ctx).await,
        "CharGetBills" => bills(ctx, ctx.session.char_id, Side::Payable).await,
        "CharGetBillsReceivable" => bills(ctx, ctx.session.char_id, Side::Receivable).await,
        "GetCorporationBills" => corporation_bills(ctx).await,
        "GetCorporationBillsReceivable" => bills(ctx, ctx.session.corp_id, Side::Receivable).await,
        "CharPayBill" => return Some(pay_bill(ctx, call.arg_id(0), false).await),
        "PayCorporationBill" => return Some(pay_bill(ctx, call.arg_id(0), true).await),
        _ => return None,
    };
    Some(Outcome::Value(value))
}

const BILL_TYPE_RENTAL: i64 = 2;
const BILL_TYPE_WAR: i64 = 4;
const BILL_TYPE_ALLIANCE_MAINTENANCE: i64 = 5;
/// market_refTypes: War Fee, Alliance Maintainance Fee.
const REF_WAR_FEE: i32 = 49;
const REF_ALLIANCE_MAINTENANCE_FEE: i32 = 50;
pub const REF_OFFICE_RENTAL_FEE: i32 = 13;
const REF_CORPORATION_PAYMENT: i32 = 11;

/// Pays one of the caller's (or, with the Accountant role, its corporation's)
/// open bills to the creditor (wallet.py PayBill).
async fn pay_bill(ctx: &Ctx<'_>, bill_id: i64, corp: bool) -> Outcome {
    let debtor = if corp {
        if !ctx.session.has_corp_role(CORP_ROLE_ACCOUNTANT) {
            return Outcome::UserError {
                msg: "CrpAccessDenied".into(),
                args: vec![("reason".into(), Value::bytes("you do not hold the Accountant role"))],
            };
        }
        ctx.session.corp_id
    } else {
        ctx.session.char_id
    };
    let Some(debtor) = debtor else { return Outcome::Value(Value::None) };
    // The executor corporation pays its alliance's bills from its own wallet.
    if corp {
        if let Some(alliance) = executor_alliance(ctx).await {
            let sql = r#"SELECT 1 FROM "billsReceivable" WHERE "billID" = $1 AND "debtorID" = $2 AND "paid" = 0"#;
            if ctx.query_one(sql, &[&(bill_id as i32), &(alliance as i32)]).await.is_some() {
                return settle_bill(ctx, bill_id, alliance, debtor).await;
            }
        }
    }
    settle_bill(ctx, bill_id, debtor, debtor).await
}

/// The caller's alliance, when its corporation is that alliance's executor.
async fn executor_alliance(ctx: &Ctx<'_>) -> Option<i64> {
    let (corp, alliance) = (ctx.session.corp_id?, ctx.session.alliance_id.filter(|a| *a != 0)?);
    let sql = r#"SELECT 1 FROM alliances WHERE "allianceID" = $1 AND "executorCorpID" = $2 AND "deleted" = 0"#;
    ctx.query_one(sql, &[&(alliance as i32), &(corp as i32)]).await.map(|_| alliance)
}

/// The corporation wallet's bills: its own, and, for an alliance's executor
/// corporation, the alliance's (maintenance and war bills), which it pays.
async fn corporation_bills(ctx: &Ctx<'_>) -> Value {
    let mut lines = match ctx.session.corp_id {
        Some(corp) => bill_lines(ctx, corp, Side::Payable).await,
        None => Vec::new(),
    };
    if let Some(alliance) = executor_alliance(ctx).await {
        lines.extend(bill_lines(ctx, alliance, Side::Payable).await);
    }
    rowset(BILL_HEADER, lines)
}

/// Office rent falls due a month at a time: five days before an office's
/// paid-up date its corporation gets a Rental Bill from the station's owner.
/// Paying it extends the office another period. Run by the heartbeat.
pub async fn bill_offices(db: &tokio_postgres::Client) {
    const DAY: i64 = 24 * 3600 * 10_000_000;
    let now = crate::bulkdata::filetime_now();
    let sql = r#"SELECT r."stationID", r."slotNumber", r."renterID", r."periodCost", r."balanceDueDate",
                        COALESCE(s."corporationID", 0), COALESCE(s."officeRentalCost", 0)::double precision
                   FROM "rentalInfo" r
                   LEFT JOIN "staStations" s ON s."stationID" = r."stationID"
                  WHERE r."discontinue" = 0 AND r."balanceDueDate" > 0 AND r."balanceDueDate" - $1 < $2
                    AND NOT EXISTS (SELECT 1 FROM "billsReceivable" b WHERE b."billID" = r."billID" AND b."paid" = 0 AND r."billID" <> 0)
                    AND NOT EXISTS (SELECT 1 FROM "botGroups" g WHERE g."ownerID" = r."renterID")
                    AND EXISTS (SELECT 1 FROM "corporation" c WHERE c."corporationID" = r."renterID" AND c."deleted" = 0)"#;
    let rows = match db.query(sql, &[&now, &(5 * DAY)]).await {
        Ok(rows) => rows,
        Err(e) => {
            error!("reading office rentals to bill: {}", crate::logging::chain(&e));
            return;
        }
    };
    for r in rows {
        let (station, slot, renter) = (int(&r, 0).unwrap_or(0), int(&r, 1).unwrap_or(0), int(&r, 2).unwrap_or(0));
        let due = int(&r, 4).unwrap_or(now);
        let owner = int(&r, 5).unwrap_or(0);
        let amount = float(&r, 3).filter(|c| *c > 0.0).or_else(|| float(&r, 6)).unwrap_or(0.0);
        if owner == 0 || amount <= 0.0 {
            continue;
        }
        let insert = r#"INSERT INTO "billsReceivable" ("billTypeID","debtorID","creditorID","amount","dueDateTime","externalID","externalID2")
                        VALUES ($1,$2,$3,$4,$5,$6,$7) RETURNING "billID""#;
        let params: [&(dyn ToSql + Sync); 7] = [&(BILL_TYPE_RENTAL as i32), &(renter as i32), &(owner as i32), &amount, &due, &(station as i32), &slot.to_string()];
        match db.query_one(insert, &params).await {
            Ok(b) => {
                let bill = int(&b, 0).unwrap_or(0) as i32;
                let link = r#"UPDATE "rentalInfo" SET "billID" = $1 WHERE "stationID" = $2 AND "slotNumber" = $3 AND "renterID" = $4"#;
                if let Err(e) = db.execute(link, &[&bill, &(station as i32), &(slot as i32), &(renter as i32)]).await {
                    error!("linking rental bill {bill}: {}", crate::logging::chain(&e));
                }
                info!("corporation {renter} billed {amount} ISK office rent at station {station}");
            }
            Err(e) => error!("billing office rent to {renter}: {}", crate::logging::chain(&e)),
        }
    }
}

/// Pays `debtor`'s open bill `bill_id` to its creditor out of `payer`'s
/// wallet (the debtor's own, or an alliance's executor corporation's).
pub async fn settle_bill(ctx: &Ctx<'_>, bill_id: i64, debtor: i64, payer: i64) -> Outcome {
    // Claim the bill first so it cannot be paid twice; released below on failure.
    let claim = r#"UPDATE "billsReceivable" SET "paid" = 1
                    WHERE "billID" = $1 AND "debtorID" = $2 AND "paid" = 0
                RETURNING "creditorID", "amount", "billTypeID""#;
    let Some(bill) = ctx.query_one(claim, &[&(bill_id as i32), &(debtor as i32)]).await else {
        return Outcome::Value(Value::None);
    };
    let creditor = int(&bill, 0).unwrap_or(0);
    let amount = float(&bill, 1).unwrap_or(0.0);
    let ref_type = match int(&bill, 2) {
        Some(BILL_TYPE_RENTAL) => REF_OFFICE_RENTAL_FEE,
        Some(BILL_TYPE_WAR) => REF_WAR_FEE,
        Some(BILL_TYPE_ALLIANCE_MAINTENANCE) => REF_ALLIANCE_MAINTENANCE_FEE,
        _ => REF_CORPORATION_PAYMENT,
    };
    let bill_ref = bill_id.to_string();
    let payment = Payment {
        payer,
        payee: Some(creditor),
        amount,
        ref_type,
        arg_id1: &bill_ref,
        reason: "",
    };
    let outcome = match move_cash(ctx.db_url, payment).await {
        Ok(()) => {
            info!("bill {bill_id}: {payer} paid {amount} ISK of {debtor}'s to {creditor}");
            if int(&bill, 2) == Some(BILL_TYPE_RENTAL) {
                // Paid rent: the office is kept another period.
                let extend = r#"UPDATE "rentalInfo"
                                   SET "balanceDueDate" = "balanceDueDate" + GREATEST("rentPeriodInDays", 1)::bigint * 864000000000, "billID" = 0
                                 WHERE "billID" = $1"#;
                ctx.execute(extend, &[&(bill_id as i32)]).await;
            }
            return Outcome::Value(Value::None);
        }
        Err(Transfer::NotEnough { need, have }) => Outcome::not_enough_money(need, have),
        Err(Transfer::NoSuchOwner) => Outcome::Unknown(format!("billMgr: bill {bill_id} has no valid creditor")),
        Err(Transfer::Db(e)) => {
            error!("bill {bill_id} payment: {}", crate::logging::chain(&e));
            Outcome::Unknown("billMgr payment failed".into())
        }
    };
    let release = r#"UPDATE "billsReceivable" SET "paid" = 0 WHERE "billID" = $1"#;
    ctx.execute(release, &[&(bill_id as i32)]).await;
    outcome
}

/// The wallet-transaction part of the `marketProxy` service.
pub async fn market_proxy(call: &Call, ctx: &Ctx<'_>) -> Option<Outcome> {
    let value = match call.method.as_str() {
        "CharGetNewTransactions" => transactions(ctx, None).await,
        "CharGetTransactions" => {
            transactions(ctx, call.arg_i64(0)).await
        }
        _ => return None,
    };
    Some(Outcome::Value(value))
}

async fn cash_balance(ctx: &Ctx<'_>, corp: bool) -> Value {
    let (sql, owner) = if corp {
        (
            r#"SELECT "balance" FROM "corporation" WHERE "corporationID" = $1"#,
            ctx.session.corp_id,
        )
    } else {
        (
            r#"SELECT "balance" FROM "character_" WHERE "characterID" = $1"#,
            ctx.session.char_id,
        )
    };
    let Some(owner) = owner else { return Value::Double(0.0) };
    match ctx.db.query(sql, &[&(owner as i32)]).await {
        Ok(rows) if !rows.is_empty() => {
            Value::Double(float(&rows[0], 0).unwrap_or(0.0))
        }
        Ok(_) => Value::Double(0.0),
        Err(e) => {
            error!("GetCashBalance: {err}", err = crate::logging::chain(&e));
            Value::Double(0.0)
        }
    }
}

async fn key_map(ctx: &Ctx<'_>) -> Value {
    const HEADER: &[&str] = &["accountKey", "accountType", "accountName", "description"];
    let sql = r#"SELECT "accountKey","accountType","accountName","description"
                   FROM "market_keyMap" ORDER BY "accountKey""#;
    match ctx.db.query(sql, &[]).await {
        Ok(rows) => {
            let lines = rows
                .iter()
                .map(|r| {
                    Value::List(vec![
                        cell(r, 0),
                        cell(r, 1),
                        cell(r, 2),
                        cell(r, 3),
                    ])
                })
                .collect();
            rowset(HEADER, lines)
        }
        Err(e) => {
            error!("GetKeyMap: {err}", err = crate::logging::chain(&e));
            rowset(HEADER, Vec::new())
        }
    }
}

async fn ref_types(ctx: &Ctx<'_>) -> Value {
    const HEADER: &[&str] = &["refTypeID", "refTypeText", "description"];
    let sql = r#"SELECT "refTypeID","refTypeText","description"
                   FROM "market_refTypes" ORDER BY "refTypeID""#;
    match ctx.db.query(sql, &[]).await {
        Ok(rows) => {
            let lines = rows
                .iter()
                .map(|r| {
                    Value::List(vec![
                        cell(r, 0),
                        cell(r, 1),
                        cell(r, 2),
                    ])
                })
                .collect();
            rowset(HEADER, lines)
        }
        Err(e) => {
            error!("GetRefTypes: {err}", err = crate::logging::chain(&e));
            rowset(HEADER, Vec::new())
        }
    }
}

async fn journal(call: &Call, ctx: &Ctx<'_>) -> Value {
    const HEADER: &[&str] = &[
        "refID", "transDate", "refTypeID", "ownerID1", "ownerID2", "argID1", "amount", "balance",
        "reason", "accountKey",
    ];

    let account_key = call.arg_i64(0).unwrap_or(1000) as i32;
    let from_date = call.arg_i64(1);
    let ref_type_id = call.arg_i64(2).map(|v| v as i32);
    let corp = call.arg_id(3) != 0;
    let from_ref_id = call.arg_i64(4).map(|v| v as i32);
    let reverse = call.arg_id(5) != 0;

    let owner = if corp { ctx.session.corp_id } else { ctx.session.char_id };
    let Some(owner) = owner else { return rowset(HEADER, Vec::new()) };
    let owner = owner as i32;

    let mut sql = String::from(
        r#"SELECT "refID","transDate","refTypeID","ownerID1","ownerID2","argID1",
                  "amount","balance","reason","accountKey"
             FROM "market_journal"
            WHERE "characterID" = $1 AND "accountKey" = $2"#,
    );
    let mut params: Vec<&(dyn ToSql + Sync)> = vec![&owner, &account_key];
    if let Some(d) = from_date.as_ref() {
        sql.push_str(&format!(r#" AND "transDate" < ${}"#, params.len() + 1));
        params.push(d);
    }
    if let Some(t) = ref_type_id.as_ref() {
        sql.push_str(&format!(r#" AND "refTypeID" = ${}"#, params.len() + 1));
        params.push(t);
    }
    if let Some(r) = from_ref_id.as_ref() {
        sql.push_str(&format!(r#" AND "refID" < ${}"#, params.len() + 1));
        params.push(r);
    }
    sql.push_str(&format!(r#" ORDER BY "refID" DESC LIMIT {}"#, JOURNAL_BATCH));

    match ctx.db.query(sql.as_str(), &params[..]).await {
        Ok(rows) => {
            let mut lines: Vec<Value> = rows
                .iter()
                .map(|r| {
                    Value::List(vec![
                        cell(r, 0),
                        cell(r, 1),
                        cell(r, 2),
                        cell(r, 3),
                        cell(r, 4),
                        text_id(text(r, 5)),
                        Value::Double(float(r, 6).unwrap_or(0.0)),
                        Value::Double(float(r, 7).unwrap_or(0.0)),
                        cell(r, 8),
                        cell(r, 9),
                    ])
                })
                .collect();
            if !reverse {
                lines.reverse();
            }
            rowset(HEADER, lines)
        }
        Err(e) => {
            error!("GetJournal: {err}", err = crate::logging::chain(&e));
            rowset(HEADER, Vec::new())
        }
    }
}

async fn bill_types(ctx: &Ctx<'_>) -> Value {
    const HEADER: &[&str] = &["billTypeID", "billTypeName", "description"];
    let sql = r#"SELECT "billTypeID","billTypeName","description"
                   FROM "billTypes" ORDER BY "billTypeID""#;
    match ctx.db.query(sql, &[]).await {
        Ok(rows) => {
            let lines = rows
                .iter()
                .map(|r| {
                    Value::List(vec![
                        cell(r, 0),
                        cell(r, 1),
                        cell(r, 2),
                    ])
                })
                .collect();
            rowset(HEADER, lines)
        }
        Err(e) => {
            error!("GetBillTypes: {err}", err = crate::logging::chain(&e));
            rowset(HEADER, Vec::new())
        }
    }
}

#[derive(Clone, Copy)]
pub enum Side {
    Payable,
    Receivable,
}

/// The open bills `owner` owes (Payable) or is owed (Receivable).
pub async fn bills(ctx: &Ctx<'_>, owner: Option<i64>, side: Side) -> Value {
    match owner {
        Some(owner) => rowset(BILL_HEADER, bill_lines(ctx, owner, side).await),
        None => rowset(BILL_HEADER, Vec::new()),
    }
}

const BILL_HEADER: &[&str] = &[
    "billID", "billTypeID", "debtorID", "creditorID", "amount", "dueDateTime", "interest",
    "externalID", "paid", "externalID2",
];

/// The open bills `owner` owes (payable) or is owed (receivable).
async fn bill_lines(ctx: &Ctx<'_>, owner: i64, side: Side) -> Vec<Value> {

    let sql = match side {
        Side::Payable => {
            r#"SELECT "billID","billTypeID","debtorID","creditorID","amount","dueDateTime",
                      "interest","externalID","paid","externalID2"
                 FROM "billsReceivable"
                WHERE "debtorID" = $1 AND "creditorID" IS NOT NULL
                      AND "billTypeID" IS NOT NULL
                ORDER BY "billID""#
        }
        Side::Receivable => {
            r#"SELECT "billID","billTypeID","debtorID","creditorID","amount","dueDateTime",
                      "interest","externalID","paid","externalID2"
                 FROM "billsReceivable"
                WHERE "creditorID" = $1 AND "debtorID" IS NOT NULL
                      AND "billTypeID" IS NOT NULL
                ORDER BY "billID""#
        }
    };

    match ctx.db.query(sql, &[&(owner as i32)]).await {
        Ok(rows) => rows
                .iter()
                .filter(|r| int(r, 8).unwrap_or(0) == 0)
                .map(|r| {
                    Value::List(vec![
                        cell(r, 0),
                        cell(r, 1),
                        cell(r, 2),
                        cell(r, 3),
                        Value::Double(float(r, 4).unwrap_or(0.0)),
                        int(r, 5).map_or(Value::None, Value::Long),
                        Value::Double(float(r, 6).unwrap_or(0.0)),
                        Value::Int(int(r, 7).unwrap_or(-1)),
                        Value::Int(0),
                        Value::Int(text(r, 9).and_then(|s| parse_i64(&s)).unwrap_or(-1)),
                    ])
                })
                .collect(),
        Err(e) => {
            error!("bills: {err}", err = crate::logging::chain(&e));
            Vec::new()
        }
    }
}

async fn transactions(ctx: &Ctx<'_>, before_id: Option<i64>) -> Value {
    const HEADER: &[&str] = &[
        "transactionID", "transactionDateTime", "typeID", "quantity", "price", "transactionType",
        "clientID", "regionID", "stationID",
    ];
    const PAGE: i64 = 25;
    let Some(char_id) = ctx.session.char_id else { return rowset(HEADER, Vec::new()) };

    let sql = r#"SELECT "transactionID","transactionDateTime","typeID","quantity","price",
                        "transactionType","clientID","regionID","stationID"
                   FROM market_transactions
                  WHERE "characterID" = $1
                    AND ($2::int IS NULL OR "transactionID" < $2)
                  ORDER BY "transactionID" DESC
                  LIMIT $3"#;
    let before = before_id.and_then(|v| i32::try_from(v).ok());
    match ctx.db.query(sql, &[&(char_id as i32), &before, &PAGE]).await {
        Ok(rows) => rowset(HEADER, crate::svc::rows::lines(&rows)),
        Err(e) => {
            error!("CharGetTransactions({char_id}): {err}", err = crate::logging::chain(&e));
            rowset(HEADER, Vec::new())
        }
    }
}

async fn give_cash(call: &Call, ctx: &Ctx<'_>, from_corp: bool) -> Outcome {
    let to_owner = call.arg_id(0);
    let amount = match call.args.get(1) {
        Some(Value::Double(d)) => *d,
        Some(v) => v.as_i64().map(|i| i as f64).unwrap_or(0.0),
        None => 0.0,
    };
    let reason: String = call
        .args
        .get(2)
        .and_then(|v| v.as_bytes())
        .map(|b| String::from_utf8_lossy(b).into_owned())
        .unwrap_or_default()
        .chars()
        .take(REASON_MAX)
        .collect();

    let Some(char_id) = ctx.session.char_id else {
        return Outcome::Unknown("account::GiveCash without a character".into());
    };
    if !amount.is_finite() || amount <= 0.0 {
        return Outcome::Unknown(format!("account::GiveCash with amount {amount}"));
    }
    let payer = if from_corp {
        let Some(corp) = ctx.session.corp_id else {
            return Outcome::Unknown("account::GiveCashFromCorpAccount without a corporation".into());
        };
        if !ctx.session.has_corp_role(CORP_ROLE_ACCOUNTANT) {
            return Outcome::UserError {
                msg: "CrpAccessDenied".into(),
                args: vec![(
                    "reason".into(),
                    Value::bytes("you do not hold the Accountant role"),
                )],
            };
        }
        corp
    } else {
        char_id
    };

    let (ref_type, arg_id1) = if from_corp {
        (REF_CORPORATION_ACCOUNT_WITHDRAWAL, char_id.to_string())
    } else {
        (REF_PLAYER_DONATION, String::new())
    };
    let payment = Payment { payer, payee: Some(to_owner), amount, ref_type, arg_id1: &arg_id1, reason: &reason };
    match move_cash(ctx.db_url, payment).await {
        Ok(()) => {
            info!(
                "GiveCash{}: {payer} -> {to_owner}, {amount} ISK ({reason:?})",
                if from_corp { "FromCorpAccount" } else { "" }
            );
            Outcome::Value(Value::None)
        }
        Err(Transfer::NotEnough { need, have }) => Outcome::not_enough_money(need, have),
        Err(Transfer::NoSuchOwner) => {
            Outcome::Unknown(format!("account::GiveCash to unknown owner {to_owner}"))
        }
        Err(Transfer::Db(e)) => {
            error!("GiveCash {payer} -> {to_owner}: {err}", err = crate::logging::chain(&e));
            Outcome::Unknown("account::GiveCash failed".into())
        }
    }
}


pub enum Transfer {
    NotEnough { need: f64, have: f64 },
    NoSuchOwner,
    Db(tokio_postgres::Error),
}

/// One movement of ISK, journalled for both sides. `payee` None means the money
/// leaves the economy (a bounty, a fee).
pub struct Payment<'a> {
    pub payer: i64,
    pub payee: Option<i64>,
    pub amount: f64,
    pub ref_type: i32,
    pub arg_id1: &'a str,
    pub reason: &'a str,
}

pub async fn move_cash(db_url: &str, p: Payment<'_>) -> Result<(), Transfer> {
    let mut client = crate::db::connect(db_url).await.map_err(Transfer::Db)?;
    let tx = client.transaction().await.map_err(Transfer::Db)?;
    let entry = Entry { ref_type: p.ref_type, payer: p.payer, payee: p.payee.unwrap_or(0), arg_id1: p.arg_id1, reason: p.reason };
    // Post in ID order so two opposite transfers cannot deadlock.
    let mut sides: Vec<(i64, f64)> = std::iter::once((p.payer, -p.amount)).chain(p.payee.map(|id| (id, p.amount))).collect();
    sides.sort_unstable_by_key(|(id, _)| *id);
    for (owner, delta) in sides {
        post(&tx, owner, delta, &entry).await?;
    }
    tx.commit().await.map_err(Transfer::Db)
}

/// What a journal line records about the payment it belongs to.
pub struct Entry<'a> {
    pub ref_type: i32,
    pub payer: i64,
    /// 0 when the money leaves the economy or comes from nowhere.
    pub payee: i64,
    pub arg_id1: &'a str,
    pub reason: &'a str,
}

/// One side of a payment, inside the caller's transaction: add `delta` to
/// `owner`'s wallet (a character or a corporation) and journal it. Refuses to
/// go below zero. Returns the new balance.
pub async fn post(
    tx: &tokio_postgres::Transaction<'_>,
    owner: i64,
    delta: f64,
    e: &Entry<'_>,
) -> Result<f64, Transfer> {
    let Some(have) = lock_balance(tx, owner).await? else { return Err(Transfer::NoSuchOwner) };
    if delta < 0.0 && have + delta < 0.0 {
        return Err(Transfer::NotEnough { need: -delta, have });
    }
    add_balance(tx, owner, delta).await?;
    let balance = have + delta;
    tx.execute(
        r#"INSERT INTO market_journal
               ("characterID","transDate","refTypeID","ownerID1","ownerID2","argID1",
                "accountID","amount","balance","reason","accountKey")
           VALUES ($1,$2,$3,$4,$5,$6,0,$7,$8,$9,$10)"#,
        &[
            &(owner as i32),
            &crate::bulkdata::filetime_now(),
            &e.ref_type,
            &(e.payer as i32),
            &(e.payee as i32),
            &e.arg_id1,
            &delta,
            &balance,
            &e.reason,
            &ACCOUNT_KEY_PERSONAL,
        ],
    )
    .await
    .map_err(Transfer::Db)?;
    Ok(balance)
}

async fn lock_balance(
    tx: &tokio_postgres::Transaction<'_>,
    owner: i64,
) -> Result<Option<f64>, Transfer> {
    let id = owner as i32;
    if let Some(r) = tx
        .query_opt(r#"SELECT balance FROM "corporation" WHERE "corporationID" = $1 FOR UPDATE"#, &[&id])
        .await
        .map_err(Transfer::Db)?
    {
        return Ok(Some(r.get(0)));
    }
    Ok(tx
        .query_opt(r#"SELECT balance FROM "character_" WHERE "characterID" = $1 FOR UPDATE"#, &[&id])
        .await
        .map_err(Transfer::Db)?
        .map(|r| r.get(0)))
}

async fn add_balance(
    tx: &tokio_postgres::Transaction<'_>,
    owner: i64,
    delta: f64,
) -> Result<(), Transfer> {
    let id = owner as i32;
    let n = tx
        .execute(
            r#"UPDATE "corporation" SET balance = balance + $2 WHERE "corporationID" = $1"#,
            &[&id, &delta],
        )
        .await
        .map_err(Transfer::Db)?;
    if n == 0 {
        tx.execute(
            r#"UPDATE "character_" SET balance = balance + $2 WHERE "characterID" = $1"#,
            &[&id, &delta],
        )
        .await
        .map_err(Transfer::Db)?;
    }
    Ok(())
}




fn text_id(v: Option<String>) -> Value {
    match v.as_deref().and_then(parse_i64) {
        Some(i) => Value::Int(i),
        None => Value::None,
    }
}


fn parse_i64(s: &str) -> Option<i64> {
    let s = s.trim();
    match s.parse::<i64>() {
        Ok(v) => Some(v),
        Err(_) => s.parse::<f64>().ok().map(|v| v as i64),
    }
}

