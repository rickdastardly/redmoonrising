//! Exploration as Revelations brought it: cosmic signatures hidden in each
//! system, found with scanner probes.
//!
//! - **Signatures**: each system holds a few DED complexes (`dungeons`) of
//!   its pirate faction, rated for its security (high-sec 1 to 5 of 10,
//!   low-sec 3 to 8, null-sec 6 to 10), somewhere between 3 and 25 AU from
//!   its sun. They last one to three days; a completed one is gone, and new
//!   ones take their place.
//! - **Probes**: a Scan Probe Launcher, activated once, launches all the
//!   probes it holds, up to eight in space per pilot. They spread out round
//!   the ship in a pattern, each half its range from it, so together they
//!   cover the space round where they were launched; this client cannot move
//!   a probe once it is out, so to scan elsewhere, launch there. A probe
//!   lasts its lifetime (explosionDelay) and covers its scanRange (in AU)
//!   all round (dogmaIM Activate, `launch`).
//!   The scanner window lists them (beyonce GetScans) and is told of each
//!   new one (DoNewProbe).
//! - **Analysis**: CombineScans(probeIDs, groupID) with three or more probes,
//!   for Cosmic Signature: every signature inside all of their ranges is a
//!   result, placed within the probes' deviation of where it really is: from
//!   maxScanDeviation with three probes, shrinking with each one more down to
//!   minScanDeviation. Harder complexes are harder to pin down.
//! - **Warping**: WarpToStuff('scan', resultID) goes to the result. One
//!   within 150 km of the truth leads to the complex itself, which opens
//!   there; a rougher one only to where the result says.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use eve_marshal::Value;
use tracing::{info, warn};

use crate::ballpark::Vec3;
use crate::combat::Rng;
use crate::services::{Call, Ctx, Outcome};
use crate::svc::rows::key_val;

const AU: f64 = 149_597_870_700.0;
/// const.groupCosmicSignature, the only group with results here.
const GROUP_COSMIC_SIGNATURE: i64 = 502;
/// The most probes a pilot can have out at once.
const MAX_PROBES: usize = 8;
/// useMissiles, the launcher's effect.
const EFFECT_USE_MISSILES: i64 = 101;
/// A result this close to the truth leads to the complex itself.
const WARPABLE: f64 = 150_000.0;
/// How close a bookmark warp must land to a signature to open it.
const NEAR_SIGNATURE: f64 = 200_000.0;
/// A day in FILETIME units (100 ns).
const DAY: i64 = 864_000_000_000;

#[derive(Debug, Clone)]
struct Signature {
    id: i64,
    dungeon: String,
    rating: u8,
    pos: Vec3,
    expires: i64,
    /// The complex's site, once someone has found their way there.
    site: Option<i64>,
    /// Given to this pilot from the dashboard: theirs to warp to, unscanned.
    owner: Option<i64>,
}

#[derive(Debug, Clone)]
struct Probe {
    id: i64,
    type_id: i64,
    system: i64,
    pos: Vec3,
    range_au: f64,
    min_dev: f64,
    max_dev: f64,
    expires: i64,
}

#[derive(Debug, Clone)]
struct ScanResult {
    id: i64,
    signal: f64,
    pos: Vec3,
    accuracy: f64,
    /// What the scanner lists: the complex's name and rating once the result
    /// is pinned down to warp to, "Cosmic Signature" until then.
    name: String,
}

/// "Pith's Penal Complex (DED 8/10)", or the dungeon's ID if it is not in
/// the catalogue.
fn site_name(dungeon: &str, rating: u8) -> String {
    let name = crate::dungeons::find(dungeon).map_or_else(|| dungeon.to_string(), |d| d.name.clone());
    if rating > 0 { format!("{name} (DED {rating}/10)") } else { name }
}

#[derive(Default)]
struct State {
    /// Each system's signatures, once someone has scanned there.
    signatures: HashMap<i64, Vec<Signature>>,
    /// Each character's probes.
    probes: HashMap<i64, Vec<Probe>>,
    /// Each character's last analysis.
    results: HashMap<i64, Vec<ScanResult>>,
}

static STATE: OnceLock<Mutex<State>> = OnceLock::new();

fn state() -> MutexGuard<'static, State> {
    STATE.get_or_init(|| Mutex::new(State::default())).lock().unwrap_or_else(|e| e.into_inner())
}

/// A launched probe's figures, read from its charge.
pub struct ProbeCharge {
    /// The launcher.
    pub module_id: i64,
    /// Its loaded probes.
    pub item_id: i64,
    pub type_id: i64,
    pub range_au: f64,
    pub life_ms: f64,
    pub min_dev: f64,
    pub max_dev: f64,
}

fn probe_value(p: &Probe) -> Value {
    key_val(vec![
        ("probeID", Value::Int(p.id)),
        ("typeID", Value::Int(p.type_id)),
        ("expiry", Value::Int(p.expires)),
        ("pos", Value::Tuple(vec![Value::Double(p.pos.x), Value::Double(p.pos.y), Value::Double(p.pos.z)])),
        ("scanRange", Value::Double(p.range_au)),
    ])
}

/// Where the n-th of a launch's probes goes, as a direction from the ship:
/// the first at the ship, the rest round it at the corners of a cube.
fn spread(n: usize) -> Vec3 {
    const CORNERS: [(f64, f64, f64); 8] =
        [(0.0, 0.0, 0.0), (1.0, 1.0, 1.0), (-1.0, -1.0, 1.0), (-1.0, 1.0, -1.0), (1.0, -1.0, -1.0), (1.0, 0.0, 0.0), (-1.0, 0.0, 0.0), (0.0, 0.0, 1.0)];
    let (x, y, z) = CORNERS[n % CORNERS.len()];
    let v = Vec3::new(x, y, z);
    if v.len() > 0.0 { v.unit() } else { v }
}

/// The launcher's module light: on, then off, as one cycle.
fn launcher_events(char_id: i64, ship_id: i64, charge: &ProbeCharge) -> Vec<Value> {
    let now = crate::combat::now_ms();
    [true, false]
        .into_iter()
        .map(|start| {
            let environment = Value::List(vec![
                Value::Int(charge.module_id),
                Value::Int(char_id),
                Value::Int(ship_id),
                Value::None,
                Value::Int(charge.item_id),
                Value::List(Vec::new()),
                Value::Int(EFFECT_USE_MISSILES),
            ]);
            Value::Tuple(vec![
                Value::Int(charge.module_id),
                Value::Int(EFFECT_USE_MISSILES),
                Value::Long(crate::combat::filetime(now)),
                Value::Int(i64::from(start)),
                Value::Int(i64::from(start)),
                environment,
                Value::Long(crate::combat::filetime(now)),
                Value::Int(1_000),
                Value::Int(0),
                Value::None,
                Value::None,
            ])
        })
        .collect()
}

/// A Scan Probe Launcher fires: all its probes at once, as many as the
/// pilot may still have out, spread round the ship.
pub async fn launch(ctx: &Ctx<'_>, charge: ProbeCharge) -> Outcome {
    let (Some(char_id), Some(system), Some(ship_id)) = (ctx.session.char_id, ctx.session.solar_system_id, ctx.session.ship_id) else {
        return Outcome::custom("Probes can only be launched in space.");
    };
    let Some(pos) = ctx.world.lock().parks.get(&system).and_then(|p| p.ships.get(&ship_id)).map(|s| s.pos) else {
        return Outcome::custom("Your ship is not in space.");
    };
    let now = crate::bulkdata::filetime_now();
    let out = {
        let mut st = state();
        let list = st.probes.entry(char_id).or_default();
        list.retain(|p| p.expires > now);
        list.len()
    };
    let room = MAX_PROBES.saturating_sub(out);
    if room == 0 {
        return Outcome::custom(format!("You already have {MAX_PROBES} probes out, the most you can control. Wait for them to expire."));
    }
    // The charges used: as many as there are, up to the room left.
    let used = async {
        let mut client = crate::db::connect(ctx.db_url).await?;
        let mut txn = crate::items::ItemTxn::begin(&mut client).await?;
        let Some(item) = txn.item(charge.item_id).await? else { return Ok(None) };
        let n = (item.quantity.max(0) as usize).min(room);
        if n == 0 {
            return Ok(None);
        }
        if item.quantity > n as i64 {
            txn.set_quantity(item.id, item.quantity - n as i64).await?;
        } else {
            txn.destroy(item.id).await?;
        }
        Ok::<_, Box<dyn std::error::Error + Send + Sync>>(Some((n, txn.commit().await?)))
    }
    .await;
    let count = match used {
        Ok(Some((n, events))) => {
            crate::items::notify(ctx.world, ctx.client_id, events);
            n
        }
        Ok(None) => return Outcome::custom("The launcher has no probes loaded."),
        Err(e) => {
            warn!("launching probes: {e}");
            return Outcome::custom("The probes could not be launched.");
        }
    };
    let range_au = charge.range_au.max(0.1);
    let probes: Vec<Probe> = (0..count)
        .map(|n| Probe {
            id: crate::npc::next_id(),
            type_id: charge.type_id,
            system,
            pos: pos + spread(n) * (range_au * AU * 0.5),
            range_au,
            min_dev: charge.min_dev.max(0.0),
            max_dev: charge.max_dev.max(charge.min_dev).max(0.0),
            expires: now + (charge.life_ms.max(60_000.0) * 10_000.0) as i64,
        })
        .collect();
    let values: Vec<Value> = probes.iter().map(probe_value).collect();
    state().probes.entry(char_id).or_default().extend(probes);
    {
        let w = ctx.world.lock();
        for event in launcher_events(char_id, ship_id, &charge) {
            w.notify(&[ctx.client_id], "OnGodmaShipEffect", event);
        }
        for value in values {
            w.notify(&[ctx.client_id], "DoNewProbe", Value::Tuple(vec![value]));
        }
    }
    info!("{char_id} launched {count} probe(s) of type {} ({range_au} AU) in {system}", charge.type_id);
    Outcome::Value(Value::None)
}

/// beyonce GetScans: the caller's probes here, and their last analysis.
pub fn get_scans(ctx: &Ctx<'_>) -> Value {
    let (Some(char_id), Some(system)) = (ctx.session.char_id, ctx.session.solar_system_id) else {
        return scans_value(Vec::new(), Vec::new());
    };
    let now = crate::bulkdata::filetime_now();
    let mut st = state();
    let probes: Vec<Probe> = match st.probes.get_mut(&char_id) {
        Some(list) => {
            list.retain(|p| p.expires > now);
            list.iter().filter(|p| p.system == system).cloned().collect()
        }
        None => Vec::new(),
    };
    let results = st.results.get(&char_id).cloned().unwrap_or_default();
    scans_value(probes, results)
}

fn scans_value(probes: Vec<Probe>, results: Vec<ScanResult>) -> Value {
    let probes = Value::Dict(probes.iter().map(|p| (Value::Int(p.id), probe_value(p))).collect());
    let results = Value::List(
        results
            .iter()
            .map(|r| {
                Value::Tuple(vec![
                    Value::Int(r.id),
                    Value::text(format!("{:.0}%", r.signal)),
                    Value::Tuple(vec![Value::Double(r.pos.x), Value::Double(r.pos.y), Value::Double(r.pos.z)]),
                    Value::Double(r.accuracy),
                    // Read by the launcher's scanner hook (Start.bat), which
                    // lists it in place of the result ID.
                    Value::text(r.name.clone()),
                ])
            })
            .collect(),
    );
    Value::Dict(vec![(Value::bytes("probes"), probes), (Value::bytes("results"), results)])
}

/// beyonce CombineScans(probeIDs, groupID): an analysis with these probes.
pub async fn combine(ctx: &Ctx<'_>, call: &Call) -> Value {
    let ids: Vec<i64> = call.args.first().and_then(|v| v.seq()).map(|s| s.iter().filter_map(|v| v.as_i64()).collect()).unwrap_or_default();
    let group = call.arg_i64(1).unwrap_or(0);
    let (Some(char_id), Some(system)) = (ctx.session.char_id, ctx.session.solar_system_id) else { return Value::None };
    ensure_signatures(ctx, system).await;
    let now = crate::bulkdata::filetime_now();
    let mut rng = Rng::seeded(now as u64 ^ char_id as u64);
    let refresh = {
        let mut st = state();
        let probes: Vec<Probe> = st
            .probes
            .get(&char_id)
            .map(|l| l.iter().filter(|p| ids.contains(&p.id) && p.system == system && p.expires > now).cloned().collect())
            .unwrap_or_default();
        let mut results = Vec::new();
        if probes.len() >= 3 && group == GROUP_COSMIC_SIGNATURE {
            let n = probes.len() as i32;
            let min_dev = probes.iter().map(|p| p.min_dev).sum::<f64>() / probes.len() as f64;
            let max_dev = probes.iter().map(|p| p.max_dev).sum::<f64>() / probes.len() as f64;
            let spread = (max_dev * 0.55f64.powi(n - 3)).max(min_dev);
            for sig in st.signatures.get(&system).map(Vec::as_slice).unwrap_or(&[]) {
                if sig.expires <= now || !probes.iter().all(|p| (p.pos - sig.pos).len() <= p.range_au * AU) {
                    continue;
                }
                let accuracy = spread * (1.0 + f64::from(sig.rating) * 0.06);
                let off = rng.in_sphere(1.0);
                let pos = if accuracy > 0.0 { sig.pos + off * accuracy } else { sig.pos };
                let signal = (100.0 - f64::from(sig.rating) * 7.0 + rng.range(-4.0, 4.0)).clamp(5.0, 100.0);
                let name = if accuracy <= WARPABLE { site_name(&sig.dungeon, sig.rating) } else { "Cosmic Signature".to_string() };
                results.push(ScanResult { id: sig.id, signal, pos, accuracy, name });
            }
        }
        // Complexes given to this pilot stay listed, whatever they scan.
        let given: Vec<i64> = st.signatures.values().flatten().filter(|s| s.owner == Some(char_id)).map(|s| s.id).collect();
        if let Some(old) = st.results.get(&char_id) {
            results.extend(old.iter().filter(|r| given.contains(&r.id) && !results.iter().any(|n| n.id == r.id)).cloned().collect::<Vec<_>>());
        }
        info!("{char_id} analysed with {} probe(s) for group {group} in {system}: {} result(s)", probes.len(), results.len());
        st.results.insert(char_id, results);
        probes.first().map(probe_value)
    };
    // The scanner window refreshes when told of a probe.
    if let Some(probe) = refresh {
        ctx.world.lock().notify(&[ctx.client_id], "DoNewProbe", Value::Tuple(vec![probe]));
    }
    Value::None
}

/// beyonce GiveProbe(charID, probeID): hands a probe to another pilot.
pub fn give(ctx: &Ctx<'_>, call: &Call) -> Value {
    let (Some(me), Some(to), Some(id)) = (ctx.session.char_id, call.arg_i64(0), call.arg_i64(1)) else { return Value::None };
    let mut st = state();
    let probe = st.probes.get_mut(&me).and_then(|l| l.iter().position(|p| p.id == id).map(|i| l.remove(i)));
    if let Some(p) = probe {
        st.probes.entry(to).or_default().push(p);
    }
    Value::None
}

/// WarpToStuff('scan', resultID): where the result leads, opening the
/// complex if the result is good enough to find it.
pub async fn warp_target(ctx: &Ctx<'_>, id: i64) -> Option<(Vec3, f64, i64)> {
    let char_id = ctx.session.char_id?;
    let system = ctx.session.solar_system_id?;
    let (result, sig) = {
        let st = state();
        let result = st.results.get(&char_id)?.iter().find(|r| r.id == id)?.clone();
        let sig = st.signatures.get(&system).and_then(|l| l.iter().find(|s| s.id == id)).cloned();
        (result, sig)
    };
    let Some(sig) = sig.filter(|_| result.accuracy <= WARPABLE) else {
        return Some((result.pos, 0.0, 0));
    };
    open_signature(ctx, system, char_id, &sig).await;
    Some((sig.pos, 0.0, 0))
}

/// A warp to a bookmark or point: opens any complex there that this pilot
/// has found or was given, so its bookmark leads into it too.
pub async fn open_near(ctx: &Ctx<'_>, at: Vec3) {
    let (Some(char_id), Some(system)) = (ctx.session.char_id, ctx.session.solar_system_id) else { return };
    let now = crate::bulkdata::filetime_now();
    let found: Vec<Signature> = {
        let st = state();
        let mine = st.results.get(&char_id);
        st.signatures
            .get(&system)
            .map(|l| {
                l.iter()
                    .filter(|s| s.expires > now && (s.pos - at).len() <= NEAR_SIGNATURE)
                    .filter(|s| s.owner == Some(char_id) || mine.is_some_and(|r| r.iter().any(|r| r.id == s.id && r.accuracy <= WARPABLE)))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    };
    for sig in found {
        open_signature(ctx, system, char_id, &sig).await;
    }
}

/// Builds the complex at a signature unless it is already open.
async fn open_signature(ctx: &Ctx<'_>, system: i64, char_id: i64, sig: &Signature) {
    let id = sig.id;
    let open = sig.site.is_some_and(|site| ctx.world.lock().parks.get(&system).is_some_and(|p| p.sites.contains_key(&site)));
    if open {
        return;
    }
    let Some(def) = crate::dungeons::find(&sig.dungeon) else {
        warn!("signature {id} names unknown complex {}", sig.dungeon);
        return;
    };
    let rats = match crate::npc::load_dungeon_rats(ctx.db, &def.faction, &def.ship_names(), &def.name_classes()).await {
        Ok(Some(r)) => r,
        Ok(None) => {
            warn!("opening {}: no ships for {}", def.id, def.faction);
            return;
        }
        Err(e) => {
            warn!("opening {}: {}", def.id, crate::logging::chain(&e));
            return;
        }
    };
    let site = {
        let mut w = ctx.world.lock();
        let opened = w.parks.get_mut(&system).map(|park| crate::dungeons::open(park, Arc::clone(&def), Arc::new(rats), sig.pos, char_id, 0, crate::combat::now_ms()));
        match opened {
            Some((site, nearby)) => {
                crate::svc::space::notify_targeted_updates(&w, nearby);
                Some(site)
            }
            None => None,
        }
    };
    if let Some(site) = site {
        info!("{char_id} opened {} (signature {id}) in {system} as site {site}", def.id);
        let mut st = state();
        if let Some(s) = st.signatures.get_mut(&system).and_then(|l| l.iter_mut().find(|s| s.id == id)) {
            s.site = Some(site);
        }
    }
}

/// A complex was completed: its signature is spent.
pub fn site_done(site_id: i64) {
    let mut st = state();
    for list in st.signatures.values_mut() {
        list.retain(|s| s.site != Some(site_id));
    }
}

/// The system's signatures, made (again) where there are none left.
async fn ensure_signatures(ctx: &Ctx<'_>, system: i64) {
    let now = crate::bulkdata::filetime_now();
    {
        let mut st = state();
        if let Some(list) = st.signatures.get_mut(&system) {
            list.retain(|s| s.expires > now);
            if list.iter().any(|s| s.owner.is_none()) {
                return;
            }
        }
    }
    let row = ctx
        .db
        .query_opt(
            r#"SELECT COALESCE(s."security", 0)::float8, COALESCE(s."factionID", r."factionID")
                 FROM "mapSolarSystems" s LEFT JOIN "mapRegions" r ON r."regionID" = s."regionID"
                WHERE s."solarSystemID" = $1"#,
            &[&(system as i32)],
        )
        .await
        .ok()
        .flatten();
    let (security, faction): (f64, Option<i32>) = row.map_or((1.0, None), |r| (r.get(0), r.get(1)));
    let key = crate::npc::pirate_key(faction.map(i64::from), system);
    let (lo, hi, count) = if security >= 0.45 {
        (1, 5, (1, 2))
    } else if security > 0.0 {
        (3, 8, (2, 3))
    } else {
        (6, 10, (2, 4))
    };
    let pool = crate::dungeons::pool(key, lo, hi);
    if pool.is_empty() {
        return;
    }
    let mut rng = Rng::seeded(now as u64 ^ system as u64);
    let n = count.0 + rng.below(count.1 - count.0 + 1);
    let made: Vec<Signature> = (0..n)
        .map(|_| {
            let def = &pool[rng.below(pool.len())];
            let dir = rng.in_sphere(1.0);
            let dir = if dir.len() > 0.1 { dir.unit() } else { Vec3::new(1.0, 0.0, 0.0) };
            let pos = Vec3::new(dir.x, dir.y * 0.15, dir.z) * (rng.range(3.0, 25.0) * AU);
            Signature {
                id: crate::npc::next_id(),
                dungeon: def.id.clone(),
                rating: def.rating,
                pos,
                expires: now + (rng.range(1.0, 3.0) * DAY as f64) as i64,
                site: None,
                owner: None,
            }
        })
        .collect();
    info!("system {system} ({security:.1}, {key}): {} signature(s): {}", made.len(), made.iter().map(|s| s.dungeon.as_str()).collect::<Vec<_>>().join(", "));
    state().signatures.entry(system).or_default().extend(made);
}

/// A complex given to a pilot (from the dashboard): a signature already
/// found, pinned exactly, so their system scanner lists it with no probes out
/// and Warp To takes them straight in. `site` is set when it is already open.
pub fn grant(char_id: i64, system: i64, dungeon: &str, rating: u8, pos: Vec3, site: Option<i64>) -> i64 {
    let now = crate::bulkdata::filetime_now();
    let id = crate::npc::next_id();
    let mut st = state();
    st.signatures.entry(system).or_default().push(Signature {
        id,
        dungeon: dungeon.to_string(),
        rating,
        pos,
        expires: now + 3 * DAY,
        site,
        owner: Some(char_id),
    });
    st.results.entry(char_id).or_default().push(ScanResult { id, signal: 100.0, pos, accuracy: 0.0, name: site_name(dungeon, rating) });
    id
}
