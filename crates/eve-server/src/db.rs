use tokio_postgres::{Client, NoTls};
use tracing::{error, info};

#[derive(Debug, Clone)]
pub struct Account {
    pub account_id: i64,
    pub name: String,
    pub role: i64,
}

pub async fn connect(url: &str) -> Result<Client, tokio_postgres::Error> {
    let (client, connection) = tokio_postgres::connect(url, NoTls).await?;
    tokio::spawn(async move {
        if let Err(e) = connection.await {
            error!("database connection lost: {err}", err = crate::logging::chain(&e));
        }
    });
    Ok(client)
}

pub async fn authenticate(
    db: &Client,
    username: &str,
    password: &str,
) -> Result<Option<Account>, tokio_postgres::Error> {
    let rows = db
        .query(
            r#"SELECT "accountID", "accountName", "password", "role", "banned", "passwordAlgo"
                 FROM "account" WHERE "accountName" = $1"#,
            &[&username],
        )
        .await?;
    let Some(row) = rows.first() else {
        return if crate::config::auto_create_accounts() && !username.trim().is_empty() && !password.is_empty() {
            create_account(db, username, password).await.map(Some)
        } else {
            Ok(None)
        };
    };
    if row.get::<_, i16>(4) != 0 {
        info!("{username:?} is banned");
        return Ok(None);
    }

    // Passwords are kept only as salted PBKDF2 hashes (crate::secret). An
    // account still holding an old plain one is checked once that way and
    // given a hash on the spot.
    let stored: String = row.get(2);
    let algo: String = row.get(5);
    let account_id: i32 = row.get(0);
    if stored.is_empty() {
        return Ok(None);
    }
    let (given, kept) = (password.to_string(), stored.clone());
    let hashed = algo == crate::secret::ALGO && crate::secret::is_hash(&stored);
    let ok = tokio::task::spawn_blocking(move || {
        if hashed { crate::secret::verify_password(&given, &kept) } else { crate::secret::same(given.as_bytes(), kept.as_bytes()) }
    })
    .await
    .unwrap_or(false);
    if !ok {
        return Ok(None);
    }
    if !hashed {
        let fresh = hash_blocking(password).await;
        db.execute(r#"UPDATE "account" SET "password" = $1, "passwordAlgo" = $2 WHERE "accountID" = $3"#,
            &[&fresh, &crate::secret::ALGO, &account_id]).await?;
        info!("account {username:?}: password now stored hashed");
    }
    Ok(Some(Account {
        account_id: account_id as i64,
        name: row.get(1),
        // bigint: roles above 2^31 exist (ROLE_TRANSLATIONEDITOR = 2^32).
        role: row.get(3),
    }))
}

/// A player account made at its first login (`autoCreateAccounts`).
async fn create_account(db: &Client, username: &str, password: &str) -> Result<Account, tokio_postgres::Error> {
    let hashed = hash_blocking(password).await;
    let row = db
        .query_one(
            r#"INSERT INTO "account" ("accountName", "password", "passwordAlgo", "role", "createdAt") VALUES ($1, $2, $3, 0, $4) RETURNING "accountID""#,
            &[&username, &hashed, &crate::secret::ALGO, &crate::bulkdata::filetime_now()],
        )
        .await?;
    info!("account {username:?} made at its first login");
    Ok(Account { account_id: row.get::<_, i32>(0) as i64, name: username.to_string(), role: 0 })
}

/// Makes the config file's accounts, or refreshes their passwords and bans.
/// Existing roles stay in the database so dashboard changes survive restart.
/// Returns their accountIDs, in the file's order.
/// Also returns, for each account whose password the file gave in the clear,
/// its name and the hash now stored, for `config::seal_passwords`.
pub async fn sync_accounts(db: &Client, accounts: &[crate::config::Account]) -> Result<(Vec<i64>, Vec<(String, String)>), tokio_postgres::Error> {
    let now = crate::bulkdata::filetime_now();
    // Accounts once made with IDs of their own leave the sequence behind.
    let sql = r#"SELECT setval('"account_accountID_seq"', GREATEST((SELECT max("accountID") FROM "account"), 1))"#;
    db.execute(sql, &[]).await?;
    let mut ids = Vec::with_capacity(accounts.len());
    let mut sealed = Vec::new();
    for a in accounts {
        let banned = i16::from(a.banned);
        // The file gives a password (hashed here) or its hash (kept as is).
        let hash = match (&a.password_hash, a.password.is_empty()) {
            (Some(h), true) => h.clone(),
            _ => hash_blocking(&a.password).await,
        };
        let row = db
            .query_one(
                r#"INSERT INTO "account" ("accountName", "password", "passwordAlgo", "role", "banned", "createdAt") VALUES ($1, $2, $3, $4, $5, $6)
                   ON CONFLICT ("accountName") DO UPDATE SET "password" = $2, "passwordAlgo" = $3, "banned" = $5
                   RETURNING "accountID""#,
                &[&a.username, &hash, &crate::secret::ALGO, &a.role(), &banned, &now],
            )
            .await?;
        ids.push(row.get::<_, i32>(0) as i64);
        if !a.password.is_empty() {
            sealed.push((a.username.clone(), hash));
        }
    }
    Ok((ids, sealed))
}

/// A password's salted hash, worked out off the async threads (it takes a
/// moment on purpose).
pub async fn hash_blocking(password: &str) -> String {
    let password = password.to_string();
    tokio::task::spawn_blocking(move || crate::secret::hash_password(&password)).await.unwrap_or_default()
}

/// At start: every account still holding a plain password gets it hashed,
/// so no password is kept in the clear in the database.
pub async fn hash_stored_passwords(db: &Client) -> Result<usize, tokio_postgres::Error> {
    let rows = db
        .query(r#"SELECT "accountID", "password" FROM "account" WHERE "passwordAlgo" <> $1 AND "password" <> ''"#, &[&crate::secret::ALGO])
        .await?;
    for r in &rows {
        let id: i32 = r.get(0);
        let plain: String = r.get(1);
        let hash = hash_blocking(&plain).await;
        db.execute(r#"UPDATE "account" SET "password" = $1, "passwordAlgo" = $2 WHERE "accountID" = $3"#, &[&hash, &crate::secret::ALGO, &id]).await?;
    }
    Ok(rows.len())
}
