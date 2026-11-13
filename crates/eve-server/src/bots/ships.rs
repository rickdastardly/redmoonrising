//! What bots fly: a ladder of hull classes for each role, climbed as a
//! bot's wallet grows, and the modules a hull of each class is fitted with.
//!
//! - Fighters (ratters and pirates) go frigate, destroyer, cruiser,
//!   battlecruiser, battleship; explorers stop at cruisers; miners go
//!   frigate, mining barge, exhumer; haulers climb the industrials by hold
//!   size; traders fly shuttles, then industrials.
//! - A bot moves up a rung when it can buy the next hull several times
//!   over (`upgrade`), and after a loss buys the hull it lost again if it
//!   can, or the best it can afford (`rebuy`). Hulls come off the market
//!   the bots supply (`economy::buy_hull`).
//! - A fitting is real module types, the same for the same bot and hull:
//!   guns of its race and size in the turret slots, a propulsion module and
//!   a tank in the mids and lows, mining lasers for miners, and ammunition.
//!   It is what a dead bot's wreck holds and its killmail lists.

use std::collections::HashMap;

use tokio_postgres::Client;

use super::behaviour::Role;
use crate::combat::Rng;
use crate::svc::rows::{float, int, text};

const ATTR_LOW_SLOTS: i32 = 12;
const ATTR_MED_SLOTS: i32 = 13;
const ATTR_HI_SLOTS: i32 = 14;
const ATTR_CAPACITY: i32 = 38;
const ATTR_TURRET_SLOTS: i32 = 102;
const ATTR_CHARGE_SIZE: i32 = 128;
const ATTR_META_LEVEL: i32 = 633;
const ATTR_LAUNCHER_SLOTS: i32 = 101;
const ATTR_DRONE_CAPACITY: i32 = 283;
const ATTR_CPU_OUTPUT: i32 = 48;
const ATTR_POWER_OUTPUT: i32 = 11;
const ATTR_CPU: i32 = 50;
const ATTR_POWER: i32 = 30;
/// Bots have trained the fitting skills (CPU Management, Engineering).
const FITTING_SKILLS: f64 = 1.25;

pub const GROUP_FRIGATE: i32 = 25;
pub const GROUP_CRUISER: i32 = 26;
pub const GROUP_BATTLESHIP: i32 = 27;
pub const GROUP_INDUSTRIAL: i32 = 28;
pub const GROUP_SHUTTLE: i32 = 31;
pub const GROUP_BATTLECRUISER: i32 = 419;
pub const GROUP_DESTROYER: i32 = 420;
pub const GROUP_BARGE: i32 = 463;
pub const GROUP_EXHUMER: i32 = 543;
const HULL_GROUPS: [i32; 9] =
    [GROUP_FRIGATE, GROUP_CRUISER, GROUP_BATTLESHIP, GROUP_INDUSTRIAL, GROUP_SHUTTLE, GROUP_BATTLECRUISER, GROUP_DESTROYER, GROUP_BARGE, GROUP_EXHUMER];

const RACE_CALDARI: i64 = 1;
const RACE_MINMATAR: i64 = 2;
const RACE_AMARR: i64 = 4;

// Module groups (this client's invGroups).
const ENERGY_WEAPON: i32 = 53;
const PROJECTILE_WEAPON: i32 = 55;
const HYBRID_WEAPON: i32 = 74;
const MINING_LASER: i32 = 54;
const STRIP_MINER: i32 = 464;
const PROJECTILE_AMMO: i32 = 83;
const HYBRID_CHARGE: i32 = 85;
const FREQUENCY_CRYSTAL: i32 = 86;
const LAUNCHER_CRUISE: i32 = 506;
const LAUNCHER_ROCKET: i32 = 507;
const LAUNCHER_SIEGE: i32 = 508;
const LAUNCHER_STANDARD: i32 = 509;
const LAUNCHER_HEAVY: i32 = 510;
const LAUNCHER_CITADEL: i32 = 524;
const CITADEL_TORPEDO: i32 = 476;
const TORPEDO: i32 = 89;
const LIGHT_MISSILE: i32 = 384;
const HEAVY_MISSILE: i32 = 385;
const CRUISE_MISSILE: i32 = 386;
const ROCKET: i32 = 387;
const SCAN_PROBE_LAUNCHER: i32 = 481;
const SCANNER_PROBE: i32 = 479;
const ENERGY_VAMPIRE: i32 = 68;
const SHIELD_EXTENDER: i32 = 38;
const SHIELD_BOOSTER: i32 = 40;
const SHIELD_HARDENER: i32 = 77;
const AFTERBURNER: i32 = 46;
const MICROWARPDRIVE: i32 = 475;
const STASIS_WEB: i32 = 65;
const WARP_SCRAMBLER: i32 = 52;
const CAP_RECHARGER: i32 = 43;
const DAMAGE_CONTROL: i32 = 60;
const ARMOR_REPAIRER: i32 = 62;
const ARMOR_PLATE: i32 = 329;
const ARMOR_HARDENER: i32 = 328;
const GYROSTABILIZER: i32 = 59;
const HEAT_SINK: i32 = 205;
const MAGNETIC_STABILIZER: i32 = 302;
const BALLISTIC_CONTROL: i32 = 367;
const MINING_UPGRADE: i32 = 546;
pub const COMBAT_DRONE: i32 = 100;
const MINING_DRONE: i32 = 101;
const MODULE_GROUPS: [i32; 40] = [
    ENERGY_WEAPON, PROJECTILE_WEAPON, HYBRID_WEAPON, MINING_LASER, STRIP_MINER, PROJECTILE_AMMO, HYBRID_CHARGE, FREQUENCY_CRYSTAL,
    LAUNCHER_CRUISE, LAUNCHER_ROCKET, LAUNCHER_SIEGE, LAUNCHER_STANDARD, LAUNCHER_HEAVY, TORPEDO, LIGHT_MISSILE, HEAVY_MISSILE,
    CRUISE_MISSILE, ROCKET, SCAN_PROBE_LAUNCHER, SCANNER_PROBE, ENERGY_VAMPIRE, SHIELD_EXTENDER, SHIELD_BOOSTER, SHIELD_HARDENER,
    AFTERBURNER, MICROWARPDRIVE, STASIS_WEB, WARP_SCRAMBLER, CAP_RECHARGER, DAMAGE_CONTROL, ARMOR_REPAIRER, ARMOR_PLATE,
    ARMOR_HARDENER, GYROSTABILIZER, HEAT_SINK, MAGNETIC_STABILIZER, BALLISTIC_CONTROL, MINING_UPGRADE, COMBAT_DRONE, MINING_DRONE,
];

/// A hull a bot can fly.
#[derive(Debug, Clone)]
pub struct Hull {
    pub type_id: i32,
    pub group: i32,
    pub race: i64,
    /// What it costs on the market, before the bots' own supply moves it.
    pub price: f64,
    /// Its hold, m3.
    pub capacity: f64,
    pub slots: (u8, u8, u8),
    pub turrets: u8,
    pub launchers: u8,
    /// Drone bay, m3.
    pub drone_bay: f64,
    /// CPU (tf) and powergrid (MW) before skills.
    pub cpu: f64,
    pub power: f64,
}

/// A module, charge or drone a fitting may hold.
#[derive(Debug, Clone)]
struct Module {
    type_id: i32,
    group: i32,
    /// Turret and charge size (1 small to 4 capital); 0 for the rest.
    size: i32,
    meta: i32,
    name: String,
    cpu: f64,
    power: f64,
    /// m3: what a drone takes of the bay.
    volume: f64,
    race: i64,
}

pub struct Shipyard {
    pub hulls: HashMap<i32, Hull>,
    /// Hulls by (group, race), cheapest first.
    by_group_race: HashMap<(i32, i64), Vec<i32>>,
    modules: Vec<Module>,
}

/// The rungs of a role's ladder: the hull groups of each tier.
pub fn ladder(role: Role) -> &'static [i32] {
    match role {
        Role::Ratter | Role::Pirate => &[GROUP_FRIGATE, GROUP_DESTROYER, GROUP_CRUISER, GROUP_BATTLECRUISER, GROUP_BATTLESHIP],
        Role::Explorer => &[GROUP_FRIGATE, GROUP_DESTROYER, GROUP_CRUISER],
        Role::Miner => &[GROUP_FRIGATE, GROUP_BARGE, GROUP_EXHUMER],
        // Industrials, small to big: the tier picks a third of them by hold.
        Role::Hauler => &[GROUP_INDUSTRIAL, GROUP_INDUSTRIAL, GROUP_INDUSTRIAL],
        Role::Trader => &[GROUP_SHUTTLE, GROUP_INDUSTRIAL],
    }
}

/// The class a hull's group stands for in fights: frigate 0 to battleship 4.
pub fn weight_class(group: i32) -> u8 {
    match group {
        GROUP_DESTROYER => 1,
        GROUP_CRUISER | GROUP_BARGE | GROUP_EXHUMER | GROUP_INDUSTRIAL => 2,
        GROUP_BATTLECRUISER => 3,
        GROUP_BATTLESHIP => 4,
        _ => 0,
    }
}

/// The market groups under the market's Ships tab, for a query's WITH
/// RECURSIVE: bots fly only what a player could buy there.
pub const SHIPS_MARKET: &str = r#"ships_market AS (
        SELECT "marketGroupID" FROM "invMarketGroups" WHERE "parentGroupID" IS NULL AND "marketGroupName" = 'Ships'
        UNION SELECT g."marketGroupID" FROM "invMarketGroups" g JOIN ships_market s ON g."parentGroupID" = s."marketGroupID")"#;

/// Every hull type the ladders draw on: published, raced, and sold under
/// the market's Ships tab.
pub async fn hull_types(db: &Client) -> Result<Vec<i32>, tokio_postgres::Error> {
    let sql = format!(
        r#"WITH RECURSIVE {SHIPS_MARKET}
           SELECT "typeID" FROM "invTypes" WHERE "groupID" = ANY($1) AND "raceID" IS NOT NULL AND "published" = 1
              AND "marketGroupID" IN (SELECT "marketGroupID" FROM ships_market) ORDER BY 1"#
    );
    Ok(db.query(sql.as_str(), &[&HULL_GROUPS.to_vec()]).await?.iter().filter_map(|r| int(r, 0)).map(|t| t as i32).collect())
}

impl Shipyard {
    /// Every hull of the ladders, with its market price (from `prices`, the
    /// NPC asks), hold, slots, hardpoints, drone bay and fitting resources;
    /// and the modules, charges and drones fittings draw on.
    pub async fn load(db: &Client, prices: &HashMap<i32, f64>) -> Result<Shipyard, tokio_postgres::Error> {
        let sql = r#"SELECT t."typeID", t."groupID", t."raceID",
                            COALESCE(MAX(COALESCE(a."valueFloat", a."valueInt")) FILTER (WHERE a."attributeID" = $2), 0)::float8,
                            COALESCE(MAX(COALESCE(a."valueFloat", a."valueInt")) FILTER (WHERE a."attributeID" = $3), 0)::float8,
                            COALESCE(MAX(COALESCE(a."valueFloat", a."valueInt")) FILTER (WHERE a."attributeID" = $4), 0)::float8,
                            COALESCE(MAX(COALESCE(a."valueFloat", a."valueInt")) FILTER (WHERE a."attributeID" = $5), 0)::float8,
                            COALESCE(MAX(COALESCE(a."valueFloat", a."valueInt")) FILTER (WHERE a."attributeID" = $6), 0)::float8,
                            COALESCE(t."capacity", 0)::float8,
                            COALESCE(MAX(COALESCE(a."valueFloat", a."valueInt")) FILTER (WHERE a."attributeID" = $7), 0)::float8,
                            COALESCE(MAX(COALESCE(a."valueFloat", a."valueInt")) FILTER (WHERE a."attributeID" = $8), 0)::float8,
                            COALESCE(MAX(COALESCE(a."valueFloat", a."valueInt")) FILTER (WHERE a."attributeID" = $9), 0)::float8,
                            COALESCE(MAX(COALESCE(a."valueFloat", a."valueInt")) FILTER (WHERE a."attributeID" = $10), 0)::float8
                       FROM "invTypes" t
                       LEFT JOIN "dgmTypeAttributes" a ON a."typeID" = t."typeID"
                      WHERE t."groupID" = ANY($1) AND t."raceID" IS NOT NULL AND t."published" = 1
                        AND t."marketGroupID" IN (SELECT "marketGroupID" FROM ships_market)
                      GROUP BY t."typeID" ORDER BY t."typeID""#;
        let sql = format!("WITH RECURSIVE {SHIPS_MARKET} {}", sql.trim_start());
        let mut hulls = HashMap::new();
        let mut by_group_race: HashMap<(i32, i64), Vec<i32>> = HashMap::new();
        let groups = HULL_GROUPS.to_vec();
        let params: [&(dyn tokio_postgres::types::ToSql + Sync); 10] = [
            &groups, &ATTR_CAPACITY, &ATTR_HI_SLOTS, &ATTR_MED_SLOTS, &ATTR_LOW_SLOTS, &ATTR_TURRET_SLOTS,
            &ATTR_LAUNCHER_SLOTS, &ATTR_DRONE_CAPACITY, &ATTR_CPU_OUTPUT, &ATTR_POWER_OUTPUT,
        ];
        for r in db.query(sql.as_str(), &params).await? {
            let type_id = int(&r, 0).unwrap_or(0) as i32;
            let Some(&price) = prices.get(&type_id) else { continue };
            let capacity = float(&r, 3).filter(|c| *c > 0.0).or_else(|| float(&r, 8)).unwrap_or(0.0);
            let slot = |i: usize| float(&r, i).unwrap_or(0.0).clamp(0.0, 8.0) as u8;
            let hull = Hull {
                type_id,
                group: int(&r, 1).unwrap_or(0) as i32,
                race: int(&r, 2).unwrap_or(0),
                price,
                capacity,
                slots: (slot(4), slot(5), slot(6)),
                turrets: slot(7),
                launchers: slot(9),
                drone_bay: float(&r, 10).unwrap_or(0.0),
                cpu: float(&r, 11).unwrap_or(0.0),
                power: float(&r, 12).unwrap_or(0.0),
            };
            by_group_race.entry((hull.group, hull.race)).or_default().push(type_id);
            hulls.insert(type_id, hull);
        }
        for list in by_group_race.values_mut() {
            list.sort_by(|a, b| hulls[a].price.total_cmp(&hulls[b].price).then(a.cmp(b)));
        }
        let sql = r#"SELECT t."typeID", t."groupID", t."typeName",
                            COALESCE(MAX(COALESCE(a."valueInt", a."valueFloat"::int)) FILTER (WHERE a."attributeID" = $2), 0),
                            COALESCE(MAX(COALESCE(a."valueInt", a."valueFloat"::int)) FILTER (WHERE a."attributeID" = $3), 0),
                            COALESCE(MAX(COALESCE(a."valueFloat", a."valueInt")) FILTER (WHERE a."attributeID" = $4), 0)::float8,
                            COALESCE(MAX(COALESCE(a."valueFloat", a."valueInt")) FILTER (WHERE a."attributeID" = $5), 0)::float8,
                            COALESCE(t."volume", 0)::float8, COALESCE(t."raceID", 0)
                       FROM "invTypes" t
                       LEFT JOIN "dgmTypeAttributes" a ON a."typeID" = t."typeID"
                      WHERE t."groupID" = ANY($1) AND t."marketGroupID" IS NOT NULL AND t."published" = 1
                      GROUP BY t."typeID" ORDER BY t."typeID""#;
        let groups = MODULE_GROUPS.to_vec();
        let modules = db
            .query(sql, &[&groups, &ATTR_CHARGE_SIZE, &ATTR_META_LEVEL, &ATTR_CPU, &ATTR_POWER])
            .await?
            .iter()
            .map(|r| Module {
                type_id: int(r, 0).unwrap_or(0) as i32,
                group: int(r, 1).unwrap_or(0) as i32,
                name: text(r, 2).unwrap_or_default(),
                size: int(r, 3).unwrap_or(0) as i32,
                meta: int(r, 4).unwrap_or(0) as i32,
                cpu: float(r, 5).unwrap_or(0.0),
                power: float(r, 6).unwrap_or(0.0),
                volume: float(r, 7).unwrap_or(0.0),
                race: int(r, 8).unwrap_or(0),
            })
            .collect();
        Ok(Shipyard { hulls, by_group_race, modules })
    }

    /// The hull of a role's tier for a race, chosen by `seed` among those of
    /// the right class (another race's when its own has none).
    pub fn hull_for(&self, role: Role, race: i64, tier: u8, seed: u64) -> Option<i32> {
        let rungs = ladder(role);
        let tier = (tier as usize).min(rungs.len() - 1);
        let group = rungs[tier];
        let list = self
            .by_group_race
            .get(&(group, race))
            .or_else(|| self.by_group_race.iter().filter(|((g, _), _)| *g == group).min_by_key(|((_, r), _)| *r).map(|(_, l)| l))?;
        if role == Role::Hauler {
            // Industrials by hold: tier 0 the smallest third, 2 the biggest.
            let mut by_hold = list.clone();
            by_hold.sort_by(|a, b| self.hulls[a].capacity.total_cmp(&self.hulls[b].capacity));
            let third = by_hold.len().div_ceil(3).max(1);
            let part: Vec<i32> = by_hold.chunks(third).nth(tier).or(by_hold.chunks(third).last()).map(|c| c.to_vec()).unwrap_or_default();
            return part.get((seed % part.len().max(1) as u64) as usize).copied();
        }
        list.get((seed % list.len() as u64) as usize).copied()
    }

    /// Where a hull stands on a role's ladder.
    pub fn tier_of(&self, role: Role, hull: i32) -> u8 {
        let Some(h) = self.hulls.get(&hull) else { return 0 };
        if role == Role::Hauler {
            let mut same: Vec<&Hull> = self.hulls.values().filter(|o| o.group == GROUP_INDUSTRIAL && o.race == h.race).collect();
            same.sort_by(|a, b| a.capacity.total_cmp(&b.capacity));
            // The same thirds `hull_for` picks from.
            let at = same.iter().position(|o| o.type_id == hull).unwrap_or(0);
            let third = same.len().div_ceil(3).max(1);
            return (at / third).min(2) as u8;
        }
        ladder(role).iter().position(|g| *g == h.group).unwrap_or(0) as u8
    }

    pub fn price(&self, hull: i32) -> f64 {
        self.hulls.get(&hull).map_or(0.0, |h| h.price)
    }

    pub fn race_of(&self, hull: i32) -> i64 {
        self.hulls.get(&hull).map_or(RACE_AMARR, |h| h.race)
    }

    pub fn capacity(&self, hull: i32) -> f64 {
        self.hulls.get(&hull).map_or(0.0, |h| h.capacity)
    }

    pub fn group_of(&self, hull: i32) -> i32 {
        self.hulls.get(&hull).map_or(0, |h| h.group)
    }

    /// All the hulls any ladder holds: the types space must know.
    pub fn all(&self) -> impl Iterator<Item = i32> + '_ {
        self.hulls.keys().copied()
    }

    /// The module of a group (and size) a bot fits: meta 0 for most, the
    /// best named one (meta up to 4) for rich bots; `words` narrow it, a
    /// race is preferred. The same `rng` gives the same choice.
    fn pick(&self, rng: &mut Rng, group: i32, size: Option<i32>, words: &[&str], race: Option<i64>, rich: bool) -> Option<&Module> {
        let fits = |m: &&Module| m.group == group && size.is_none_or(|s| m.size == s) && m.meta <= if rich { 4 } else { 0 };
        let mut list: Vec<&Module> = self.modules.iter().filter(fits).filter(|m| words.is_empty() || words.iter().any(|w| m.name.contains(w))).collect();
        if list.is_empty() {
            list = self.modules.iter().filter(fits).collect();
        }
        if let Some(race) = race {
            let own: Vec<&Module> = list.iter().copied().filter(|m| m.race == race).collect();
            if !own.is_empty() {
                list = own;
            }
        }
        let best = list.iter().map(|m| m.meta).max()?;
        let tier: Vec<&Module> = list.into_iter().filter(|m| m.meta == best).collect();
        Some(tier[rng.below(tier.len())])
    }

    fn module(&self, type_id: i32) -> Option<&Module> {
        self.modules.iter().find(|m| m.type_id == type_id)
    }

    /// What a bot's hull carries: (type, quantity) of its modules, the
    /// charges they use, and drones in its bay. A fitting suits the hull and
    /// the bot's work, fills the slots, and keeps within the hull's CPU and
    /// powergrid (dropping the least needed modules if it must). The same
    /// bot, role and hull always fit the same; richer bots fit named modules.
    pub fn fitting(&self, hull: i32, role: Role, seed: u64, rich: bool) -> Vec<(i32, i64)> {
        let Some(h) = self.hulls.get(&hull) else { return Vec::new() };
        let mut rng = Rng::seeded(seed ^ (hull as u64).wrapping_mul(0x9e37_79b9) ^ role as u64);
        let class = weight_class(h.group);
        let size = match class {
            0 | 1 => 1,
            2 | 3 => 2,
            _ => 3,
        };
        let words: &[&str] = match size {
            1 => &["Small", "1MN", "Micro"],
            2 => &["Medium", "10MN"],
            _ => &["Large", "100MN"],
        };
        let race = Some(h.race);
        // Fitted modules, one entry a unit, each with how much it matters
        // (the least are dropped first when CPU or grid runs out).
        let mut fitted: Vec<(i32, u8)> = Vec::new();
        let mut put = |m: Option<&Module>, n: i32, first: u8, rest: u8| {
            if let Some(m) = m {
                for i in 0..n.max(0) {
                    fitted.push((m.type_id, if i == 0 { first } else { rest }));
                }
            }
        };
        let (hi, med, low) = (h.slots.0 as i32, h.slots.1 as i32, h.slots.2 as i32);
        let fighter = matches!(role, Role::Ratter | Role::Pirate | Role::Explorer);
        let mining_hull = matches!(h.group, GROUP_BARGE | GROUP_EXHUMER);
        let cargo_hull = matches!(h.group, GROUP_INDUSTRIAL | GROUP_SHUTTLE);
        let shield = matches!(h.race, RACE_CALDARI | RACE_MINMATAR);
        let (gun_group, launcher_group) = match h.race {
            RACE_AMARR => (ENERGY_WEAPON, None),
            RACE_MINMATAR => (PROJECTILE_WEAPON, None),
            _ => (HYBRID_WEAPON, None::<i32>),
        };
        let launcher_group = launcher_group.unwrap_or(match size {
            1 => if rng.unit() < 0.4 { LAUNCHER_ROCKET } else { LAUNCHER_STANDARD },
            2 => LAUNCHER_HEAVY,
            _ => if rng.unit() < 0.4 { LAUNCHER_SIEGE } else { LAUNCHER_CRUISE },
        });

        // Highs.
        let (mut guns, mut launchers) = (0, 0);
        if mining_hull {
            put(self.pick(&mut rng, STRIP_MINER, None, &[], None, rich), hi.max(1), 10, 8);
        } else if cargo_hull {
            // Nothing in the highs: haulers run, they don't fight.
        } else if role == Role::Miner {
            let lasers = (h.turrets as i32).clamp(1, hi.max(1));
            put(self.pick(&mut rng, MINING_LASER, None, &[], None, rich), lasers, 10, 8);
        } else {
            let mut free = hi;
            if role == Role::Explorer && free > 0 {
                put(self.pick(&mut rng, SCAN_PROBE_LAUNCHER, None, &[], None, rich), 1, 9, 9);
                free -= 1;
            }
            guns = (h.turrets as i32).min(free);
            launchers = (h.launchers as i32).min(free - guns);
            put(self.pick(&mut rng, gun_group, Some(size), &[], None, rich), guns, 10, 6);
            put(self.pick(&mut rng, launcher_group, None, &[], race, rich), launchers, if guns == 0 { 10 } else { 7 }, 6);
            if role == Role::Pirate && free - guns - launchers > 0 {
                put(self.pick(&mut rng, ENERGY_VAMPIRE, None, words, None, rich), 1, 2, 2);
            }
        }

        // Mids.
        let mut m = med;
        if m > 0 {
            let prop = if role == Role::Pirate && (rich || class >= 2) { MICROWARPDRIVE } else { AFTERBURNER };
            put(self.pick(&mut rng, prop, None, words, None, rich), 1, 7, 7);
            m -= 1;
        }
        if role == Role::Pirate && m > 0 {
            put(self.pick(&mut rng, WARP_SCRAMBLER, None, &[], None, rich), 1, 6, 6);
            m -= 1;
            if m > 1 {
                put(self.pick(&mut rng, STASIS_WEB, None, &[], None, rich), 1, 5, 5);
                m -= 1;
            }
        }
        if fighter && shield {
            if m > 0 {
                put(self.pick(&mut rng, SHIELD_BOOSTER, None, words, None, rich), 1, 7, 7);
                m -= 1;
            }
            if m > 1 {
                put(self.pick(&mut rng, SHIELD_HARDENER, None, &[], None, rich), 1, 3, 3);
                m -= 1;
            }
            put(self.pick(&mut rng, SHIELD_EXTENDER, None, words, None, rich), m, 5, 5);
        } else if fighter {
            put(self.pick(&mut rng, CAP_RECHARGER, None, &[], None, rich), m, 3, 3);
        } else {
            put(self.pick(&mut rng, SHIELD_EXTENDER, None, words, None, rich), m, 4, 4);
        }

        // Lows.
        let mut l = low;
        if mining_hull || role == Role::Miner {
            let upgrades = l.min(2);
            put(self.pick(&mut rng, MINING_UPGRADE, None, &[], None, rich), upgrades, 5, 5);
            l -= upgrades;
            put(self.pick(&mut rng, DAMAGE_CONTROL, None, &[], None, rich), l.min(1), 8, 8);
        } else if cargo_hull || !fighter {
            put(self.pick(&mut rng, DAMAGE_CONTROL, None, &[], None, rich), l.min(1), 8, 8);
        } else {
            let damage = if launchers > guns {
                BALLISTIC_CONTROL
            } else {
                match gun_group {
                    PROJECTILE_WEAPON => GYROSTABILIZER,
                    ENERGY_WEAPON => HEAT_SINK,
                    _ => MAGNETIC_STABILIZER,
                }
            };
            if l > 0 {
                put(self.pick(&mut rng, DAMAGE_CONTROL, None, &[], None, rich), 1, 8, 8);
                l -= 1;
            }
            if !shield {
                if l > 0 {
                    put(self.pick(&mut rng, ARMOR_REPAIRER, None, words, None, rich), 1, 7, 7);
                    l -= 1;
                }
                let plates = l.min(if class >= 3 { 2 } else { 1 });
                put(self.pick(&mut rng, ARMOR_PLATE, None, words, None, rich), plates, 5, 5);
                l -= plates;
                if l > 1 {
                    put(self.pick(&mut rng, ARMOR_HARDENER, None, &[], None, rich), 1, 3, 3);
                    l -= 1;
                }
            }
            put(self.pick(&mut rng, damage, None, &[], None, rich), l, 4, 4);
        }

        // Within CPU and powergrid: drop the least needed until it fits.
        let (cpu, power) = (h.cpu * FITTING_SKILLS, h.power * FITTING_SKILLS);
        if cpu > 0.0 || power > 0.0 {
            loop {
                let used = fitted.iter().filter_map(|(t, _)| self.module(*t)).fold((0.0, 0.0), |(c, p), m| (c + m.cpu, p + m.power));
                if (used.0 <= cpu || cpu <= 0.0) && (used.1 <= power || power <= 0.0) || fitted.len() <= 1 {
                    break;
                }
                let least = fitted.iter().enumerate().min_by_key(|(i, (_, rank))| (*rank, usize::MAX - i)).map(|(i, _)| i).unwrap_or(0);
                fitted.remove(least);
            }
        }

        let mut out: Vec<(i32, i64)> = Vec::new();
        let mut add = |t: i32, n: i64| {
            if n > 0 {
                match out.iter_mut().find(|(o, _)| *o == t) {
                    Some(e) => e.1 += n,
                    None => out.push((t, n)),
                }
            }
        };
        for (t, _) in &fitted {
            add(*t, 1);
        }
        // What the fitted guns and launchers shoot.
        let count = |group: i32| fitted.iter().filter(|(t, _)| self.module(*t).is_some_and(|m| m.group == group)).count() as i64;
        let guns_left = count(gun_group);
        if guns_left > 0 {
            let ammo = match gun_group {
                ENERGY_WEAPON => FREQUENCY_CRYSTAL,
                PROJECTILE_WEAPON => PROJECTILE_AMMO,
                _ => HYBRID_CHARGE,
            };
            let rounds = if ammo == FREQUENCY_CRYSTAL { guns_left } else { guns_left * 200 };
            if let Some(a) = self.pick(&mut rng, ammo, Some(size), &[], None, false) {
                add(a.type_id, rounds);
            }
        }
        let launchers_left = count(launcher_group);
        if launchers_left > 0 {
            let missile = match launcher_group {
                LAUNCHER_ROCKET => ROCKET,
                LAUNCHER_STANDARD => LIGHT_MISSILE,
                LAUNCHER_HEAVY => HEAVY_MISSILE,
                LAUNCHER_SIEGE => TORPEDO,
                _ => CRUISE_MISSILE,
            };
            if let Some(a) = self.pick(&mut rng, missile, None, &[], race, false) {
                add(a.type_id, launchers_left * 100);
            }
        }
        if count(SCAN_PROBE_LAUNCHER) > 0 {
            if let Some(p) = self.pick(&mut rng, SCANNER_PROBE, None, &[], None, false) {
                add(p.type_id, 8);
            }
        }
        // Drones, as many as the bay holds (five fly at once).
        if h.drone_bay >= 5.0 && !cargo_hull || h.drone_bay >= 5.0 && role == Role::Miner {
            let mining = role == Role::Miner || mining_hull;
            if mining || fighter {
                let want = if mining { 5.0 } else { match class { 0 | 1 => 5.0, 2 => if h.drone_bay >= 50.0 { 10.0 } else { 5.0 }, _ => if h.drone_bay >= 125.0 { 25.0 } else { 10.0 } } };
                let group = if mining { MINING_DRONE } else { COMBAT_DRONE };
                let kinds: Vec<&Module> = self.modules.iter().filter(|m| m.group == group && m.volume > 0.0 && m.volume <= want && m.meta <= if rich { 5 } else { 0 }).collect();
                let biggest = kinds.iter().map(|m| m.volume).fold(0.0, f64::max);
                let mut sized: Vec<&Module> = kinds.into_iter().filter(|m| m.volume == biggest).collect();
                let own: Vec<&Module> = sized.iter().copied().filter(|m| m.race == h.race).collect();
                if !own.is_empty() {
                    sized = own;
                }
                let best = sized.iter().map(|m| m.meta).max().unwrap_or(0);
                sized.retain(|m| m.meta == best);
                if !sized.is_empty() && biggest > 0.0 {
                    let d = sized[rng.below(sized.len())];
                    let n = ((h.drone_bay / d.volume).floor() as i64).min(10);
                    add(d.type_id, n);
                }
            }
        }
        out
    }

    /// The combat drones a fitting carries, and how many fly at once.
    pub fn combat_drones(&self, fitting: &[(i32, i64)]) -> Option<(i32, usize)> {
        fitting.iter().find(|(t, _)| self.module(*t).is_some_and(|m| m.group == COMBAT_DRONE)).map(|(t, n)| (*t, (*n).min(5) as usize))
    }

    /// Every combat drone type a fitting may hold, for space to know them.
    pub fn drone_types(&self) -> impl Iterator<Item = i32> + '_ {
        self.modules.iter().filter(|m| m.group == COMBAT_DRONE).map(|m| m.type_id)
    }

    /// The guns and launchers in a fitting and what they load, for its
    /// ship to mount and fire them where players can see.
    pub fn kit(&self, fitting: &[(i32, i64)]) -> Kit {
        let group = |t: i32| self.module(t).map(|m| m.group).or_else(|| super::capitals::group_of(t));
        let mut kit = Kit::default();
        for &(t, n) in fitting {
            let Some(g) = group(t) else { continue };
            match g {
                ENERGY_WEAPON | PROJECTILE_WEAPON | HYBRID_WEAPON => {
                    if kit.turret.is_none_or(|(_, _, have)| n as usize > have) {
                        kit.turret = Some((t, g, n as usize));
                    }
                }
                LAUNCHER_CRUISE | LAUNCHER_ROCKET | LAUNCHER_SIEGE | LAUNCHER_STANDARD | LAUNCHER_HEAVY | LAUNCHER_CITADEL => {
                    kit.launcher = Some((t, n as usize));
                }
                PROJECTILE_AMMO | HYBRID_CHARGE | FREQUENCY_CRYSTAL => kit.ammo = Some(t),
                TORPEDO | LIGHT_MISSILE | HEAVY_MISSILE | CRUISE_MISSILE | ROCKET | CITADEL_TORPEDO => kit.missile = Some((t, g)),
                _ => {}
            }
        }
        kit
    }

    /// How much harder a fitting hits than a plain one: named guns and
    /// launchers do a little more.
    pub fn punch(&self, fitting: &[(i32, i64)]) -> f64 {
        let best = fitting
            .iter()
            .filter_map(|(t, _)| self.module(*t).filter(|m| matches!(m.group, ENERGY_WEAPON | PROJECTILE_WEAPON | HYBRID_WEAPON | LAUNCHER_CRUISE | LAUNCHER_ROCKET | LAUNCHER_SIEGE | LAUNCHER_STANDARD | LAUNCHER_HEAVY)))
            .map(|m| m.meta)
            .max()
            .unwrap_or(0);
        1.0 + 0.05 * best as f64
    }
}

/// A fitting's weapons: its turrets (type, group, how many), launchers
/// (type, how many), the charge its turrets load and the missiles (type,
/// group) its launchers fire.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Kit {
    pub turret: Option<(i32, i32, usize)>,
    pub launcher: Option<(i32, usize)>,
    pub ammo: Option<i32>,
    pub missile: Option<(i32, i32)>,
}

pub const GROUP_ENERGY_WEAPON: i32 = ENERGY_WEAPON;
pub const GROUP_PROJECTILE_WEAPON: i32 = PROJECTILE_WEAPON;
pub const GROUP_ROCKET: i32 = ROCKET;
pub const GROUP_LIGHT_MISSILE: i32 = LIGHT_MISSILE;
pub const GROUP_HEAVY_MISSILE: i32 = HEAVY_MISSILE;
pub const GROUP_CRUISE_MISSILE: i32 = CRUISE_MISSILE;
pub const GROUP_TORPEDO: i32 = TORPEDO;
pub const GROUP_CITADEL_TORPEDO: i32 = CITADEL_TORPEDO;

/// The tier a bot moves up to, if its wallet allows: when it holds the next
/// hull's price `CUSHION` times over.
pub const CUSHION: f64 = 6.0;

#[cfg(test)]
pub mod tests {
    use super::*;

    /// A shipyard with a frigate, a cruiser and a battleship of one race,
    /// a barge, two industrials, and a Caldari missile frigate.
    pub fn yard() -> Shipyard {
        let hull = |type_id, group, price, capacity, slots, turrets| Hull {
            type_id, group, race: RACE_MINMATAR, price, capacity, slots, turrets, launchers: 0, drone_bay: 0.0, cpu: 0.0, power: 0.0,
        };
        let mut list = vec![
            hull(587, GROUP_FRIGATE, 300_000.0, 140.0, (4, 3, 3), 3),
            Hull { drone_bay: 30.0, ..hull(621, GROUP_CRUISER, 6_000_000.0, 400.0, (5, 4, 4), 4) },
            hull(643, GROUP_BATTLESHIP, 90_000_000.0, 600.0, (7, 5, 6), 6),
            hull(17478, GROUP_BARGE, 20_000_000.0, 8_000.0, (2, 2, 1), 0),
            hull(648, GROUP_INDUSTRIAL, 500_000.0, 2_000.0, (2, 2, 3), 0),
            hull(650, GROUP_INDUSTRIAL, 900_000.0, 6_000.0, (2, 3, 4), 0),
        ];
        list.push(Hull { race: RACE_CALDARI, launchers: 3, ..hull(603, GROUP_FRIGATE, 300_000.0, 150.0, (4, 4, 2), 1) });
        let mut by_group_race: HashMap<(i32, i64), Vec<i32>> = HashMap::new();
        for h in &list {
            by_group_race.entry((h.group, h.race)).or_default().push(h.type_id);
        }
        let module = |type_id, group, size, name: &str| Module { type_id, group, size, meta: 0, name: name.into(), cpu: 0.0, power: 0.0, volume: 5.0, race: 0 };
        Shipyard {
            hulls: list.into_iter().map(|h| (h.type_id, h)).collect(),
            by_group_race,
            modules: vec![
                module(486, PROJECTILE_WEAPON, 1, "125mm Gatling AutoCannon I"),
                module(497, PROJECTILE_WEAPON, 2, "220mm Vulcan AutoCannon I"),
                module(179, PROJECTILE_AMMO, 1, "EMP S"),
                module(180, PROJECTILE_AMMO, 2, "EMP M"),
                module(561, HYBRID_WEAPON, 1, "75mm Gatling Rail I"),
                module(230, HYBRID_CHARGE, 1, "Antimatter Charge S"),
                module(499, LAUNCHER_STANDARD, 0, "Standard Missile Launcher I"),
                module(10631, LAUNCHER_ROCKET, 0, "Rocket Launcher I"),
                module(213, LIGHT_MISSILE, 0, "Bloodclaw Light Missile"),
                module(2516, ROCKET, 0, "Foxfire Rocket"),
                module(438, AFTERBURNER, 0, "1MN Afterburner I"),
                module(380, SHIELD_EXTENDER, 0, "Small Shield Extender I"),
                module(399, SHIELD_BOOSTER, 0, "Small Shield Booster I"),
                module(519, GYROSTABILIZER, 0, "Gyrostabilizer I"),
                module(22291, BALLISTIC_CONTROL, 0, "Ballistic Control System I"),
                module(2046, DAMAGE_CONTROL, 0, "Damage Control I"),
                module(17482, STRIP_MINER, 0, "Strip Miner I"),
                Module { race: RACE_MINMATAR, ..module(2488, COMBAT_DRONE, 0, "Warrior I") },
                Module { volume: 10.0, race: RACE_MINMATAR, ..module(2183, COMBAT_DRONE, 0, "Valkyrie I") },
            ],
        }
    }

    #[test]
    fn ladders_climb_by_class_and_haulers_by_hold() {
        let y = yard();
        assert_eq!(y.hull_for(Role::Ratter, RACE_MINMATAR, 0, 1), Some(587));
        assert_eq!(y.hull_for(Role::Ratter, RACE_MINMATAR, 2, 1), Some(621));
        assert_eq!(y.hull_for(Role::Miner, RACE_MINMATAR, 1, 1), Some(17478));
        assert_eq!(y.hull_for(Role::Hauler, RACE_MINMATAR, 0, 5), Some(648));
        assert_eq!(y.hull_for(Role::Hauler, RACE_MINMATAR, 2, 5), Some(650));
        assert_eq!(y.tier_of(Role::Ratter, 643), 4);
        // With only two industrials, the bigger is as high as haulers go.
        assert_eq!(y.tier_of(Role::Hauler, 650), 1);
    }

    #[test]
    fn a_fitting_is_guns_of_the_race_and_size_with_ammunition_and_a_tank() {
        let y = yard();
        let frigate = y.fitting(587, Role::Ratter, 7, false);
        assert!(frigate.contains(&(486, 3)), "{frigate:?}");
        assert!(frigate.iter().any(|(t, n)| *t == 179 && *n == 600));
        assert!(frigate.contains(&(438, 1)) && frigate.contains(&(399, 1)) && frigate.contains(&(380, 1)), "{frigate:?}");
        assert!(frigate.contains(&(2046, 1)) && frigate.contains(&(519, 2)), "every low used: {frigate:?}");
        // The same bot and hull always fit the same.
        assert_eq!(frigate, y.fitting(587, Role::Ratter, 7, false));
        let cruiser = y.fitting(621, Role::Ratter, 7, false);
        assert!(cruiser.iter().any(|(t, n)| *t == 497 && *n == 4), "{cruiser:?}");
        let barge = y.fitting(17478, Role::Miner, 7, false);
        assert!(barge.contains(&(17482, 2)), "{barge:?}");
    }

    #[test]
    fn missile_boats_fit_launchers_and_drone_boats_carry_drones() {
        let y = yard();
        let kestrel = y.fitting(603, Role::Ratter, 3, false);
        let launchers: i64 = kestrel.iter().filter(|(t, _)| *t == 499 || *t == 10631).map(|(_, n)| n).sum();
        assert_eq!(launchers, 3, "{kestrel:?}");
        assert!(kestrel.iter().any(|(t, n)| (*t == 213 || *t == 2516) && *n == 300), "missiles for them: {kestrel:?}");
        assert!(kestrel.contains(&(561, 1)) && kestrel.contains(&(22291, 1)), "{kestrel:?}");
        // A 30 m3 bay on a cruiser is too small for a flight of mediums:
        // six light drones, five of which fly.
        let cruiser = y.fitting(621, Role::Ratter, 7, false);
        assert!(cruiser.contains(&(2488, 6)), "{cruiser:?}");
        assert_eq!(y.combat_drones(&cruiser), Some((2488, 5)));
        if let Some(h) = y.hulls.get(&621).cloned() {
            let mut y = y;
            y.hulls.insert(621, Hull { drone_bay: 50.0, ..h });
            assert!(y.fitting(621, Role::Ratter, 7, false).contains(&(2183, 5)), "a 50 m3 bay takes five mediums");
        }
    }

    #[test]
    fn a_fitting_keeps_within_cpu_and_grid() {
        let mut y = yard();
        if let Some(h) = y.hulls.get_mut(&587) {
            h.cpu = 100.0;
            h.power = 1_000.0;
        }
        for m in &mut y.modules {
            m.cpu = 30.0;
        }
        // 125 tf with skills: four 30 tf modules fit, the guns first.
        let fit = y.fitting(587, Role::Ratter, 7, false);
        let modules: i64 = fit.iter().filter(|(t, _)| ![179].contains(t)).map(|(_, n)| n).sum();
        assert_eq!(modules, 4, "{fit:?}");
        assert!(fit.contains(&(486, 1)), "the first gun stays: {fit:?}");
    }
}
