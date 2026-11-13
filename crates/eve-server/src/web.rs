//! The web dashboard: a page, built in, and the JSON it reads, showing
//! who is playing, the wars under way, the alliances and the space they
//! hold, the busiest systems, the news from null-sec, the latest kills and
//! the killboard, the bots' market, their feelings about players, the
//! pilots being followed and politics' upheavals. `RMR_WEB` is where it
//! listens (empty: off).
//!
//! It closes each connection after its answer. Bot controls and account role
//! edits use POST; account management is available only from loopback.
//!
//! The page is TypeScript (web/src/app.ts), compiled to web/dist/app.js with
//! `npm run build` in crates/eve-server/web; the compiled file is part of the
//! source, so building the server needs no Node.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use eve_marshal::Value;
use serde_json::{json, Map, Value as Json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_postgres::types::Type;
use tokio_postgres::Row;
use tracing::{debug, error, info, warn};

use crate::conn::Server;

const INDEX: &str = include_str!("../web/index.html");
const STYLE: &str = include_str!("../web/style.css");
const SCRIPT: &str = include_str!("../web/dist/app.js");
const EVE_ICON: &[u8] = include_bytes!("../web/eve-icon.png");

/// The most a request may be before it is turned away.
const MAX_REQUEST: usize = 8 * 1024;
const MAX_BODY: usize = 64 * 1024;
/// FILETIME of 1970-01-01, in milliseconds since 1601.
const UNIX_EPOCH_MS: i64 = 11_644_473_600_000;

static STARTED: OnceLock<Instant> = OnceLock::new();

pub async fn ensure_storage(db: &tokio_postgres::Client) -> Result<(), tokio_postgres::Error> {
    db.batch_execute(r#"
        CREATE TABLE IF NOT EXISTS "rmrNewsArticles" (
            id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
            title text NOT NULL,
            body text NOT NULL,
            published_ms bigint NOT NULL
        );
        CREATE TABLE IF NOT EXISTS "rmrDowntime" (
            id integer PRIMARY KEY CHECK (id = 1),
            deadline_ms bigint NOT NULL,
            message text NOT NULL,
            active boolean NOT NULL DEFAULT false
        );
        ALTER TABLE "rmrDowntime" ADD COLUMN IF NOT EXISTS restart boolean NOT NULL DEFAULT false;
    "#).await
}

fn unix_ms_now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as i64
}

/// Called by the main accept loop so bot and ship state follow the ordinary
/// graceful shutdown path before the launcher closes the database.
/// Some(true) when the launcher should start the server again afterwards.
pub async fn take_due_downtime(db: &tokio_postgres::Client) -> Result<Option<bool>, tokio_postgres::Error> {
    Ok(db.query_opt(r#"UPDATE "rmrDowntime" SET active = false
        WHERE id = 1 AND active AND deadline_ms <= $1 RETURNING restart"#, &[&unix_ms_now()]).await?
        .map(|row| row.get::<_, bool>(0)))
}

/// A restart needs the launcher, which starts the server again; it sets this.
fn restart_available() -> bool {
    std::env::var_os("RMR_SHUTDOWN_FILE").is_some()
}

pub async fn notify_joined(server: &Server, client_id: i64) {
    if let Ok(Some(row)) = server.db.query_opt(r#"SELECT deadline_ms, message FROM "rmrDowntime"
        WHERE id = 1 AND active AND deadline_ms > $1"#, &[&unix_ms_now()]).await {
        server.world.lock().notify(&[client_id], "OnClusterShutdownInitiated", Value::Tuple(vec![
            Value::text(row.get::<_, String>(1)), Value::Long(filetime_at_ms(row.get(0))),
        ]));
    }
}

fn filetime_at_ms(ms: i64) -> i64 {
    (ms + UNIX_EPOCH_MS) * 10_000
}

pub async fn run(server: Arc<Server>, bind: String) {
    if bind.is_empty() {
        return;
    }
    STARTED.get_or_init(Instant::now);
    let listener = match TcpListener::bind(&bind).await {
        Ok(l) => l,
        Err(e) => {
            error!("web dashboard: cannot listen on {bind}: {e}");
            return;
        }
    };
    info!("web dashboard on http://{bind}/");
    loop {
        let Ok((stream, _)) = listener.accept().await else { continue };
        let server = Arc::clone(&server);
        tokio::spawn(async move {
            if let Err(e) = answer(stream, &server).await {
                debug!("web: {e}");
            }
        });
    }
}

async fn answer(mut stream: TcpStream, server: &Server) -> std::io::Result<()> {
    let local = stream.peer_addr()?.ip().is_loopback();
    let mut buf = Vec::with_capacity(1024);
    let mut chunk = [0u8; 1024];
    while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
        let n = stream.read(&mut chunk).await?;
        if n == 0 {
            return Ok(());
        }
        buf.extend_from_slice(&chunk[..n]);
        if buf.len() > MAX_REQUEST {
            return respond(&mut stream, 431, "text/plain", b"Request too large").await;
        }
    }
    let head_end = buf.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
    let head = String::from_utf8_lossy(&buf[..head_end]).into_owned();
    let mut parts = head.lines().next().unwrap_or("").split_whitespace();
    let (method, target) = (parts.next().unwrap_or(""), parts.next().unwrap_or("/"));
    let path = target.split('?').next().unwrap_or("/");
    let body = if method == "POST" && matches!(path, "/api/articles" | "/api/articles/preview" | "/api/downtime" | "/api/seed" | "/api/bots" | "/api/dungeon-spawn" | "/api/dungeons-reload") {
        let Some(len) = header(&head, "Content-Length").and_then(|n| n.parse::<usize>().ok()) else {
            return respond(&mut stream, 400, "application/json", br#"{"error":"Content-Length is required."}"#).await;
        };
        if len > MAX_BODY { return respond(&mut stream, 413, "text/plain", b"Article is too long").await; }
        while buf.len() - head_end < len {
            let n = stream.read(&mut chunk).await?;
            if n == 0 { return respond(&mut stream, 400, "text/plain", b"Incomplete request").await; }
            buf.extend_from_slice(&chunk[..n]);
        }
        buf[head_end..head_end + len].to_vec()
    } else { Vec::new() };
    if matches!(path, "/api/articles" | "/api/articles/preview" | "/api/downtime" | "/api/seed" | "/api/bots" | "/api/dungeon-spawn" | "/api/dungeons-reload") && method == "POST" {
        let host = header(&head, "Host").unwrap_or("");
        if !local || !local_host(host, stream.local_addr()?.port()) ||
            header(&head, "X-RMR-Admin") != Some("1") ||
            header(&head, "Origin") != Some(format!("http://{host}").as_str()) {
            return respond(&mut stream, 403, "application/json", br#"{"error":"This control is available only on the server dashboard."}"#).await;
        }
        let result = match path {
            "/api/articles" => publish_article(server, &body).await,
            "/api/articles/preview" => preview_article(&body),
            "/api/seed" => change_seeding(server, &body).await,
            "/api/bots" => change_bots(server, &body).await,
            "/api/dungeon-spawn" => spawn_dungeon(server, &body).await,
            "/api/dungeons-reload" => Ok(json!({"ok": format!("{} dungeon(s) loaded.", crate::dungeons::load_catalog())})),
            _ => change_downtime(server, &body).await,
        };
        let (status, reply) = match result {
            Ok(v) => (200, v),
            Err((status, message)) => (status, json!({"error": message})),
        };
        return respond(&mut stream, status, "application/json", reply.to_string().as_bytes()).await;
    }
    if path == "/api/users" || path == "/api/users/role" {
        let host = header(&head, "Host").unwrap_or("");
        if !local || !local_host(host, stream.local_addr()?.port()) {
            return respond(&mut stream, 403, "application/json", br#"{"error":"Account management is available only on the server machine."}"#).await;
        }
        if method == "POST" && path == "/api/users/role" {
            // A custom header blocks cross-origin form posts; the browser's
            // Origin must also match this dashboard's Host when it sends one.
            let origin = header(&head, "Origin");
            if header(&head, "X-RMR-Admin") != Some("1") ||
                origin != Some(format!("http://{host}").as_str()) {
                return respond(&mut stream, 403, "application/json", br#"{"error":"Invalid dashboard request."}"#).await;
            }
            let query = target.split_once('?').map_or("", |(_, q)| q);
            let id = query.split('&').find_map(|kv| kv.strip_prefix("id=")).and_then(|s| s.parse::<i32>().ok());
            let role = query.split('&').find_map(|kv| kv.strip_prefix("role=")).map(decode);
            let (Some(id), Some(role)) = (id, role) else {
                return respond(&mut stream, 400, "application/json", br#"{"error":"Account ID and role are required."}"#).await;
            };
            let Some(bits) = crate::config::staff_role(&role) else {
                return respond(&mut stream, 400, "application/json", br#"{"error":"Unknown staff role."}"#).await;
            };
            let mask = !crate::config::STAFF_ROLE_BITS;
            let result = server.db.query_opt(
                r#"UPDATE "account" SET "role" = ("role" & $2) | $3 WHERE "accountID" = $1 RETURNING "accountName", "role""#,
                &[&id, &mask, &bits],
            ).await;
            let (status, body) = match result {
                Ok(Some(row)) => {
                    info!(account = %row.get::<_, String>(0), role = %role, "dashboard account role changed");
                    (200, json!({"ok": format!("{} is now {}. Relog in EVE to apply the role.", row.get::<_, String>(0), role)}))
                }
                Ok(None) => (404, json!({"error": "Account not found."})),
                Err(e) => {
                    warn!("dashboard role change: {}", crate::logging::chain(&e));
                    (500, json!({"error": "Database error."}))
                }
            };
            return respond(&mut stream, status, "application/json", body.to_string().as_bytes()).await;
        }
        if method != "GET" || path != "/api/users" {
            return respond(&mut stream, 405, "text/plain", b"Method not allowed").await;
        }
    }
    if method == "POST" && path == "/api/control" {
        let query = target.split_once('?').map_or("", |(_, q)| q);
        let asked = query.split('&').find_map(|kv| kv.strip_prefix("do=")).map(decode).unwrap_or_default();
        let (status, body) = if !crate::bots::controls_enabled() {
            (403, json!({ "error": "Dashboard controls are off (bots.dashboardControls in rmr.yml)." }))
        } else {
            let words: Vec<&str> = asked.split_whitespace().collect();
            match crate::bots::request(&words) {
                Ok(said) => (200, json!({ "ok": said })),
                Err(why) => (400, json!({ "error": why })),
            }
        };
        return respond(&mut stream, status, "application/json", body.to_string().as_bytes()).await;
    }
    if method != "GET" {
        return respond(&mut stream, 405, "text/plain", b"Only GET").await;
    }
    match path {
        "/" | "/index.html" => respond(&mut stream, 200, "text/html; charset=utf-8", INDEX.as_bytes()).await,
        "/client-news" => {
            let page = client_news(server).await.unwrap_or_else(|_| "<html><body><h2>News unavailable</h2></body></html>".into());
            respond(&mut stream, 200, "text/html; charset=utf-8", page.as_bytes()).await
        }
        "/style.css" => respond(&mut stream, 200, "text/css; charset=utf-8", STYLE.as_bytes()).await,
        "/app.js" => respond(&mut stream, 200, "text/javascript; charset=utf-8", SCRIPT.as_bytes()).await,
        "/eve-icon.png" => respond(&mut stream, 200, "image/png", EVE_ICON).await,
        img if img.starts_with("/img/") => {
            let query = target.split_once('?').map_or("", |(_, q)| q);
            let size = query.split('&').find_map(|kv| kv.strip_prefix("size=")).and_then(|s| s.parse::<u32>().ok()).unwrap_or(128).clamp(16, 1024);
            let mut parts = img.trim_start_matches("/img/").splitn(2, '/');
            let kind = parts.next().unwrap_or("");
            let id = parts.next().and_then(|s| s.trim_end_matches(".png").parse::<i64>().ok()).unwrap_or(0);
            match picture(server, kind, id, size).await {
                Picture::Png(bytes) => respond_image(&mut stream, &bytes).await,
                Picture::Redirect(url) => redirect(&mut stream, &url).await,
                Picture::Missing => respond(&mut stream, 404, "text/plain", b"No image").await,
            }
        }
        api if api.starts_with("/api/") => {
            let body = match api {
                "/api/summary" => Some(summary(server).await),
                "/api/news" => Some(news(server)),
                "/api/articles" => Some(articles(server).await),
                "/api/downtime" => Some(downtime(server).await),
                "/api/bots" => Some(Ok(bot_settings())),
                "/api/health" => Some(Ok(crate::health::report())),
                "/api/sieges" => Some(sieges(server)),
                "/api/wars" => Some(wars(server).await),
                "/api/players" => Some(players(server).await),
                "/api/users" => Some(users(server).await),
                "/api/alliances" => Some(alliances(server).await),
                "/api/corporations" => Some(corporations(server).await),
                "/api/systems" => Some(systems(server).await),
                "/api/kills" => {
                    let query = target.split_once('?').map_or("", |(_, q)| q);
                    let page = query.split('&').find_map(|kv| kv.strip_prefix("page=")).and_then(|s| s.parse::<i64>().ok()).unwrap_or(0).clamp(0, 10_000);
                    Some(kills(server, page).await)
                }
                "/api/killboard" => Some(killboard(server).await),
                "/api/stations" => {
                    let query = target.split_once('?').map_or("", |(_, q)| q);
                    let q = query.split('&').find_map(|kv| kv.strip_prefix("q=")).map(decode).unwrap_or_default();
                    Some(stations(server, &q).await)
                }
                "/api/sovmap/static" => Some(sov_map_static(server).await),
                "/api/sovmap" => Some(sov_map(server).await),
                "/api/dungeons" => Some(Ok(crate::dungeons::catalog_json())),
                "/api/guides" => Some(Ok(crate::dungeons::guides_json())),
                "/api/kill" => {
                    let query = target.split_once('?').map_or("", |(_, q)| q);
                    let id = query.split('&').find_map(|kv| kv.strip_prefix("id=")).and_then(|s| s.parse::<i64>().ok()).unwrap_or(0);
                    Some(kill_detail(server, id).await)
                }
                "/api/pilot" => {
                    let query = target.split_once('?').map_or("", |(_, q)| q);
                    let id = query.split('&').find_map(|kv| kv.strip_prefix("id=")).and_then(|s| s.parse::<i64>().ok()).unwrap_or(0);
                    Some(pilot_profile(server, id).await)
                }
                "/api/corporation" | "/api/alliance" => {
                    let query = target.split_once('?').map_or("", |(_, q)| q);
                    let id = query.split('&').find_map(|kv| kv.strip_prefix("id=")).and_then(|s| s.parse::<i64>().ok()).unwrap_or(0);
                    let kind = if api == "/api/alliance" { Group::Alliance } else { Group::Corporation };
                    Some(group_profile(server, kind, id).await)
                }
                "/api/group-kills" => {
                    let query = target.split_once('?').map_or("", |(_, q)| q);
                    let get = |name: &str| query.split('&').find_map(|kv| kv.strip_prefix(name)).map(decode).unwrap_or_default();
                    let id = get("id=").parse::<i64>().unwrap_or(0);
                    let page = get("page=").parse::<i64>().unwrap_or(0).clamp(0, 10_000);
                    let kind = if get("kind=") == "alliance" { Group::Alliance } else { Group::Corporation };
                    Some(group_kills(server, kind, id, &get("mode="), page).await)
                }
                "/api/pilot-kills" => {
                    let query = target.split_once('?').map_or("", |(_, q)| q);
                    let get = |name: &str| query.split('&').find_map(|kv| kv.strip_prefix(name)).map(decode).unwrap_or_default();
                    let id = get("id=").parse::<i64>().unwrap_or(0);
                    let page = get("page=").parse::<i64>().unwrap_or(0).clamp(0, 10_000);
                    let mode = get("mode=");
                    Some(pilot_kills(server, id, &mode, page).await)
                }
                "/api/pilot-search" => {
                    let query = target.split_once('?').map_or("", |(_, q)| q);
                    let q = query.split('&').find_map(|kv| kv.strip_prefix("q=")).map(decode).unwrap_or_default();
                    Some(pilot_search(server, &q).await)
                }
                "/api/market" => Some(Ok(server.world.lock().bot_panels.market.clone())),
                "/api/relations" => Some(Ok(server.world.lock().bot_panels.relations.clone())),
                "/api/followed" => Some(Ok(server.world.lock().bot_panels.followed.clone())),
                "/api/events" => Some(Ok(server.world.lock().bot_panels.events.clone())),
                "/api/pilots" => {
                    let query = target.split_once('?').map_or("", |(_, q)| q);
                    let q = query.split('&').find_map(|kv| kv.strip_prefix("q=")).map(decode).unwrap_or_default();
                    Some(Ok(pilots(server, &q)))
                }
                _ => None,
            };
            match body {
                Some(Ok(v)) => respond(&mut stream, 200, "application/json", v.to_string().as_bytes()).await,
                Some(Err(e)) => {
                    warn!("web {api}: {}", crate::logging::chain(&e));
                    respond(&mut stream, 500, "application/json", br#"{"error":"database"}"#).await
                }
                None => respond(&mut stream, 404, "application/json", br#"{"error":"no such thing"}"#).await,
            }
        }
        _ => respond(&mut stream, 404, "text/plain", b"Not found").await,
    }
}

fn header<'a>(head: &'a str, key: &str) -> Option<&'a str> {
    head.lines().skip(1).take_while(|line| !line.trim().is_empty()).find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.trim().eq_ignore_ascii_case(key).then_some(value.trim())
    })
}

fn local_host(host: &str, port: u16) -> bool {
    host == format!("127.0.0.1:{port}") ||
        host == format!("localhost:{port}") ||
        host == format!("[::1]:{port}")
}

async fn respond(stream: &mut TcpStream, status: u16, kind: &str, body: &[u8]) -> std::io::Result<()> {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        431 => "Request Header Fields Too Large",
        413 => "Content Too Large",
        _ => "Internal Server Error",
    };
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes()).await?;
    stream.write_all(body).await?;
    stream.shutdown().await
}

/// A portrait or logo for the killboard.
enum Picture {
    Png(Vec<u8>),
    Redirect(String),
    Missing,
}

/// Portraits come from the game client's own cache (the launcher names the
/// client folder in RMR_CLIENT_PATH); alliance logos from the logos alliances
/// upload; NPC corporation and faction logos from CCP's image server.
async fn picture(server: &Server, kind: &str, id: i64, size: u32) -> Picture {
    if id <= 0 || id > i64::from(i32::MAX) {
        return Picture::Missing;
    }
    match kind {
        "portrait" => portrait_file(id, size).map_or(Picture::Missing, Picture::Png),
        "alliance" => {
            let row = server.db.query_opt(r#"SELECT "logoImage" FROM alliances WHERE "allianceID" = $1"#, &[&(id as i32)]).await;
            match row {
                Ok(Some(r)) => r.try_get::<_, Option<Vec<u8>>>(0).ok().flatten().filter(|b| !b.is_empty()).map_or(Picture::Missing, Picture::Png),
                _ => Picture::Missing,
            }
        }
        "corporation" => {
            let npc = server.db.query_opt(
                r#"SELECT 1 FROM "crpNPCCorporations" WHERE "corporationID" = $1
                   UNION ALL SELECT 1 FROM "chrFactions" WHERE "factionID" = $1 LIMIT 1"#,
                &[&(id as i32)]).await;
            if matches!(npc, Ok(Some(_))) {
                let size = [32, 64, 128, 256].into_iter().find(|s| *s >= size).unwrap_or(256);
                Picture::Redirect(format!("https://images.evetech.net/corporations/{id}/logo?size={size}"))
            } else {
                Picture::Missing
            }
        }
        _ => Picture::Missing,
    }
}

/// The client caches portraits as Portraits/<characterID>_<size>.png; the
/// closest size at or above the one asked for, else the largest below it.
fn portrait_file(id: i64, size: u32) -> Option<Vec<u8>> {
    let client = std::env::var_os("RMR_CLIENT_PATH")?;
    let dir = std::path::Path::new(&client).join("cache").join("Pictures").join("Portraits");
    const SIZES: [u32; 8] = [32, 64, 128, 256, 484, 512, 692, 1024];
    let mut order: Vec<u32> = SIZES.iter().copied().filter(|s| *s >= size).collect();
    order.extend(SIZES.iter().copied().filter(|s| *s < size).rev());
    order.into_iter().find_map(|s| std::fs::read(dir.join(format!("{id}_{s}.png"))).ok())
}

async fn respond_image(stream: &mut TcpStream, body: &[u8]) -> std::io::Result<()> {
    let head = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: {}\r\nCache-Control: max-age=600\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes()).await?;
    stream.write_all(body).await?;
    stream.shutdown().await
}

async fn redirect(stream: &mut TcpStream, url: &str) -> std::io::Result<()> {
    let head = format!("HTTP/1.1 302 Found\r\nLocation: {url}\r\nContent-Length: 0\r\nCache-Control: max-age=3600\r\nConnection: close\r\n\r\n");
    stream.write_all(head.as_bytes()).await?;
    stream.shutdown().await
}

/// Rows as JSON objects, keyed by their column names.
fn rows(rows: &[Row]) -> Json {
    Json::Array(
        rows.iter()
            .map(|r| {
                let mut o = Map::new();
                for (i, c) in r.columns().iter().enumerate() {
                    let v = match *c.type_() {
                        Type::INT2 => r.try_get::<_, Option<i16>>(i).ok().flatten().map(Json::from),
                        Type::INT4 => r.try_get::<_, Option<i32>>(i).ok().flatten().map(Json::from),
                        Type::INT8 => r.try_get::<_, Option<i64>>(i).ok().flatten().map(Json::from),
                        Type::FLOAT4 => r.try_get::<_, Option<f32>>(i).ok().flatten().map(|f| json!(f)),
                        Type::FLOAT8 => r.try_get::<_, Option<f64>>(i).ok().flatten().map(|f| json!(f)),
                        Type::BOOL => r.try_get::<_, Option<bool>>(i).ok().flatten().map(Json::from),
                        _ => r.try_get::<_, Option<String>>(i).ok().flatten().map(Json::from),
                    };
                    o.insert(c.name().to_string(), v.unwrap_or(Json::Null));
                }
                Json::Object(o)
            })
            .collect(),
    )
}

/// A FILETIME column as milliseconds since 1970, for the page's Date.
fn ms(column: &str) -> String {
    format!("({column} / 10000 - {UNIX_EPOCH_MS})::bigint")
}

type Answer = Result<Json, tokio_postgres::Error>;

async fn users(server: &Server) -> Answer {
    let found = server.db.query(
        r#"SELECT "accountID" AS id, "accountName" AS name, "role", "banned" FROM "account" ORDER BY "accountName" LIMIT 2000"#,
        &[],
    ).await?;
    let mut list = rows(&found);
    if let Json::Array(accounts) = &mut list {
        for (item, row) in accounts.iter_mut().zip(found.iter()) {
            item["staffRole"] = json!(crate::config::staff_role_name(row.get::<_, i64>(2)));
        }
    }
    Ok(list)
}

async fn summary(server: &Server) -> Answer {
    let held: i64 = server.db.query_one(r#"SELECT count(*) FROM "mapSolarSystems" WHERE "allianceID" IS NOT NULL"#, &[]).await?.get(0);
    let w = server.world.lock();
    let b = &w.bot_stats;
    Ok(json!({
        "players": w.players().len(),
        "bots": { "total": b.total, "online": b.online, "inSpace": b.in_space, "running": b.running },
        "corporations": b.corporations,
        "alliances": b.alliances,
        "wars": b.wars,
        "lostInFights": b.lost,
        "systemsHeld": held,
        "uptimeSeconds": STARTED.get().map_or(0, |s| s.elapsed().as_secs()),
    }))
}

/// Sieges announced or under way, soonest first: where to go to see a battle.
fn sieges(server: &Server) -> Answer {
    let mut list = server.world.lock().bot_stats.sieges.clone();
    list.sort_by_key(|s| s.starts);
    Ok(Json::Array(
        list.iter()
            .map(|s| {
                json!({
                    "attacker": s.attacker, "defender": s.defender, "system": s.system, "region": s.region,
                    "security": (s.security.max(0.0) * 10.0).round() / 10.0, "starts": s.starts / 10_000 - UNIX_EPOCH_MS,
                    "fighting": s.fleets.is_some(),
                    "attackers": s.fleets.map(|f| f.0), "defenders": s.fleets.map(|f| f.1),
                    "stakes": s.stakes, "brought": [s.brought.0, s.brought.1], "lost": [s.lost.0, s.lost.1],
                    "attackerAllies": s.allies.0, "defenderAllies": s.allies.1, "seen": s.seen,
                })
            })
            .collect(),
    ))
}

/// What the bot alliances did in null-sec lately, newest first.
fn news(server: &Server) -> Answer {
    let news = server.world.lock().bot_stats.news.clone();
    Ok(Json::Array(news.iter().rev().map(|(at, what)| json!({ "time": at / 10_000 - UNIX_EPOCH_MS, "text": what })).collect()))
}

async fn articles(server: &Server) -> Answer {
    let mut list = rows(&server.db.query(r#"SELECT id, title, body, published_ms AS time
        FROM "rmrNewsArticles" ORDER BY published_ms DESC, id DESC LIMIT 100"#, &[]).await?);
    if let Json::Array(items) = &mut list {
        for item in items.iter_mut() {
            let html = crate::news::render(item["body"].as_str().unwrap_or(""), crate::news::Target::Dashboard);
            item["html"] = json!(html);
        }
    }
    Ok(list)
}

async fn downtime(server: &Server) -> Answer {
    let row = server.db.query_opt(r#"SELECT deadline_ms, message, active, restart FROM "rmrDowntime" WHERE id = 1"#, &[]).await?;
    Ok(match row {
        Some(row) => json!({"active": row.get::<_, bool>(2), "deadline": row.get::<_, i64>(0),
            "message": row.get::<_, String>(1), "restart": row.get::<_, bool>(3),
            "restartAvailable": restart_available(), "now": unix_ms_now()}),
        None => json!({"active": false, "deadline": null, "message": "", "restart": false,
            "restartAvailable": restart_available(), "now": unix_ms_now()}),
    })
}

type EditResult = Result<Json, (u16, String)>;

/// The most bots the dashboard will set; more makes a very long first start
/// (each is created once) and a busier database.
const MAX_BOTS: u64 = 1_000_000;

/// The bot settings for the dashboard: how many play now, how many the
/// config file asks for at the next start, and how they behave.
fn bot_settings() -> Json {
    json!({
        "running": crate::bots::running_count(),
        "configured": crate::config::bots_in_file().ok(),
        "aggression": crate::bots::aggression().as_str(),
        "chat": crate::bots::chatter().as_str(),
        "trashTalk": crate::bots::trash_talk(),
        "joinPlayerCorps": crate::bots::recruiting().as_str(),
        "joinPlayerAlliances": crate::bots::alliance_applying().as_str(),
        "acceptApplications": crate::bots::door().as_str(),
        "onlineActivity": (crate::bots::activity_factor() * 100.0).round() / 100.0,
        "shownPerSystem": crate::bots::shown_per_system(),
        "piratesAttackPlayers": crate::bots::pirates_attack_players(),
        "roamingGangs": crate::bots::roaming().as_str(),
        "huntPlayers": crate::bots::hunt_players(),
        "miningFleets": crate::bots::mining_fleets(),
        "clearBelts": crate::bots::clear_belts(),
        "wingmen": crate::bots::wingmen().as_str(),
        "botMarket": crate::bots::bot_market().as_str(),
        "sovWarfare": crate::bots::sov_warfare().as_str(),
        "battleSize": crate::bots::battle_size(),
        "battleSizeRange": [crate::bots::BATTLE_SIZE_RANGE.0, crate::bots::BATTLE_SIZE_RANGE.1],
        "countOverridden": crate::config::overridden("RMR_BOTS"),
        "aggressionOverridden": crate::config::overridden("RMR_BOTS_AGGRESSION"),
        "restartAvailable": restart_available(),
        "max": MAX_BOTS,
        "activityRange": [crate::bots::ACTIVITY_RANGE.0, crate::bots::ACTIVITY_RANGE.1],
        "shownRange": [crate::bots::SHOWN_RANGE.0, crate::bots::SHOWN_RANGE.1],
    })
}

/// Saves one bots setting to the config file, so it outlasts a restart.
fn save_bots_setting(key: &str, value: &str) -> Result<(), (u16, String)> {
    crate::config::set_bots_setting(key, value).map_err(|e| (500, format!("The bots changed for now, but the setting was not saved: {e}")))
}

/// Changes the bots: the count (saved for the next start, optionally
/// restarting now) and their behaviour (at once, and saved).
async fn change_bots(server: &Server, body: &[u8]) -> EditResult {
    use crate::bots::{Aggression, Chatter, Door, Rate, SovWarfare, Wingmen};
    let input: Json = serde_json::from_slice(body).map_err(|_| (400, "Invalid bot settings.".to_string()))?;
    let mut said: Vec<String> = Vec::new();
    let choice = |key: &str, choices: &str| (400, format!("{key}: choose {choices}."));
    if let Some(word) = input["aggression"].as_str() {
        let mode = Aggression::parse(word).ok_or_else(|| choice("Aggression", "calm, some aggressive or all aggressive"))?;
        crate::bots::set_aggression(mode);
        save_bots_setting("aggression", mode.as_str())?;
        said.push(match mode {
            Aggression::Calm => "All bots are calm: they attack no player unless shot first.",
            Aggression::Usual => "Some bots are aggressive: pirates outside high-sec, angered sides and hunters attack players.",
            Aggression::All => "All bots are aggressive: they attack every player who is not their friend.",
        }.to_string());
    }
    if let Some(word) = input["chat"].as_str() {
        let c = Chatter::parse(word).ok_or_else(|| choice("Local chat", "off, quiet, normal or chatty"))?;
        crate::bots::set_chatter(c);
        save_bots_setting("chat", c.as_str())?;
        said.push(format!("Local chat: {}.", c.as_str()));
    }
    if let Some(on) = input["trashTalk"].as_bool() {
        crate::bots::set_trash_talk(on);
        save_bots_setting("trashTalk", if on { "true" } else { "false" })?;
        said.push(if on { "Bots trash talk and troll." } else { "Bots keep it polite." }.to_string());
    }
    if let Some(word) = input["joinPlayerCorps"].as_str() {
        let r = Rate::parse(word).ok_or_else(|| choice("Applications to player corporations", "off, low, normal, high or extra-high"))?;
        crate::bots::set_recruiting(r);
        save_bots_setting("joinPlayerCorps", r.as_str())?;
        said.push(format!("Bots apply to player corporations: {}.", r.as_str()));
    }
    if let Some(word) = input["joinPlayerAlliances"].as_str() {
        let r = Rate::parse(word).ok_or_else(|| choice("Applications to player alliances", "off, low, normal, high or extra-high"))?;
        crate::bots::set_alliance_applying(r);
        save_bots_setting("joinPlayerAlliances", r.as_str())?;
        said.push(format!("Bot corporations apply to player alliances: {}.", r.as_str()));
    }
    if let Some(word) = input["acceptApplications"].as_str() {
        let d = Door::parse(word).ok_or_else(|| choice("Answering applications", "closed, selective or open"))?;
        crate::bots::set_door(d);
        save_bots_setting("acceptApplications", d.as_str())?;
        said.push(format!("Bot corporations and alliances are {} to applications.", d.as_str()));
    }
    if let Some(f) = input["onlineActivity"].as_f64() {
        let (lo, hi) = crate::bots::ACTIVITY_RANGE;
        if !(lo..=hi).contains(&f) {
            return Err((400, format!("Online activity must be from {lo} to {hi} times normal.")));
        }
        crate::bots::set_activity_factor(f);
        save_bots_setting("onlineActivity", &format!("{f}"))?;
        said.push(format!("Online activity: {f}x normal. Bots log on or off over the next few minutes."));
    }
    if let Some(n) = input["shownPerSystem"].as_u64() {
        let (lo, hi) = crate::bots::SHOWN_RANGE;
        if !(lo as u64..=hi as u64).contains(&n) {
            return Err((400, format!("Bot ships shown per system must be from {lo} to {hi}.")));
        }
        crate::bots::set_shown_per_system(n as usize);
        save_bots_setting("shownPerSystem", &n.to_string())?;
        said.push(format!("Up to {n} bot ships shown per system."));
    }
    if let Some(on) = input["piratesAttackPlayers"].as_bool() {
        crate::bots::set_pirates_attack_players(on);
        save_bots_setting("piratesAttackPlayers", if on { "true" } else { "false" })?;
        said.push(if on { "Pirates attack players outside high-sec." } else { "Pirates leave players alone unless angered." }.to_string());
    }
    if let Some(word) = input["roamingGangs"].as_str() {
        let r = Rate::parse(word).ok_or_else(|| choice("Roaming gangs", "off, low, normal or high"))?;
        crate::bots::set_roaming(r);
        save_bots_setting("roamingGangs", r.as_str())?;
        said.push(format!("Roaming gangs: {}.", r.as_str()));
    }
    if let Some(on) = input["huntPlayers"].as_bool() {
        crate::bots::set_hunt_players(on);
        save_bots_setting("huntPlayers", if on { "true" } else { "false" })?;
        said.push(if on { "Roaming gangs hunt for players in low and null-sec." } else { "Roaming gangs wander without looking for players." }.to_string());
    }
    if let Some(on) = input["miningFleets"].as_bool() {
        crate::bots::set_mining_fleets(on);
        save_bots_setting("miningFleets", if on { "true" } else { "false" })?;
        said.push(if on { "Miners from the same station mine together in fleets." } else { "Miners spread out over the belts." }.to_string());
    }
    if let Some(on) = input["clearBelts"].as_bool() {
        crate::bots::set_clear_belts(on);
        save_bots_setting("clearBelts", if on { "true" } else { "false" })?;
        said.push(if on { "Bot miners mine asteroids down and clear belts." } else { "Bot miners leave the asteroids as they are." }.to_string());
    }
    if let Some(word) = input["wingmen"].as_str() {
        let w = Wingmen::parse(word).ok_or_else(|| choice("Wingmen", "off, free, cheap or paid"))?;
        crate::bots::set_wingmen(w);
        save_bots_setting("wingmen", w.as_str())?;
        said.push(match w {
            Wingmen::Off => "Bots turn down gang invitations.",
            Wingmen::Free => "Bots join players' gangs for free.",
            Wingmen::Cheap => "Bots join players' gangs at a quarter of their usual rate.",
            Wingmen::Paid => "Bots join players' gangs at their usual hourly rate.",
        }.to_string());
    }
    if let Some(word) = input["botMarket"].as_str() {
        let r = Rate::parse(word).ok_or_else(|| choice("Bot market", "off, low, normal or high"))?;
        crate::bots::set_bot_market(r);
        save_bots_setting("botMarket", r.as_str())?;
        said.push(format!("Bot traders' market orders: {}.", r.as_str()));
    }
    if let Some(word) = input["sovWarfare"].as_str() {
        let w = SovWarfare::parse(word).ok_or_else(|| choice("Sovereignty warfare", "low, moderate or war"))?;
        crate::bots::set_sov_warfare(w);
        save_bots_setting("sovWarfare", w.as_str())?;
        said.push(match w {
            SovWarfare::Low => "Sovereignty warfare: low. Rare sieges, at most 2 at once, one ally a side, and slow expansion.",
            SovWarfare::Moderate => "Sovereignty warfare: moderate. Regular sieges, at most 6 at once, up to 3 allies a side.",
            SovWarfare::War => "Sovereignty warfare: war. Sieges everywhere, up to 24 at once, up to 5 allies a side, and fast expansion.",
        }.to_string());
    }
    // A number, or a preset button's value as text.
    let battle = input["battleSize"].as_u64().or_else(|| input["battleSize"].as_str().and_then(|t| t.trim().parse().ok()));
    if let Some(n) = battle {
        let (lo, hi) = crate::bots::BATTLE_SIZE_RANGE;
        if !(lo as u64..=hi as u64).contains(&n) {
            return Err((400, format!("Battle size must be from {lo} to {hi} ships a side.")));
        }
        crate::bots::set_battle_size(n as usize);
        save_bots_setting("battleSize", &n.to_string())?;
        said.push(format!("Battles: up to {n} ships a side, from the next battle on."));
    }
    let restart = input["restart"].as_bool().unwrap_or(false);
    if !input["count"].is_null() {
        let count = input["count"].as_u64().filter(|n| *n <= MAX_BOTS)
            .ok_or_else(|| (400, format!("Enter a whole number of bots from 0 to {MAX_BOTS}.")))?;
        crate::config::set_bots_setting("count", &count.to_string())
            .map_err(|e| (500, format!("Could not save the bot count: {e}")))?;
        info!(count, "dashboard set the bot count");
        said.push(if restart {
            format!("Bot count set to {count}.")
        } else {
            format!("Bot count set to {count}; it takes effect when the server restarts.")
        });
        if crate::config::overridden("RMR_BOTS") {
            said.push("Note: the RMR_BOTS environment variable overrides this setting.".into());
        }
    }
    if restart {
        if !restart_available() {
            return Err((400, "Restarting needs the RMR launcher (Start.bat), which starts the server again.".into()));
        }
        change_downtime(server, br#"{"action":"restart-now"}"#).await?;
        said.push("Restarting the server now.".into());
    }
    if said.is_empty() {
        return Err((400, "Nothing to change.".into()));
    }
    info!(changes = %said.join(" "), "dashboard changed the bots");
    Ok(json!({"ok": said.join(" "), "settings": bot_settings()}))
}

/// Article bodies, formatting tags included.
const ARTICLE_MAX_CHARS: usize = 30_000;

/// How an article body will look, for the dashboard's preview.
fn preview_article(body: &[u8]) -> EditResult {
    let input: Json = serde_json::from_slice(body).map_err(|_| (400, "Invalid article data.".to_string()))?;
    let text = input["body"].as_str().unwrap_or("");
    if text.chars().count() > ARTICLE_MAX_CHARS {
        return Err((400, "Articles can be up to 30,000 characters.".into()));
    }
    Ok(json!({"html": crate::news::render(text.trim(), crate::news::Target::Dashboard)}))
}

async fn publish_article(server: &Server, body: &[u8]) -> EditResult {
    let input: Json = serde_json::from_slice(body).map_err(|_| (400, "Invalid article data.".to_string()))?;
    let title = input["title"].as_str().unwrap_or("").trim();
    let text = input["body"].as_str().unwrap_or("").trim();
    if title.is_empty() || title.chars().count() > 140 || text.is_empty() || text.chars().count() > ARTICLE_MAX_CHARS {
        return Err((400, "Enter a title (up to 140 characters) and article (up to 30,000 characters).".into()));
    }
    let published = unix_ms_now();
    let row = server.db.query_one(r#"INSERT INTO "rmrNewsArticles" (title, body, published_ms)
        VALUES ($1, $2, $3) RETURNING id"#, &[&title, &text, &published]).await
        .map_err(|e| { warn!("publishing news: {}", crate::logging::chain(&e)); (500, "Could not save article.".into()) })?;
    crate::news::record_note(title, text, published);
    Ok(json!({"ok": "Article published to the dashboard and character select.", "id": row.get::<_, i64>(0)}))
}

async fn change_downtime(server: &Server, body: &[u8]) -> EditResult {
    let input: Json = serde_json::from_slice(body).map_err(|_| (400, "Invalid downtime data.".to_string()))?;
    let action = input["action"].as_str().unwrap_or("");
    if action == "cancel" {
        let removed = server.db.query_opt(r#"UPDATE "rmrDowntime" SET active = false
            WHERE id = 1 AND active RETURNING id"#, &[]).await
            .map_err(|e| { warn!("canceling downtime: {}", crate::logging::chain(&e)); (500, "Could not cancel downtime.".into()) })?;
        if removed.is_some() {
            let w = server.world.lock();
            w.notify(&w.all_clients(), "OnClusterShutdownCancelled",
                Value::Tuple(vec![Value::text("The scheduled downtime has been cancelled.")]));
        }
        return Ok(json!({"ok": if removed.is_some() { "Downtime cancelled." } else { "No downtime was scheduled." }}));
    }
    let (countdown, restart) = match action {
        "schedule" => (true, false),
        "now" => (false, false),
        "restart" => (true, true),
        "restart-now" => (false, true),
        _ => return Err((400, "Choose a countdown, an immediate shutdown or restart, or cancel.".into())),
    };
    if restart && !restart_available() {
        return Err((400, "Restarting needs the RMR launcher (Start.bat), which starts the server again.".into()));
    }
    let message = input["message"].as_str().unwrap_or("").trim();
    if message.chars().count() > 500 {
        return Err((400, "Downtime announcement must be under 500 characters.".into()));
    }
    let message = match (message.is_empty(), restart) {
        (false, _) => message,
        (true, true) => "The server is restarting. It will be back in a few minutes.",
        (true, false) => "The server is going down for maintenance.",
    };
    let deadline = unix_ms_now() + if countdown { 300_000 } else { 0 };
    server.db.execute(r#"INSERT INTO "rmrDowntime" (id, deadline_ms, message, active, restart)
        VALUES (1, $1, $2, true, $3) ON CONFLICT (id) DO UPDATE SET
        deadline_ms = EXCLUDED.deadline_ms, message = EXCLUDED.message, active = true, restart = EXCLUDED.restart"#,
        &[&deadline, &message, &restart]).await
        .map_err(|e| { warn!("scheduling downtime: {}", crate::logging::chain(&e)); (500, "Could not schedule downtime.".into()) })?;
    let w = server.world.lock();
    w.notify(&w.all_clients(), "OnClusterShutdownInitiated", Value::Tuple(vec![
        Value::text(message), Value::Long(filetime_at_ms(deadline)),
    ]));
    info!(deadline, action, "dashboard scheduled downtime");
    let ok = match action {
        "schedule" => "Five-minute downtime countdown started.",
        "restart" => "Five-minute restart countdown started. The server will save, stop and start again.",
        "restart-now" => "Restarting the server. It will save, stop and start again.",
        _ => "Shutting down the server.",
    };
    Ok(json!({"ok": ok, "deadline": deadline}))
}

/// Stations for the market-seeding picker: with no search, the seeded
/// stations and every player-owned station (outposts); otherwise up to 40
/// stations whose name, system or region matches.
async fn stations(server: &Server, q: &str) -> Answer {
    let seeded = crate::market_seed::seeded_ids(&server.db).await?;
    let q = q.trim();
    let pattern = format!("%{}%", q.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_"));
    let sql = r#"SELECT s."stationID" AS id, COALESCE(s."stationName", '') AS name,
                        COALESCE(m."solarSystemName", '') AS system, COALESCE(r."regionName", '') AS region,
                        round(GREATEST(COALESCE(m."security", 0), 0)::numeric, 1)::float8 AS security,
                        COALESCE(co."corporationName", '') AS owner,
                        (s."corporationID" >= $3) AS "playerOwned",
                        (s."stationID" = ANY($1::int[])) AS seeded,
                        (SELECT count(*) FROM market_orders o WHERE o."stationID" = s."stationID"
                            AND o."bid" = 0 AND o."orderState" = 0 AND o."volRemaining" > 0) AS "sellOrders"
                   FROM "staStations" s
                   LEFT JOIN "mapSolarSystems" m ON m."solarSystemID" = s."solarSystemID"
                   LEFT JOIN "mapRegions" r ON r."regionID" = s."regionID"
                   LEFT JOIN "corporation" co ON co."corporationID" = s."corporationID"
                  WHERE CASE WHEN $2 = '%%' THEN s."stationID" = ANY($1::int[]) OR s."corporationID" >= $3
                                             OR s."stationID" >= 61000000
                             ELSE s."stationName" ILIKE $2 OR m."solarSystemName" ILIKE $2 OR r."regionName" ILIKE $2 END
                  ORDER BY (s."stationID" = ANY($1::int[])) DESC, (s."corporationID" >= $3) DESC, s."stationName"
                  LIMIT 40"#;
    Ok(rows(&server.db.query(sql, &[&seeded, &pattern, &crate::market_seed::FIRST_PLAYER]).await?))
}

/// Opens a dungeon for a pilot in space, out of sight of where they are, and
/// warps them to it; a bookmark marks it for later.
async fn spawn_dungeon(server: &Server, body: &[u8]) -> EditResult {
    let input: Json = serde_json::from_slice(body).map_err(|_| (400, "Invalid request.".to_string()))?;
    let id = input["dungeon"].as_str().unwrap_or("");
    let pilot = input["characterID"].as_i64().unwrap_or(0);
    let def = crate::dungeons::find(id).ok_or((404, format!("There is no dungeon {id}.")))?;
    let presence = server.world.lock().presence_of(pilot).ok_or((400, "That pilot is not online.".to_string()))?;
    // The system: one named, else wherever the pilot is.
    let wanted = input["system"].as_str().unwrap_or("").trim();
    let (system, system_name): (i64, String) = if wanted.is_empty() {
        let name = server.db.query_opt(r#"SELECT "solarSystemName" FROM "mapSolarSystems" WHERE "solarSystemID" = $1"#, &[&(presence.solar_system_id as i32)])
            .await.ok().flatten().map(|r| r.get::<_, String>(0)).unwrap_or_default();
        (presence.solar_system_id, name)
    } else {
        let row = server.db.query_opt(r#"SELECT "solarSystemID", "solarSystemName" FROM "mapSolarSystems" WHERE lower("solarSystemName") = lower($1)"#, &[&wanted])
            .await.map_err(|e| (500, format!("Looking up the system failed: {}", crate::logging::chain(&e))))?
            .ok_or((400, format!("There is no solar system called {wanted}.")))?;
        (i64::from(row.get::<_, i32>(0)), row.get(1))
    };
    let here = system == presence.solar_system_id && !presence.docked;
    if !here {
        // Not in space there: the complex waits as a signature already found,
        // somewhere 3 to 15 AU from the sun, and opens when they warp to it.
        const AU: f64 = 149_597_870_700.0;
        let mut rng = crate::combat::Rng::seeded(crate::bulkdata::filetime_now() as u64 ^ pilot as u64);
        let dir = rng.in_sphere(1.0);
        let dir = if dir.len() > 0.1 { dir.unit() } else { crate::ballpark::Vec3::new(1.0, 0.0, 0.0) };
        let at = crate::ballpark::Vec3::new(dir.x, dir.y * 0.15, dir.z) * (rng.range(3.0, 15.0) * AU);
        crate::exploration::grant(pilot, system, &def.id, def.rating, at, None);
        bookmark_site(server, pilot, system, &def, at).await;
        info!(pilot, dungeon = %def.id, system, "dashboard gave a pilot a dungeon");
        return Ok(json!({"ok": format!(
            "{} (DED {}/10, {} rooms) is waiting in {system_name}. In that system, open the System Scanner: it is listed as a result already pinned down, with no probes needed. Right-click it and Warp To, and the complex opens as you arrive.",
            def.name, def.rating, def.rooms.len()
        )}));
    }
    let rats = crate::npc::load_dungeon_rats(&server.db, &def.faction, &def.ship_names(), &def.name_classes())
        .await
        .map_err(|e| (500, format!("Loading the site's ships failed: {}", crate::logging::chain(&e))))?
        .ok_or((400, format!("There is no pirate faction {}.", def.faction)))?;
    let now = crate::combat::now_ms();
    let (at, landing) = {
        let mut w = server.world.lock();
        let (ship_id, at, landing, nearby) = {
            let Some(park) = w.parks.get_mut(&system) else { return Err((400, "That pilot's system is not loaded.".into())) };
            let Some((ship_id, pos)) = park.ships.values().find(|s| s.is_player() && s.pilot.char_id == pilot).map(|s| (s.id, s.pos)) else {
                return Err((400, "That pilot's ship is not in space.".into()));
            };
            // Out of sight of where they are: about 0.03 AU off, any way.
            let dir = crate::combat::Rng::seeded(now as u64 ^ pilot as u64).in_sphere(1.0);
            let dir = if dir.len() > 0.1 { dir.unit() } else { crate::ballpark::Vec3::new(1.0, 0.0, 0.0) };
            let at = pos + dir * 5.0e9;
            let landing = at + (pos - at).unit() * 2_500.0;
            let (site, nearby) = crate::dungeons::open(park, std::sync::Arc::clone(&def), std::sync::Arc::new(rats), at, pilot, 0, now);
            crate::exploration::grant(pilot, system, &def.id, def.rating, at, Some(site));
            (ship_id, at, landing, nearby)
        };
        crate::svc::space::notify_targeted_updates(&w, nearby);
        crate::svc::space::order(&mut w, system, ship_id, crate::ballpark::Command::Warp { target: landing, range: 0.0 });
        (at, landing)
    };
    let _ = landing;
    bookmark_site(server, pilot, system, &def, at).await;
    info!(pilot, dungeon = %def.id, "dashboard spawned a dungeon");
    Ok(json!({"ok": format!(
        "{} (DED {}/10, {} rooms) is open in {system_name}, and the pilot's ship is warping there now. It is also listed in their System Scanner to warp back to.",
        def.name, def.rating, def.rooms.len()
    )}))
}

/// A bookmark at a dungeon given from the dashboard (People & Places shows
/// it from the pilot's next login).
async fn bookmark_site(server: &Server, pilot: i64, system: i64, def: &crate::dungeons::Dungeon, at: crate::ballpark::Vec3) {
    let memo = format!("DED {}/10 {}", def.rating, def.name);
    if let Err(e) = server
        .db
        .execute(
            r#"INSERT INTO bookmarks ("ownerID","itemID","typeID","flag","memo","created","x","y","z","locationID")
               VALUES ($1, $2, 5, NULL, $3, $4, $5, $6, $7, $2)"#,
            &[&(pilot as i32), &(system as i32), &memo, &crate::bulkdata::filetime_now(), &at.x, &at.y, &at.z],
        )
        .await
    {
        warn!("dungeon bookmark: {}", crate::logging::chain(&e));
    }
}

async fn change_seeding(server: &Server, body: &[u8]) -> EditResult {
    let input: Json = serde_json::from_slice(body).map_err(|_| (400, "Invalid seeding request.".to_string()))?;
    let station = input["stationID"].as_i64().filter(|id| (60_000_000..64_000_000).contains(id))
        .ok_or((400, "Pick a station to seed.".to_string()))? as i32;
    let name: Option<String> = server.db.query_opt(r#"SELECT COALESCE("stationName", '') FROM "staStations" WHERE "stationID" = $1"#, &[&station]).await
        .map_err(|e| { warn!("seeding lookup: {}", crate::logging::chain(&e)); (500, "Could not look up the station.".to_string()) })?
        .map(|r| r.get(0));
    let Some(name) = name else { return Err((404, "That station does not exist.".into())) };
    match input["action"].as_str().unwrap_or("seed") {
        "seed" => {
            let (added, topped) = crate::market_seed::seed_station(&server.db, station).await
                .map_err(|e| { warn!("seeding station {station}: {}", crate::logging::chain(&e)); (500, "Could not seed the station market.".to_string()) })?;
            Ok(json!({"ok": format!("{name}: market seeded ({added} new NPC sell orders, {topped} restocked). It is restocked at every server start."),
                "added": added, "restocked": topped}))
        }
        "unseed" => {
            let removed = crate::market_seed::unseed_station(&server.db, station).await
                .map_err(|e| { warn!("unseeding station {station}: {}", crate::logging::chain(&e)); (500, "Could not stop seeding the station.".to_string()) })?;
            Ok(json!({"ok": format!("{name}: no longer seeded ({removed} NPC sell orders removed)."), "removed": removed}))
        }
        _ => Err((400, "Choose seed or unseed.".into())),
    }
}

/// Text for the EVE client's browser, which knows the named entities for
/// markup characters but not numeric ones such as &#39;.
fn client_text(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// The character selection news panel: patch-notes style, newest first.
async fn client_news(server: &Server) -> Result<String, tokio_postgres::Error> {
    let articles = server.db.query(r#"SELECT title, body,
        to_char(to_timestamp(published_ms / 1000.0) AT TIME ZONE 'UTC', 'FMDD FMMonth YYYY') AS published
        FROM "rmrNewsArticles"
        ORDER BY published_ms DESC, id DESC LIMIT 25"#, &[]).await?;
    let mut html = String::from("<html><body bgcolor=\"#101820\" text=\"#d8dee4\" link=\"#9bd0ff\"><font face=\"Tahoma,Arial\" size=\"4\"><h1><font color=\"#ffffff\">Red Moon Rising updates</font></h1><hr>");
    if articles.is_empty() { html.push_str("<p>No news articles have been published yet.</p>"); }
    for row in articles {
        let title: String = row.get(0);
        let body: String = row.get(1);
        let at: String = row.get(2);
        html.push_str(&format!("<h2><font color=\"{}\">{}</font></h2><font color=\"#8c949c\" size=\"3\">{}</font><br><br>{}<br><br>",
            crate::news::CLIENT_TITLE, client_text(&title), at, crate::news::render(&body, crate::news::Target::Client)));
    }
    html.push_str("</font></body></html>");
    Ok(html)
}

async fn wars(server: &Server) -> Answer {
    let now = crate::bulkdata::filetime_now();
    let sql = format!(
        r#"SELECT w."warID" AS id,
                  COALESCE(da."allianceName", dc."corporationName", '?') AS "declaredBy",
                  COALESCE(da."shortName", dc."tickerName", '') AS "declaredByTicker",
                  COALESCE(aa."allianceName", ac."corporationName", '?') AS against,
                  COALESCE(aa."shortName", ac."tickerName", '') AS "againstTicker",
                  {declared} AS declared, {finished} AS ends,
                  w."mutual" = 1 AS mutual, EXISTS (SELECT 1 FROM "botGroups" g WHERE g."ownerID" = w."declaredByID") AS bots
             FROM "crpWars" w
             LEFT JOIN alliances da ON da."allianceID" = w."declaredByID"
             LEFT JOIN "corporation" dc ON dc."corporationID" = w."declaredByID"
             LEFT JOIN alliances aa ON aa."allianceID" = w."againstID"
             LEFT JOIN "corporation" ac ON ac."corporationID" = w."againstID"
            WHERE w."retracted" IS NULL OR w."retracted" > $1
            ORDER BY w."timeDeclared" DESC LIMIT 200"#,
        declared = ms(r#"w."timeDeclared""#),
        finished = ms(r#"w."timeFinished""#),
    );
    let found = server.db.query(sql.as_str(), &[&now]).await?;
    let losses = server.world.lock().bot_stats.war_losses.clone();
    let mut list = rows(&found);
    if let Json::Array(items) = &mut list {
        for (item, r) in items.iter_mut().zip(&found) {
            let id: i32 = r.get(0);
            item["losses"] = json!(losses.get(&(id as i64)).copied().unwrap_or(0));
        }
    }
    Ok(list)
}

/// The people playing, not the bots: where each is and in what.
async fn players(server: &Server) -> Answer {
    let present = server.world.lock().players();
    let ids: Vec<i32> = present.iter().map(|p| p.char_id as i32).collect();
    let sql = r#"SELECT c."characterID" AS id, c."characterName" AS name, co."corporationName" AS corporation,
                        co."tickerName" AS ticker, a."allianceName" AS alliance, round(c."securityRating"::numeric, 1)::float8 AS security,
                        t."typeName" AS ship
                   FROM "character_" c
                   LEFT JOIN "corporation" co ON co."corporationID" = c."corporationID"
                   LEFT JOIN alliances a ON a."allianceID" = co."allianceID"
                   LEFT JOIN entity e ON e."itemID" = c."shipID"
                   LEFT JOIN "invTypes" t ON t."typeID" = e."typeID"
                  WHERE c."characterID" = ANY($1) ORDER BY c."characterName""#;
    let found = server.db.query(sql, &[&ids]).await?;
    let systems: Vec<i32> = present.iter().map(|p| p.solar_system_id as i32).collect();
    let names: HashMap<i32, (String, f64)> = server
        .db
        .query(r#"SELECT "solarSystemID", "solarSystemName", "security" FROM "mapSolarSystems" WHERE "solarSystemID" = ANY($1)"#, &[&systems])
        .await?
        .iter()
        .map(|r| (r.get::<_, i32>(0), (r.get::<_, Option<String>>(1).unwrap_or_default(), r.get::<_, Option<f64>>(2).unwrap_or(0.0))))
        .collect();
    let mut list = rows(&found);
    if let Json::Array(items) = &mut list {
        for item in items.iter_mut() {
            let id = item["id"].as_i64().unwrap_or(0);
            if let Some(p) = present.iter().find(|p| p.char_id == id) {
                let (system, security) = names.get(&(p.solar_system_id as i32)).cloned().unwrap_or_default();
                item["system"] = json!(system);
                item["systemSecurity"] = json!((security.max(0.0) * 10.0).round() / 10.0);
                item["docked"] = json!(p.docked);
            }
        }
    }
    Ok(list)
}

async fn alliances(server: &Server) -> Answer {
    let sql = r#"SELECT a."allianceID" AS id, a."allianceName" AS name, a."shortName" AS ticker,
                        count(co."corporationID")::int AS corporations, COALESCE(sum(co."memberCount"), 0)::int AS pilots,
                        CASE g."kind" WHEN 11 THEN 'null-sec' WHEN 12 THEN 'pirate' WHEN 13 THEN 'industrial' ELSE 'players' END AS kind,
                        r."regionName" AS region,
                        (SELECT count(*) FROM "mapSolarSystems" s WHERE s."allianceID" = a."allianceID")::int AS systems
                   FROM alliances a
                   LEFT JOIN "corporation" co ON co."allianceID" = a."allianceID" AND co."deleted" = 0
                   LEFT JOIN "botGroups" g ON g."ownerID" = a."allianceID"
                   LEFT JOIN "mapRegions" r ON r."regionID" = g."regionID"
                  WHERE a."deleted" = 0
                  GROUP BY a."allianceID", g."kind", r."regionName"
                  ORDER BY pilots DESC LIMIT 60"#;
    Ok(rows(&server.db.query(sql, &[]).await?))
}

async fn corporations(server: &Server) -> Answer {
    let sql = r#"SELECT co."corporationID" AS id, co."corporationName" AS name, co."tickerName" AS ticker, co."memberCount" AS members,
                        NULLIF(co."allianceID", 0) AS "allianceID",
                        a."shortName" AS alliance, EXISTS (SELECT 1 FROM "botGroups" g WHERE g."ownerID" = co."corporationID") AS bots
                   FROM "corporation" co
                   LEFT JOIN alliances a ON a."allianceID" = co."allianceID"
                  WHERE co."deleted" = 0
                    AND NOT EXISTS (SELECT 1 FROM "crpNPCCorporations" n WHERE n."corporationID" = co."corporationID")
                  ORDER BY co."memberCount" DESC LIMIT 40"#;
    Ok(rows(&server.db.query(sql, &[]).await?))
}

/// The busiest systems by pilots online: in space and docked, bots included.
async fn systems(server: &Server) -> Answer {
    let (space, docked) = server.world.lock().counts();
    let mut busiest: Vec<(i64, i64, i64)> = space.iter().map(|(s, n)| (*s, *n, docked.get(s).copied().unwrap_or(0))).collect();
    busiest.sort_by(|a, b| (b.1 + b.2).cmp(&(a.1 + a.2)).then(a.0.cmp(&b.0)));
    busiest.truncate(30);
    let ids: Vec<i32> = busiest.iter().map(|(s, _, _)| *s as i32).collect();
    let sql = r#"SELECT s."solarSystemID", s."solarSystemName", round(GREATEST(s."security", 0)::numeric, 1)::float8, r."regionName"
                   FROM "mapSolarSystems" s JOIN "mapRegions" r ON r."regionID" = s."regionID"
                  WHERE s."solarSystemID" = ANY($1)"#;
    let names: HashMap<i32, (String, f64, String)> = server
        .db
        .query(sql, &[&ids])
        .await?
        .iter()
        .map(|r| (r.get(0), (r.get::<_, Option<String>>(1).unwrap_or_default(), r.get::<_, Option<f64>>(2).unwrap_or(0.0), r.get::<_, Option<String>>(3).unwrap_or_default())))
        .collect();
    Ok(Json::Array(
        busiest
            .iter()
            .map(|(id, in_space, in_station)| {
                let (name, security, region) = names.get(&(*id as i32)).cloned().unwrap_or_default();
                json!({ "id": id, "name": name, "security": security, "region": region, "inSpace": in_space, "docked": in_station })
            })
            .collect(),
    ))
}

/// A query string's value: `+` for spaces, `%XX` for bytes.
fn decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < bytes.len() => {
                match u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or(""), 16) {
                    Ok(b) => {
                        out.push(b);
                        i += 2;
                    }
                    Err(_) => out.push(b'%'),
                }
            }
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Bots whose names start with (or contain) `q`, for following one.
fn pilots(server: &Server, q: &str) -> Json {
    let q = q.trim().to_lowercase();
    if q.len() < 2 {
        return Json::Array(Vec::new());
    }
    let roster = std::sync::Arc::clone(&server.world.lock().bot_roster);
    let mut found: Vec<&crate::world::LocalBot> = roster.iter().filter(|b| b.name.to_lowercase().starts_with(&q)).take(15).collect();
    if found.len() < 15 {
        found.extend(roster.iter().filter(|b| !b.name.to_lowercase().starts_with(&q) && b.name.to_lowercase().contains(&q)).take(15 - found.len()));
    }
    Json::Array(found.iter().map(|b| json!({ "id": b.char_id, "name": b.name })).collect())
}

/// The last day's kills and losses by alliance (or corporation outside
/// one), bots' unseen fights included.
async fn killboard(server: &Server) -> Answer {
    let since = crate::bulkdata::filetime_now() - 24 * 3_600 * 10_000_000;
    let week = crate::bulkdata::filetime_now() - 7 * 24 * 3_600 * 10_000_000;
    let sql = r#"WITH recent AS (SELECT * FROM "killLog" WHERE "killTime" > $1),
                      k AS (SELECT COALESCE(NULLIF(p."allianceID", 0), p."corporationID") AS side, count(*)::int AS n
                              FROM "killLogParticipants" p JOIN recent r ON r."killID" = p."killID" WHERE p."finalBlow" = 1 GROUP BY 1),
                      l AS (SELECT COALESCE(NULLIF(r."victimAllianceID", 0), r."victimCorpID") AS side, count(*)::int AS n FROM recent r GROUP BY 1),
                      sides AS (SELECT side FROM k UNION SELECT side FROM l)
                 SELECT s.side AS id, COALESCE(a."allianceName", c."corporationName", '?') AS name,
                        COALESCE(a."shortName", c."tickerName", '') AS ticker,
                        COALESCE(k.n, 0) AS kills, COALESCE(l.n, 0) AS losses,
                        EXISTS (SELECT 1 FROM "botGroups" g WHERE g."ownerID" = s.side) AS bots
                   FROM sides s LEFT JOIN k ON k.side = s.side LEFT JOIN l ON l.side = s.side
                   LEFT JOIN alliances a ON a."allianceID" = s.side LEFT JOIN "corporation" c ON c."corporationID" = s.side
                  ORDER BY kills DESC, losses DESC LIMIT 30"#;
    let sides = rows(&server.db.query(sql, &[&since]).await?);
    let counts = server.db.query_one(r#"SELECT count(*) AS total,
         count(*) FILTER (WHERE "killTime" > $1) AS day,
         count(*) FILTER (WHERE "killTime" > $2) AS week,
         count(*) FILTER (WHERE "isPodKill" = 1) AS pods FROM "killLog""#, &[&since, &week]).await?;
    let total: i64 = counts.get("total");
    let ships = rows(
        &server
            .db
            .query(
                r#"SELECT t."typeName" AS ship, count(*)::int AS lost FROM "killLog" k JOIN "invTypes" t ON t."typeID" = k."victimShipTypeID"
                    WHERE k."killTime" > $1 GROUP BY 1 ORDER BY 2 DESC LIMIT 10"#,
                &[&since],
            )
            .await?,
    );
    let pilots = rows(&server.db.query(
        r#"SELECT p."ownerID" AS id, COALESCE(c."characterName", 'Unknown pilot') AS name,
                  COALESCE(co."tickerName", '') AS corporation, count(*)::int AS kills
             FROM "killLogParticipants" p
             JOIN "killLog" k ON k."killID" = p."killID"
             LEFT JOIN "character_" c ON c."characterID" = p."ownerID"
             LEFT JOIN "corporation" co ON co."corporationID" = c."corporationID"
            WHERE p."finalBlow" = 1
            GROUP BY p."ownerID", c."characterName", co."tickerName"
    ORDER BY kills DESC, name LIMIT 25"#, &[]).await?);
    // The most valuable losses of the week, as zKillboard's top rows: ships
    // (category 6) and structures (category 23), ranked by hull base price.
    let featured_sql = |category: i32| format!(r#"SELECT k."killID" AS id, {time} AS time,
        k."victimCharID" AS "victimID", COALESCE(c."characterName", 'Unknown pilot') AS victim,
        COALESCE(co."corporationName", '') AS "victimCorporation",
        COALESCE(t."typeName", 'Unknown ship') AS ship, k."victimShipTypeID" AS "shipTypeID",
        COALESCE(s."solarSystemName", 'Unknown system') AS system,
        COALESCE(t."basePrice", 0)::float8 AS "hullValue", {value} AS "totalValue"
        FROM "killLog" k
        LEFT JOIN "character_" c ON c."characterID" = k."victimCharID"
        LEFT JOIN "corporation" co ON co."corporationID" = k."victimCorpID"
        LEFT JOIN "invTypes" t ON t."typeID" = k."victimShipTypeID"
        LEFT JOIN "invGroups" g ON g."groupID" = t."groupID"
        LEFT JOIN "mapSolarSystems" s ON s."solarSystemID" = k."solarSystemID"
        WHERE k."killTime" > $1 AND COALESCE(t."basePrice", 0) > 0 AND g."categoryID" = {category}
        ORDER BY t."basePrice" DESC, k."killID" DESC LIMIT 6"#, time = ms(r#"k."killTime""#), value = KILL_VALUE);
    let featured = rows(&server.db.query(&featured_sql(6), &[&week]).await?);
    let structures = rows(&server.db.query(&featured_sql(23), &[&week]).await?);
    let activity = server.db.query_one(r#"WITH recent AS (
            SELECT "killID", "victimCharID", "victimCorpID", NULLIF("victimAllianceID", 0) AS "victimAllianceID",
                   "victimShipTypeID", "solarSystemID" FROM "killLog" WHERE "killTime" > $1),
          people AS (
            SELECT p."ownerID" AS who, p."corporationID" AS corp, NULLIF(p."allianceID", 0) AS alliance, p."shipTypeID" AS ship
              FROM "killLogParticipants" p JOIN recent r ON r."killID" = p."killID" WHERE p."ownerID" > 0
            UNION ALL
            SELECT "victimCharID", "victimCorpID", "victimAllianceID", "victimShipTypeID" FROM recent)
        SELECT (SELECT count(DISTINCT who) FROM people) AS characters,
               (SELECT count(DISTINCT corp) FROM people) AS corporations,
               (SELECT count(DISTINCT alliance) FROM people) AS alliances,
               (SELECT count(DISTINCT ship) FROM people) AS ships,
               (SELECT count(DISTINCT "solarSystemID") FROM recent) AS systems,
               (SELECT count(DISTINCT s."regionID") FROM recent r JOIN "mapSolarSystems" s ON s."solarSystemID" = r."solarSystemID") AS regions,
               (SELECT count(*) FROM recent) AS kills"#, &[&week]).await?;
    let top = |kind: &str| -> String {
        let (id, name, ticker, join) = match kind {
            "corporation" => (r#"p."corporationID""#, r#"co."corporationName""#, r#"co."tickerName""#,
                r#"LEFT JOIN "corporation" co ON co."corporationID" = p."corporationID""#),
            "alliance" => (r#"NULLIF(p."allianceID", 0)"#, r#"a."allianceName""#, r#"a."shortName""#,
                r#"LEFT JOIN alliances a ON a."allianceID" = p."allianceID""#),
            _ => (r#"p."ownerID""#, r#"c."characterName""#, r#"''"#,
                r#"LEFT JOIN "character_" c ON c."characterID" = p."ownerID""#),
        };
        format!(r#"SELECT {id} AS id, COALESCE({name}, 'Unknown') AS name, COALESCE({ticker}, '') AS ticker,
                          count(DISTINCT p."killID")::int AS kills
                     FROM "killLogParticipants" p JOIN "killLog" k ON k."killID" = p."killID" {join}
                    WHERE k."killTime" > $1 AND p."ownerID" > 0 AND {id} IS NOT NULL
                    GROUP BY 1, 2, 3 ORDER BY kills DESC, name LIMIT 10"#)
    };
    let top_characters = rows(&server.db.query(&top("character"), &[&week]).await?);
    let top_corporations = rows(&server.db.query(&top("corporation"), &[&week]).await?);
    let top_alliances = rows(&server.db.query(&top("alliance"), &[&week]).await?);
    let count = |name: &str| activity.get::<_, i64>(name);
    Ok(json!({ "total": total, "day": counts.get::<_, i64>("day"), "week": counts.get::<_, i64>("week"),
        "pods": counts.get::<_, i64>("pods"), "sides": sides, "ships": ships,
        "pilots": pilots, "featured": featured, "structures": structures,
        "activity": { "characters": count("characters"), "corporations": count("corporations"),
            "alliances": count("alliances"), "ships": count("ships"), "systems": count("systems"),
            "regions": count("regions"), "kills": count("kills") },
        "topCharacters": top_characters, "topCorporations": top_corporations, "topAlliances": top_alliances }))
}

/// What the sovereignty map draws but never changes: k-space systems as
/// [id, name, regionID, x, z, security] in light years from the galaxy's
/// centre, each stargate link once as [from, to], and the region names.
async fn sov_map_static(server: &Server) -> Answer {
    const LIGHT_YEAR: f64 = 9.460_730_472_580_8e15;
    let systems = server.db.query(r#"SELECT "solarSystemID", COALESCE("solarSystemName", ''), "regionID",
            COALESCE(x, 0)::float8, COALESCE(z, 0)::float8, COALESCE("security", 0)::float8
          FROM "mapSolarSystems" WHERE "regionID" < 11000000 ORDER BY "solarSystemID""#, &[]).await?;
    let round = |v: f64, places: i32| (v * 10f64.powi(places)).round() / 10f64.powi(places);
    let systems: Vec<Json> = systems.iter().map(|r| json!([
        r.get::<_, i32>(0), r.get::<_, String>(1), r.get::<_, Option<i32>>(2).unwrap_or(0),
        round(r.get::<_, f64>(3) / LIGHT_YEAR, 3), round(r.get::<_, f64>(4) / LIGHT_YEAR, 3),
        round(r.get::<_, f64>(5), 2),
    ])).collect();
    let jumps = server.db.query(r#"SELECT "fromSolarSystemID", "toSolarSystemID" FROM "mapSolarSystemJumps"
          WHERE "fromSolarSystemID" < "toSolarSystemID" AND "fromRegionID" < 11000000 AND "toRegionID" < 11000000"#, &[]).await?;
    let jumps: Vec<Json> = jumps.iter().map(|r| json!([r.get::<_, i32>(0), r.get::<_, i32>(1)])).collect();
    let regions = server.db.query(r#"SELECT "regionID", COALESCE("regionName", '') FROM "mapRegions" WHERE "regionID" < 11000000"#, &[]).await?;
    let regions: Vec<Json> = regions.iter().map(|r| json!([r.get::<_, i32>(0), r.get::<_, String>(1)])).collect();
    Ok(json!({ "systems": systems, "jumps": jumps, "regions": regions }))
}

/// Who holds what, refreshed as sovereignty resolves: each held system as
/// [systemID, allianceID, held since (ms)], the holding alliances, the latest
/// changes of hands and the sieges under way.
async fn sov_map(server: &Server) -> Answer {
    let since = ms(r#""sovereigntySince""#);
    let held = server.db.query(&format!(r#"SELECT "solarSystemID", "allianceID", {since}
          FROM "mapSolarSystems" WHERE "allianceID" IS NOT NULL AND "regionID" < 11000000"#), &[]).await?;
    let systems: Vec<Json> = held.iter().map(|r| json!([
        r.get::<_, i32>(0), r.get::<_, i32>(1), r.get::<_, Option<i64>>(2),
    ])).collect();
    let alliances = rows(&server.db.query(r#"SELECT a."allianceID" AS id, a."allianceName" AS name, a."shortName" AS ticker,
            count(*)::int AS systems,
            EXISTS (SELECT 1 FROM "botGroups" g WHERE g."ownerID" = a."allianceID") AS bots,
            (SELECT COALESCE(sum(co."memberCount"), 0)::int FROM "corporation" co
              WHERE co."allianceID" = a."allianceID" AND co."deleted" = 0) AS pilots
          FROM "mapSolarSystems" s JOIN alliances a ON a."allianceID" = s."allianceID"
         WHERE s."regionID" < 11000000
         GROUP BY a."allianceID", a."allianceName", a."shortName"
         ORDER BY systems DESC, name"#, &[]).await?);
    let recent = rows(&server.db.query(&format!(r#"SELECT s."solarSystemID" AS id, s."solarSystemName" AS system,
            r."regionName" AS region, a."allianceName" AS alliance, a."shortName" AS ticker, {since} AS since
          FROM "mapSolarSystems" s JOIN alliances a ON a."allianceID" = s."allianceID"
          JOIN "mapRegions" r ON r."regionID" = s."regionID"
         WHERE s."sovereigntySince" IS NOT NULL
         ORDER BY s."sovereigntySince" DESC, s."solarSystemName" LIMIT 15"#), &[]).await?);
    let sieges = server.world.lock().bot_stats.sieges.clone();
    let sieges: Vec<Json> = sieges.iter().map(|s| json!({
        "system": s.system, "attacker": s.attacker, "defender": s.defender,
        "starts": s.starts / 10_000 - UNIX_EPOCH_MS, "fighting": s.fleets.is_some(),
    })).collect();
    Ok(json!({ "systems": systems, "alliances": alliances, "recent": recent, "sieges": sieges, "now": unix_ms_now() }))
}

async fn kill_detail(server: &Server, id: i64) -> Answer {
    if id <= 0 { return Ok(json!({ "error": "Invalid kill ID" })); }
    let sql = format!(r#"SELECT k."killID" AS id, {time} AS time,
           k."victimCharID" AS "victimID", COALESCE(vc."characterName", 'Unknown pilot') AS victim,
           COALESCE(vco."corporationName", '') AS "victimCorporation",
           COALESCE(va."allianceName", '') AS "victimAlliance",
           COALESCE(t."typeName", 'Unknown ship') AS ship,
           k."victimShipTypeID" AS "shipTypeID", COALESCE(g."groupName", '') AS "shipClass",
           COALESCE(t."basePrice", 0)::float8 AS "hullValue",
           COALESCE(s."solarSystemName", 'Unknown system') AS system,
           COALESCE(s."security", 0)::float8 AS security,
           COALESCE(r."regionName", '') AS region,
           k."totalDamage" AS damage, k."isPodKill" = 1 AS pod,
           k."victimCorpID" AS "victimCorpID", NULLIF(k."victimAllianceID", 0) AS "victimAllianceID"
      FROM "killLog" k
      LEFT JOIN "character_" vc ON vc."characterID" = k."victimCharID"
      LEFT JOIN "corporation" vco ON vco."corporationID" = k."victimCorpID"
      LEFT JOIN alliances va ON va."allianceID" = k."victimAllianceID"
      LEFT JOIN "invTypes" t ON t."typeID" = k."victimShipTypeID"
      LEFT JOIN "invGroups" g ON g."groupID" = t."groupID"
      LEFT JOIN "mapSolarSystems" s ON s."solarSystemID" = k."solarSystemID"
      LEFT JOIN "mapRegions" r ON r."regionID" = s."regionID"
     WHERE k."killID" = $1"#, time = ms(r#"k."killTime""#));
    let Some(kill) = server.db.query_opt(sql.as_str(), &[&id]).await? else {
        return Ok(json!({ "error": "Kill not found" }));
    };
    let participants = rows(&server.db.query(
        r#"SELECT p."ownerID" AS id, COALESCE(c."characterName", npc."typeName", 'Unknown pilot') AS pilot,
                  p."ownerID" < 0 AS npc,
                  COALESCE(co."corporationName", '') AS corporation,
                  COALESCE(a."allianceName", '') AS alliance,
                  COALESCE(ship."typeName", '') AS ship, p."shipTypeID" AS "shipTypeID",
                  CASE WHEN COALESCE(p."weaponTypeID", 0) <= 0 THEN 'Unknown weapon'
                       ELSE COALESCE(weapon."typeName", 'Unknown weapon') END AS weapon,
                  p."damageDone" AS damage, p."finalBlow" = 1 AS "finalBlow",
                  p."corporationID" AS "corporationID", NULLIF(p."allianceID", 0) AS "allianceID"
             FROM "killLogParticipants" p
             LEFT JOIN "character_" c ON c."characterID" = p."ownerID"
             LEFT JOIN "corporation" co ON co."corporationID" = p."corporationID"
             LEFT JOIN alliances a ON a."allianceID" = p."allianceID"
             LEFT JOIN "invTypes" ship ON ship."typeID" = p."shipTypeID"
             LEFT JOIN "invTypes" weapon ON weapon."typeID" = p."weaponTypeID"
             LEFT JOIN "invTypes" npc ON npc."typeID" = -p."ownerID"
            WHERE p."killID" = $1 ORDER BY p."finalBlow" DESC, p."damageDone" DESC"#, &[&id]).await?);
    let items = rows(&server.db.query(r#"SELECT i."typeID" AS "typeID", t."typeName" AS name,
                 i."flag" AS flag, i."quantity" AS quantity, i."dropped" AS dropped,
                 COALESCE(t."basePrice",0)::float8 AS "unitValue"
            FROM "killLogItems" i JOIN "invTypes" t ON t."typeID"=i."typeID"
           WHERE i."killID"=$1 ORDER BY i."flag", i."typeID""#, &[&id]).await?);
    Ok(json!({ "kill": rows(&[kill]).as_array().and_then(|a| a.first()).cloned(), "participants": participants, "items": items }))
}

/// A kill's estimated value: the hull's base price and its recorded fitting
/// and cargo at base prices (`k` is the killLog alias).
const KILL_VALUE: &str = r#"(COALESCE(t."basePrice", 0) + COALESCE((SELECT sum(it."basePrice" * i."quantity")
        FROM "killLogItems" i JOIN "invTypes" it ON it."typeID" = i."typeID"
       WHERE i."killID" = k."killID"), 0))::float8"#;

/// The killboard's kill rows (zKillboard's list): newest first, 30 a page.
/// `filter` is a condition on `k` (the killLog row); `$1` is the offset and
/// any further parameters are the filter's.
fn kill_rows_sql(filter: &str) -> String {
    format!(
        r#"SELECT k."killID" AS id, {time} AS time, k."victimCharID" AS "victimID", vc."characterName" AS victim,
                  vco."corporationName" AS "victimCorporation", vco."tickerName" AS "victimCorpTicker",
                  va."allianceName" AS "victimAlliance", va."shortName" AS "victimAllianceTicker",
                  t."typeName" AS ship, k."victimShipTypeID" AS "shipTypeID", COALESCE(g."groupName", '') AS "shipClass",
                  COALESCE(t."basePrice",0)::float8 AS "hullValue", {value} AS "totalValue",
                  s."solarSystemName" AS system, COALESCE(s."security", 0)::float8 AS security, r."regionName" AS region,
                  k."finalBlowID" AS "finalBlowID",
                  COALESCE(fc."characterName", nt."typeName") AS "finalBlow", k."finalBlowID" < 0 AS "finalBlowNpc",
                  fco."corporationName" AS "finalBlowCorporation", fco."tickerName" AS "finalBlowCorpTicker",
                  fa."allianceName" AS "finalBlowAlliance", fa."shortName" AS "finalBlowAllianceTicker",
                  (SELECT count(*) FROM "killLogParticipants" p WHERE p."killID" = k."killID")::int AS involved,
                  k."isPodKill" = 1 AS pod,
                  k."victimCorpID" AS "victimCorpID", NULLIF(k."victimAllianceID", 0) AS "victimAllianceID",
                  fp."corporationID" AS "finalBlowCorpID", NULLIF(fp."allianceID", 0) AS "finalBlowAllianceID"
             FROM "killLog" k
             LEFT JOIN "character_" vc ON vc."characterID" = k."victimCharID"
             LEFT JOIN "corporation" vco ON vco."corporationID" = k."victimCorpID"
             LEFT JOIN alliances va ON va."allianceID" = NULLIF(k."victimAllianceID", 0)
             LEFT JOIN "invTypes" t ON t."typeID" = k."victimShipTypeID"
             LEFT JOIN "invGroups" g ON g."groupID" = t."groupID"
             LEFT JOIN "mapSolarSystems" s ON s."solarSystemID" = k."solarSystemID"
             LEFT JOIN "mapRegions" r ON r."regionID" = s."regionID"
             LEFT JOIN "character_" fc ON fc."characterID" = k."finalBlowID"
             LEFT JOIN "invTypes" nt ON nt."typeID" = -k."finalBlowID"
             LEFT JOIN LATERAL (SELECT p."corporationID", p."allianceID" FROM "killLogParticipants" p
                                 WHERE p."killID" = k."killID" AND p."finalBlow" = 1 LIMIT 1) fp ON true
             LEFT JOIN "corporation" fco ON fco."corporationID" = fp."corporationID"
             LEFT JOIN alliances fa ON fa."allianceID" = NULLIF(fp."allianceID", 0)
            WHERE {filter}
            ORDER BY k."killID" DESC LIMIT 30 OFFSET $1"#,
        time = ms(r#"k."killTime""#),
        value = KILL_VALUE,
    )
}

async fn kills(server: &Server, page: i64) -> Answer {
    let sql = kill_rows_sql("true");
    Ok(rows(&server.db.query(sql.as_str(), &[&(page * 30)]).await?))
}

/// A pilot's kills, losses or both ("all"), a page at a time.
async fn pilot_kills(server: &Server, id: i64, mode: &str, page: i64) -> Answer {
    if id <= 0 || id > i32::MAX as i64 { return Ok(json!([])); }
    let involved = r#"EXISTS (SELECT 1 FROM "killLogParticipants" p2 WHERE p2."killID" = k."killID" AND p2."ownerID" = $2)"#;
    let filter = match mode {
        "kills" => format!(r#"{involved} AND k."victimCharID" IS DISTINCT FROM $2"#),
        "losses" => r#"k."victimCharID" = $2"#.to_string(),
        _ => format!(r#"(k."victimCharID" = $2 OR {involved})"#),
    };
    let sql = kill_rows_sql(&filter);
    Ok(rows(&server.db.query(sql.as_str(), &[&(page * 30), &(id as i32)]).await?))
}

async fn pilot_search(server: &Server, query: &str) -> Answer {
    let q = query.trim();
    if q.chars().count() < 2 || q.chars().count() > 80 { return Ok(json!([])); }
    let pattern = format!("%{q}%");
    Ok(rows(&server.db.query(r#"SELECT c."characterID" AS id, c."characterName" AS name,
        co."corporationName" AS corporation, a."allianceName" AS alliance
        FROM "character_" c LEFT JOIN "corporation" co ON co."corporationID" = c."corporationID"
        LEFT JOIN alliances a ON a."allianceID" = co."allianceID"
        WHERE c."characterName" ILIKE $1 ORDER BY
        CASE WHEN lower(c."characterName") = lower($2) THEN 0 ELSE 1 END, c."characterName" LIMIT 30"#,
        &[&pattern, &q]).await?))
}

async fn pilot_profile(server: &Server, id: i64) -> Answer {
    if id <= 0 || id > i32::MAX as i64 { return Ok(json!({"error": "Invalid pilot ID"})); }
    let pid = id as i32;
    let sql = format!(r#"SELECT c."characterID" AS id,
        c."characterName" AS name, COALESCE(co."corporationName", '') AS corporation,
        COALESCE(co."tickerName", '') AS ticker, COALESCE(a."allianceName", '') AS alliance,
        COALESCE(a."shortName", '') AS "allianceTicker",
        c."corporationID" AS "corporationID", NULLIF(co."allianceID", 0) AS "allianceID",
        COALESCE(c."securityRating", 0)::float8 AS security,
        CASE WHEN c."createDateTime" > 0 THEN {born} END AS birthday,
        EXISTS (SELECT 1 FROM "bots" b WHERE b."characterID" = c."characterID") AS bot
        FROM "character_" c LEFT JOIN "corporation" co ON co."corporationID" = c."corporationID"
        LEFT JOIN alliances a ON a."allianceID" = co."allianceID"
        WHERE c."characterID" = $1"#, born = ms(r#"c."createDateTime""#));
    let Some(pilot) = server.db.query_opt(sql.as_str(), &[&pid]).await? else {
        return Ok(json!({"error": "Pilot not found"}));
    };
    // Ships destroyed and lost, their value, final blows and solo kills.
    let stats_sql = format!(r#"WITH mine AS (
            SELECT k."killID", k."victimCharID", k."isPodKill", {value} AS value,
                   (SELECT count(*) FROM "killLogParticipants" p WHERE p."killID" = k."killID") AS involved
              FROM "killLog" k LEFT JOIN "invTypes" t ON t."typeID" = k."victimShipTypeID"
             WHERE k."victimCharID" = $1
                OR EXISTS (SELECT 1 FROM "killLogParticipants" p WHERE p."killID" = k."killID" AND p."ownerID" = $1))
        SELECT count(*) FILTER (WHERE "victimCharID" IS DISTINCT FROM $1)::int8,
               count(*) FILTER (WHERE "victimCharID" = $1)::int8,
               COALESCE(sum(value) FILTER (WHERE "victimCharID" IS DISTINCT FROM $1), 0)::float8,
               COALESCE(sum(value) FILTER (WHERE "victimCharID" = $1), 0)::float8,
               count(*) FILTER (WHERE "victimCharID" IS DISTINCT FROM $1 AND involved = 1)::int8,
               COALESCE(avg(involved) FILTER (WHERE "victimCharID" IS DISTINCT FROM $1), 0)::float8,
               (SELECT count(*) FROM "killLogParticipants" WHERE "ownerID" = $1 AND "finalBlow" = 1)::int8,
               count(*) FILTER (WHERE "victimCharID" IS DISTINCT FROM $1 AND "isPodKill" = 1)::int8
          FROM mine"#, value = KILL_VALUE);
    let st = server.db.query_one(stats_sql.as_str(), &[&pid]).await?;
    let top_sql = format!(r#"SELECT k."killID" AS id, {time} AS time, k."victimCharID" AS "victimID",
        COALESCE(vc."characterName", 'Unknown pilot') AS victim, COALESCE(vco."corporationName", '') AS "victimCorporation",
        COALESCE(t."typeName", 'Unknown ship') AS ship, k."victimShipTypeID" AS "shipTypeID",
        COALESCE(s."solarSystemName", '') AS system, COALESCE(t."basePrice", 0)::float8 AS "hullValue", {value} AS "totalValue"
        FROM "killLog" k LEFT JOIN "character_" vc ON vc."characterID" = k."victimCharID"
        LEFT JOIN "corporation" vco ON vco."corporationID" = k."victimCorpID"
        LEFT JOIN "invTypes" t ON t."typeID" = k."victimShipTypeID"
        LEFT JOIN "mapSolarSystems" s ON s."solarSystemID" = k."solarSystemID"
        WHERE k."victimCharID" IS DISTINCT FROM $1
          AND EXISTS (SELECT 1 FROM "killLogParticipants" p WHERE p."killID" = k."killID" AND p."ownerID" = $1)
        ORDER BY "totalValue" DESC, k."killID" DESC LIMIT 6"#, time = ms(r#"k."killTime""#), value = KILL_VALUE);
    let top = rows(&server.db.query(top_sql.as_str(), &[&pid]).await?);
    // When the pilot fights: kills and losses of the last 90 days by weekday and hour.
    let since90 = crate::bulkdata::filetime_now() - 90 * 86_400 * 10_000_000;
    let since7 = crate::bulkdata::filetime_now() - 7 * 86_400 * 10_000_000;
    let activity = server.db.query(&format!(r#"SELECT extract(isodow FROM to_timestamp({t} / 1000.0) AT TIME ZONE 'UTC')::int AS day,
               extract(hour FROM to_timestamp({t} / 1000.0) AT TIME ZONE 'UTC')::int AS hour, count(*)::int AS n
          FROM "killLog" k
         WHERE k."killTime" >= $2 AND (k."victimCharID" = $1
            OR EXISTS (SELECT 1 FROM "killLogParticipants" p WHERE p."killID" = k."killID" AND p."ownerID" = $1))
         GROUP BY 1, 2"#, t = ms(r#"k."killTime""#)), &[&pid, &since90]).await?;
    let activity: Vec<Json> = activity.iter().map(|r| json!([r.get::<_, i32>(0), r.get::<_, i32>(1), r.get::<_, i32>(2)])).collect();
    let week = server.db.query_one(r#"SELECT count(DISTINCT p."shipTypeID")::int8, count(DISTINCT k."solarSystemID")::int8,
            count(DISTINCT s."regionID")::int8, count(DISTINCT k."killID")::int8
          FROM "killLogParticipants" p JOIN "killLog" k ON k."killID" = p."killID"
          LEFT JOIN "mapSolarSystems" s ON s."solarSystemID" = k."solarSystemID"
         WHERE p."ownerID" = $1 AND k."killTime" >= $2"#, &[&pid, &since7]).await?;
    let ships = rows(&server.db.query(r#"SELECT t."typeName" AS ship, p."shipTypeID" AS "typeID",
        count(DISTINCT p."killID")::int AS kills FROM "killLogParticipants" p
        JOIN "invTypes" t ON t."typeID" = p."shipTypeID" WHERE p."ownerID" = $1
        GROUP BY p."shipTypeID", t."typeName" ORDER BY kills DESC LIMIT 8"#, &[&pid]).await?);
    let systems = rows(&server.db.query(r#"SELECT s."solarSystemName" AS name, COALESCE(s."security", 0)::float8 AS security,
        count(DISTINCT k."killID")::int AS kills FROM "killLogParticipants" p
        JOIN "killLog" k ON k."killID" = p."killID" JOIN "mapSolarSystems" s ON s."solarSystemID" = k."solarSystemID"
        WHERE p."ownerID" = $1 GROUP BY s."solarSystemName", s."security" ORDER BY kills DESC LIMIT 5"#, &[&pid]).await?);
    let together = rows(&server.db.query(r#"SELECT o."ownerID" AS id, COALESCE(c."characterName", '') AS name,
        count(DISTINCT o."killID")::int AS kills FROM "killLogParticipants" p
        JOIN "killLogParticipants" o ON o."killID" = p."killID" AND o."ownerID" <> p."ownerID" AND o."ownerID" > 0
        JOIN "character_" c ON c."characterID" = o."ownerID"
        WHERE p."ownerID" = $1 GROUP BY o."ownerID", c."characterName" ORDER BY kills DESC LIMIT 5"#, &[&pid]).await?);
    Ok(json!({"pilot": rows(&[pilot]).as_array().and_then(|a| a.first()).cloned(),
        "destroyed": st.get::<_, i64>(0), "lost": st.get::<_, i64>(1),
        "iskDestroyed": st.get::<_, f64>(2), "iskLost": st.get::<_, f64>(3),
        "soloKills": st.get::<_, i64>(4), "avgInvolved": st.get::<_, f64>(5),
        "finalBlows": st.get::<_, i64>(6), "podKills": st.get::<_, i64>(7),
        "topKills": top, "activity": activity,
        "week": {"ships": week.get::<_, i64>(0), "systems": week.get::<_, i64>(1), "regions": week.get::<_, i64>(2), "kills": week.get::<_, i64>(3)},
        "ships": ships, "systems": systems, "together": together}))
}

/// A corporation or an alliance, for its killboard page.
#[derive(Clone, Copy, PartialEq)]
enum Group {
    Corporation,
    Alliance,
}

impl Group {
    /// The killLog column naming the victim's group, and the participants'.
    fn columns(self) -> (&'static str, &'static str) {
        match self {
            Group::Corporation => (r#"k."victimCorpID""#, r#""corporationID""#),
            Group::Alliance => (r#"k."victimAllianceID""#, r#""allianceID""#),
        }
    }

    /// (kills, losses, either) as conditions on `k`, with the group's ID as `$n`.
    fn filters(self, n: usize) -> (String, String, String) {
        let (victim, member) = self.columns();
        let involved = format!(r#"EXISTS (SELECT 1 FROM "killLogParticipants" gp WHERE gp."killID" = k."killID" AND gp.{member} = ${n})"#);
        let loss = format!("{victim} = ${n}");
        (format!("{involved} AND {victim} IS DISTINCT FROM ${n}"), loss.clone(), format!("({loss} OR {involved})"))
    }
}

async fn group_kills(server: &Server, kind: Group, id: i64, mode: &str, page: i64) -> Answer {
    if id <= 0 || id > i32::MAX as i64 { return Ok(json!([])); }
    let (kills, losses, all) = kind.filters(2);
    let filter = match mode { "kills" => kills, "losses" => losses, _ => all };
    let sql = kill_rows_sql(&filter);
    Ok(rows(&server.db.query(sql.as_str(), &[&(page * 30), &(id as i32)]).await?))
}

/// A corporation's or alliance's killboard page: who they are, ships and ISK
/// destroyed and lost, their best kills, when they fight, their top pilots.
async fn group_profile(server: &Server, kind: Group, id: i64) -> Answer {
    if id <= 0 || id > i32::MAX as i64 { return Ok(json!({"error": "Invalid ID"})); }
    let gid = id as i32;
    let info = match kind {
        Group::Corporation => server.db.query_opt(r#"SELECT co."corporationID" AS id, co."corporationName" AS name,
                co."tickerName" AS ticker, co."memberCount" AS members, COALESCE(co."description", '') AS description,
                co."ceoID" AS "ceoID", COALESCE(ceo."characterName", '') AS ceo,
                NULLIF(co."allianceID", 0) AS "allianceID", COALESCE(a."allianceName", '') AS alliance,
                COALESCE(a."shortName", '') AS "allianceTicker",
                EXISTS (SELECT 1 FROM "crpNPCCorporations" n WHERE n."corporationID" = co."corporationID") AS npc,
                (SELECT count(*) FROM "staStations" st WHERE st."corporationID" = co."corporationID" AND st."stationID" >= 61000000)::int AS outposts
              FROM "corporation" co
              LEFT JOIN "character_" ceo ON ceo."characterID" = co."ceoID"
              LEFT JOIN alliances a ON a."allianceID" = co."allianceID"
             WHERE co."corporationID" = $1"#, &[&gid]).await?,
        Group::Alliance => server.db.query_opt(&format!(r#"SELECT a."allianceID" AS id, a."allianceName" AS name,
                a."shortName" AS ticker, COALESCE(a."description", '') AS description,
                a."executorCorpID" AS "executorID", COALESCE(ex."corporationName", '') AS executor,
                (SELECT COALESCE(sum(c."memberCount"), 0) FROM "corporation" c WHERE c."allianceID" = a."allianceID" AND c."deleted" = 0)::int8 AS members,
                (SELECT count(*) FROM "corporation" c WHERE c."allianceID" = a."allianceID" AND c."deleted" = 0)::int8 AS corporations,
                (SELECT count(*) FROM "mapSolarSystems" s WHERE s."allianceID" = a."allianceID")::int8 AS systems,
                CASE WHEN a."startDate" > 0 THEN {founded} END AS founded
              FROM alliances a LEFT JOIN "corporation" ex ON ex."corporationID" = a."executorCorpID"
             WHERE a."allianceID" = $1"#, founded = ms(r#"a."startDate""#)), &[&gid]).await?,
    };
    let Some(info) = info else { return Ok(json!({"error": "Not found"})) };
    let (kills, losses, all) = kind.filters(1);
    let (_, member) = kind.columns();
    let stats_sql = format!(r#"WITH mine AS (
            SELECT k."killID", COALESCE({losses}, false) AS lost, k."isPodKill", {value} AS value,
                   (SELECT count(*) FROM "killLogParticipants" p WHERE p."killID" = k."killID") AS involved
              FROM "killLog" k LEFT JOIN "invTypes" t ON t."typeID" = k."victimShipTypeID"
             WHERE {all})
        SELECT count(*) FILTER (WHERE NOT lost)::int8, count(*) FILTER (WHERE lost)::int8,
               COALESCE(sum(value) FILTER (WHERE NOT lost), 0)::float8, COALESCE(sum(value) FILTER (WHERE lost), 0)::float8,
               count(*) FILTER (WHERE NOT lost AND involved = 1)::int8,
               COALESCE(avg(involved) FILTER (WHERE NOT lost), 0)::float8
          FROM mine"#, value = KILL_VALUE);
    let st = server.db.query_one(stats_sql.as_str(), &[&gid]).await?;
    let top_sql = format!(r#"SELECT k."killID" AS id, {time} AS time, k."victimCharID" AS "victimID",
        COALESCE(vc."characterName", 'Unknown pilot') AS victim, COALESCE(vco."corporationName", '') AS "victimCorporation",
        COALESCE(t."typeName", 'Unknown ship') AS ship, k."victimShipTypeID" AS "shipTypeID",
        COALESCE(s."solarSystemName", '') AS system, COALESCE(t."basePrice", 0)::float8 AS "hullValue", {value} AS "totalValue"
        FROM "killLog" k LEFT JOIN "character_" vc ON vc."characterID" = k."victimCharID"
        LEFT JOIN "corporation" vco ON vco."corporationID" = k."victimCorpID"
        LEFT JOIN "invTypes" t ON t."typeID" = k."victimShipTypeID"
        LEFT JOIN "mapSolarSystems" s ON s."solarSystemID" = k."solarSystemID"
        WHERE {kills} ORDER BY "totalValue" DESC, k."killID" DESC LIMIT 6"#, time = ms(r#"k."killTime""#), value = KILL_VALUE);
    let top = rows(&server.db.query(top_sql.as_str(), &[&gid]).await?);
    let since90 = crate::bulkdata::filetime_now() - 90 * 86_400 * 10_000_000;
    let since7 = crate::bulkdata::filetime_now() - 7 * 86_400 * 10_000_000;
    let activity = server.db.query(&format!(r#"SELECT extract(isodow FROM to_timestamp({t} / 1000.0) AT TIME ZONE 'UTC')::int,
               extract(hour FROM to_timestamp({t} / 1000.0) AT TIME ZONE 'UTC')::int, count(*)::int
          FROM "killLog" k WHERE k."killTime" >= $2 AND {all} GROUP BY 1, 2"#, t = ms(r#"k."killTime""#)), &[&gid, &since90]).await?;
    let activity: Vec<Json> = activity.iter().map(|r| json!([r.get::<_, i32>(0), r.get::<_, i32>(1), r.get::<_, i32>(2)])).collect();
    let week = server.db.query_one(&format!(r#"SELECT count(DISTINCT p."shipTypeID")::int8, count(DISTINCT k."solarSystemID")::int8,
            count(DISTINCT s."regionID")::int8, count(DISTINCT k."killID")::int8, count(DISTINCT p."ownerID")::int8
          FROM "killLogParticipants" p JOIN "killLog" k ON k."killID" = p."killID"
          LEFT JOIN "mapSolarSystems" s ON s."solarSystemID" = k."solarSystemID"
         WHERE p.{member} = $1 AND k."killTime" >= $2"#), &[&gid, &since7]).await?;
    let pilots = rows(&server.db.query(&format!(r#"SELECT p."ownerID" AS id, COALESCE(c."characterName", '') AS name,
        count(DISTINCT p."killID")::int AS kills FROM "killLogParticipants" p
        JOIN "character_" c ON c."characterID" = p."ownerID"
        WHERE p.{member} = $1 AND p."ownerID" > 0 GROUP BY p."ownerID", c."characterName" ORDER BY kills DESC LIMIT 8"#), &[&gid]).await?);
    let ships = rows(&server.db.query(&format!(r#"SELECT t."typeName" AS ship, p."shipTypeID" AS "typeID",
        count(DISTINCT p."killID")::int AS kills FROM "killLogParticipants" p
        JOIN "invTypes" t ON t."typeID" = p."shipTypeID" WHERE p.{member} = $1
        GROUP BY p."shipTypeID", t."typeName" ORDER BY kills DESC LIMIT 8"#), &[&gid]).await?);
    let systems = rows(&server.db.query(&format!(r#"SELECT s."solarSystemName" AS name, COALESCE(s."security", 0)::float8 AS security,
        count(DISTINCT k."killID")::int AS kills FROM "killLogParticipants" p
        JOIN "killLog" k ON k."killID" = p."killID" JOIN "mapSolarSystems" s ON s."solarSystemID" = k."solarSystemID"
        WHERE p.{member} = $1 GROUP BY s."solarSystemName", s."security" ORDER BY kills DESC LIMIT 5"#), &[&gid]).await?);
    let corporations = match kind {
        Group::Alliance => rows(&server.db.query(r#"SELECT c."corporationID" AS id, c."corporationName" AS name,
            c."tickerName" AS ticker, c."memberCount" AS members FROM "corporation" c
            WHERE c."allianceID" = $1 AND c."deleted" = 0 ORDER BY c."memberCount" DESC LIMIT 12"#, &[&gid]).await?),
        Group::Corporation => json!([]),
    };
    Ok(json!({"info": rows(&[info]).as_array().and_then(|a| a.first()).cloned(),
        "kind": if kind == Group::Alliance { "alliance" } else { "corporation" },
        "destroyed": st.get::<_, i64>(0), "lost": st.get::<_, i64>(1),
        "iskDestroyed": st.get::<_, f64>(2), "iskLost": st.get::<_, f64>(3),
        "soloKills": st.get::<_, i64>(4), "avgInvolved": st.get::<_, f64>(5),
        "topKills": top, "activity": activity,
        "week": {"ships": week.get::<_, i64>(0), "systems": week.get::<_, i64>(1), "regions": week.get::<_, i64>(2),
                 "kills": week.get::<_, i64>(3), "pilots": week.get::<_, i64>(4)},
        "pilots": pilots, "ships": ships, "systems": systems, "corporations": corporations}))
}

#[cfg(test)]
mod tests {
    use super::{client_text, filetime_at_ms, header, local_host};

    #[test]
    fn native_shutdown_timer_uses_windows_filetime() {
        assert_eq!(filetime_at_ms(0), 116_444_736_000_000_000);
        assert_eq!(filetime_at_ms(300_000), 116_444_739_000_000_000);
    }

    #[test]
    fn published_news_cannot_inject_markup_into_character_select() {
        assert_eq!(client_text("<script>alert(\"x\")</script> & Sol's"), "&lt;script&gt;alert(&quot;x&quot;)&lt;/script&gt; &amp; Sol's");
    }

    #[test]
    fn account_controls_accept_only_literal_loopback_hosts() {
        assert!(local_host("127.0.0.1:18080", 18080));
        assert!(local_host("localhost:18080", 18080));
        assert!(!local_host("evil.example:18080", 18080));
        assert!(!local_host("127.0.0.1:8080", 18080));
    }

    #[test]
    fn dashboard_header_matching_is_case_insensitive() {
        assert_eq!(header("POST / HTTP/1.1\r\nhOsT: 127.0.0.1:18080\r\nx-rmr-admin: 1\r\n\r\n", "X-RMR-Admin"), Some("1"));
    }
}
