//! Capital ships in the bots' sovereignty battles.
//!
//! Each side of a battle brings, as its size, the stakes and its wealth
//! allow:
//! - **Logistics** (Guardian, Oneiros, Basilisk, Scimitar): warp in with the
//!   fleet and keep it alive with remote repairs or shield transfers.
//! - **Cyno ships** (Pilgrim, Arazu, Falcon, Rapier): warp in first and
//!   light a cynosural field for the capitals to jump to.
//! - **Dreadnoughts**: jump in to the cyno, go into siege mode and bombard
//!   the enemy's tower.
//! - **Carriers**: jump in to the cyno and fight with their fighters.
//! - **Titans**, in the biggest fights: jump in, bridge the rest of their
//!   fleet in behind them, and fire their doomsday on the enemy's fleet.
//!
//! The richest pilots of a side fly its capitals, for that battle; one who
//! survives goes back to the ship it had before. Fought unseen, capitals
//! count for many ships and are hard to kill, logistics cut their side's
//! losses, and titans' doomsdays thin the enemy's numbers.

use super::battle::Stakes;
use super::behaviour::Bot;
use crate::combat::Rng;

const CALDARI: i64 = 1;
const MINMATAR: i64 = 2;
const AMARR: i64 = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Class {
    Logistics,
    Cyno,
    Carrier,
    Dreadnought,
    Titan,
}

impl Class {
    /// Comes by jump drive to a cynosural field rather than through a gate.
    pub fn jumps(self) -> bool {
        matches!(self, Class::Carrier | Class::Dreadnought | Class::Titan)
    }

    /// How many ships it is worth in a fight fought unseen.
    pub fn weight(self) -> f64 {
        match self {
            Class::Logistics | Class::Cyno => 1.0,
            Class::Carrier => 8.0,
            Class::Dreadnought => 10.0,
            Class::Titan => 40.0,
        }
    }

    /// The chance a shot picked for it goes to another ship instead: big
    /// hulls take a long time to kill.
    pub fn hard_to_kill(self) -> f64 {
        match self {
            Class::Logistics | Class::Cyno => 0.0,
            Class::Carrier => 0.8,
            Class::Dreadnought => 0.8,
            Class::Titan => 0.97,
        }
    }

    pub fn name(self, n: usize) -> &'static str {
        match (self, n == 1) {
            (Class::Logistics, _) => "logistics",
            (Class::Cyno, true) => "cyno ship",
            (Class::Cyno, false) => "cyno ships",
            (Class::Carrier, true) => "carrier",
            (Class::Carrier, false) => "carriers",
            (Class::Dreadnought, true) => "dreadnought",
            (Class::Dreadnought, false) => "dreadnoughts",
            (Class::Titan, true) => "titan",
            (Class::Titan, false) => "titans",
        }
    }

    /// The hull of this class a pilot of `race` flies.
    pub fn hull(self, race: i64) -> i32 {
        let pick = |amarr: i32, caldari: i32, gallente: i32, minmatar: i32| match race {
            AMARR => amarr,
            CALDARI => caldari,
            MINMATAR => minmatar,
            _ => gallente,
        };
        match self {
            Class::Logistics => pick(11987, 11985, 11989, 11978),
            Class::Cyno => pick(11965, 11957, 11969, 11963),
            Class::Carrier => pick(23757, 23915, 23911, 24483),
            Class::Dreadnought => pick(19720, 19726, 19724, 19722),
            Class::Titan => pick(11567, 3764, 671, 23773),
        }
    }

    /// Shield transfers (Caldari and Minmatar), or armor repairs.
    pub fn shield_logistics(race: i64) -> bool {
        matches!(race, CALDARI | MINMATAR)
    }
}

pub const CLASSES: [Class; 5] = [Class::Titan, Class::Dreadnought, Class::Carrier, Class::Logistics, Class::Cyno];

/// Every hull these classes use: space must know them to show them.
pub const HULLS: [i64; 20] = [
    11987, 11985, 11989, 11978, 11965, 11957, 11969, 11963, 23757, 23915, 23911, 24483, 19720, 19726, 19724, 19722, 11567, 3764,
    671, 23773,
];

/// Fighters, one per race: Templar, Dragonfly, Firbolg, Einherji.
pub const FIGHTERS: [i64; 4] = [23055, 23057, 23059, 23061];

/// What a carrier or mothership launches: its race's fighters, and how many.
/// In 2007 a carrier pilot with Carrier V flew ten; a mothership, with its
/// drone control units, fifteen.
pub fn fighters(hull: i64) -> Option<(i64, usize)> {
    Some(match hull {
        23757 => (23055, 10), // Archon: Templar
        23915 => (23057, 10), // Chimera: Dragonfly
        24483 => (23059, 10), // Nidhoggur: Firbolg
        23911 => (23061, 10), // Thanatos: Einherji
        23919 => (23055, 15), // Aeon
        23917 => (23057, 15), // Wyvern
        22852 => (23059, 15), // Hel
        23913 => (23061, 15), // Nyx
        _ => return None,
    })
}

// Capital modules and charges, from the 2007 database.
const DUAL_GIGA_PULSE: i32 = 20444;
const DUAL_1000MM_RAIL: i32 = 20448;
const ION_SIEGE_BLASTER: i32 = 20450;
const QUAD_3500MM_ARTY: i32 = 20454;
const CITADEL_LAUNCHER: i32 = 20539;
const SIEGE_MODULE: i32 = 20280;
const CAPITAL_ARMOR_REPAIRER: i32 = 20701;
const CAPITAL_SHIELD_BOOSTER: i32 = 20703;
const CAPITAL_REMOTE_ARMOR: i32 = 24569;
const DAMAGE_CONTROL: i32 = 2046;
pub const CYNO_GENERATOR: i32 = 21096;
const MULTIFREQUENCY_XL: i32 = 17686;
const ANTIMATTER_XL: i32 = 17648;
const PHASED_PLASMA_XL: i32 = 17674;
const DOOM_TORPEDO: i32 = 17863;
const THOR_TORPEDO: i32 = 17857;
const JUDGEMENT: i32 = 24550;
const OBLIVION: i32 = 24552;
const AURORA_OMINAE: i32 = 24554;
const GJALLARHORN: i32 = 23674;

/// Groups of the capital weapons and charges above: energy, hybrid and
/// projectile turrets, citadel launchers and their torpedoes, and the
/// charges the turrets load.
pub fn group_of(type_id: i32) -> Option<i32> {
    Some(match type_id {
        DUAL_GIGA_PULSE => 53,
        DUAL_1000MM_RAIL | ION_SIEGE_BLASTER => 74,
        QUAD_3500MM_ARTY => 55,
        CITADEL_LAUNCHER => 524,
        DOOM_TORPEDO | THOR_TORPEDO => 476,
        MULTIFREQUENCY_XL => 86,
        ANTIMATTER_XL => 85,
        PHASED_PLASMA_XL => 83,
        _ => return None,
    })
}

/// What a capital hull flies with into a battle, as a 2007 pilot fitted
/// it: dreadnoughts their race's siege guns or citadel launchers and a
/// siege module, carriers fighters and capital remote repairers, titans
/// their doomsday and a broadside of capital guns. None for other hulls.
pub fn fit(hull: i32) -> Option<Vec<(i32, i64)>> {
    let tank = |shield: bool| if shield { (CAPITAL_SHIELD_BOOSTER, 1) } else { (CAPITAL_ARMOR_REPAIRER, 1) };
    Some(match hull {
        // Dreadnoughts: Revelation, Phoenix, Moros, Naglfar.
        19720 => vec![(DUAL_GIGA_PULSE, 3), (MULTIFREQUENCY_XL, 3), (SIEGE_MODULE, 1), tank(false), (DAMAGE_CONTROL, 1)],
        19726 => vec![(CITADEL_LAUNCHER, 3), (DOOM_TORPEDO, 60), (SIEGE_MODULE, 1), tank(true), (DAMAGE_CONTROL, 1)],
        19724 => vec![(ION_SIEGE_BLASTER, 3), (ANTIMATTER_XL, 300), (SIEGE_MODULE, 1), tank(false), (DAMAGE_CONTROL, 1)],
        19722 => vec![(QUAD_3500MM_ARTY, 2), (PHASED_PLASMA_XL, 200), (CITADEL_LAUNCHER, 1), (THOR_TORPEDO, 20), (SIEGE_MODULE, 1), tank(true), (DAMAGE_CONTROL, 1)],
        // Carriers: Archon, Chimera, Thanatos, Nidhoggur. Their fighters
        // come from `fighters`.
        23757 | 23911 => vec![(CAPITAL_REMOTE_ARMOR, 2), tank(false), (DAMAGE_CONTROL, 1)],
        23915 | 24483 => vec![(CAPITAL_REMOTE_ARMOR, 2), tank(true), (DAMAGE_CONTROL, 1)],
        // Titans: Avatar, Leviathan, Erebus, Ragnarok.
        11567 => vec![(JUDGEMENT, 1), (DUAL_GIGA_PULSE, 4), (MULTIFREQUENCY_XL, 4), tank(false), (DAMAGE_CONTROL, 1)],
        3764 => vec![(AURORA_OMINAE, 1), (CITADEL_LAUNCHER, 4), (DOOM_TORPEDO, 80), tank(true), (DAMAGE_CONTROL, 1)],
        671 => vec![(OBLIVION, 1), (ION_SIEGE_BLASTER, 4), (ANTIMATTER_XL, 400), tank(false), (DAMAGE_CONTROL, 1)],
        23773 => vec![(GJALLARHORN, 1), (QUAD_3500MM_ARTY, 4), (PHASED_PLASMA_XL, 400), tank(true), (DAMAGE_CONTROL, 1)],
        _ => return None,
    })
}

/// The recon hulls that carry a cynosural field generator into a battle.
pub fn cyno_hull(hull: i32) -> bool {
    matches!(hull, 11965 | 11957 | 11969 | 11963)
}

/// A pilot flying something special for a battle.
#[derive(Debug, Clone, Copy)]
pub struct Assigned {
    pub bot: u32,
    pub class: Class,
    pub race: i64,
    /// What it flew before, to go back to.
    pub original: i32,
}

/// How many of each class a side of `brought` ships brings. A battle asked
/// for by hand (`showcase`) brings at least one of every kind, as far as the
/// fleet goes: a titan, dreadnoughts, carriers, logistics and a cyno ship.
pub fn plan(brought: u32, stakes: Stakes, wealth: f64, showcase: bool, rng: &mut Rng) -> Vec<(Class, usize)> {
    let n = brought as f64;
    let rich = wealth.clamp(1.0, 2.0);
    let jitter = |rng: &mut Rng, x: f64| (x * (0.75 + rng.unit() * 0.5)).floor().max(0.0) as usize;
    let (dread, carrier) = match stakes {
        Stakes::Skirmish => (n / 80.0, n / 90.0),
        Stakes::Battle => (n / 35.0, n / 45.0),
        Stakes::Decisive => (n / 18.0, n / 25.0),
    };
    let mut dreads = jitter(rng, dread * rich);
    let mut carriers = jitter(rng, carrier * rich);
    let mut titans = match stakes {
        Stakes::Decisive if brought >= 150 => 1 + (brought.saturating_sub(150) / 700) as usize,
        Stakes::Battle if brought >= 500 && wealth >= 1.5 => 1,
        _ => 0,
    }
    .min(4);
    // Capitals are a quarter of the fleet at most.
    let most = (brought / 4) as usize;
    while dreads + carriers + titans > most {
        if dreads >= carriers && dreads > 0 {
            dreads -= 1;
        } else if carriers > 0 {
            carriers -= 1;
        } else if titans > 0 {
            titans -= 1;
        } else {
            break;
        }
    }
    let mut logistics = if brought >= 10 { jitter(rng, n / 10.0).max(1) } else { 0 };
    if showcase {
        // Half the fleet at most, the rest to fight round them.
        let room = (brought / 2) as usize;
        titans = titans.max(1).min(room);
        dreads = dreads.max(3).min(room.saturating_sub(titans));
        carriers = carriers.max(2).min(room.saturating_sub(titans + dreads));
        logistics = logistics.max(3).min(room.saturating_sub(titans + dreads + carriers));
    }
    let jumpers = dreads + carriers + titans;
    let cynos = if jumpers > 0 { 1 + jumpers / 30 } else { 0 };
    vec![
        (Class::Titan, titans),
        (Class::Dreadnought, dreads),
        (Class::Carrier, carriers),
        (Class::Logistics, logistics),
        (Class::Cyno, cynos),
    ]
}

/// Picks the pilots for `plan` from `fleet`: the richest fly the biggest
/// ships. Their hulls change for the battle.
pub fn assign(fleet: &[u32], bots: &mut [Bot], plan: &[(Class, usize)], race_of: impl Fn(i32) -> i64) -> Vec<Assigned> {
    let mut order: Vec<u32> = fleet.to_vec();
    order.sort_by(|a, b| bots[*b as usize].isk.total_cmp(&bots[*a as usize].isk));
    let mut out = Vec::new();
    let mut next = order.into_iter();
    for &(class, count) in plan {
        for _ in 0..count {
            let Some(i) = next.next() else { return out };
            let bot = &mut bots[i as usize];
            let race = race_of(bot.hull);
            out.push(Assigned { bot: i, class, race, original: bot.hull });
            bot.hull = class.hull(race);
        }
    }
    out
}

/// Puts a pilot back in the ship it flew before the battle, unless it has
/// since changed ship on its own.
pub fn restore(bots: &mut [Bot], a: &Assigned) {
    let bot = &mut bots[a.bot as usize];
    if bot.hull == a.class.hull(a.race) {
        bot.hull = a.original;
    }
}

/// "2 titans, 14 dreadnoughts, 9 carriers": the capitals in `counts`.
pub fn describe(counts: &[(Class, usize)]) -> String {
    let parts: Vec<String> = counts
        .iter()
        .filter(|(c, n)| *n > 0 && c.jumps())
        .map(|(c, n)| format!("{n} {}", c.name(*n)))
        .collect();
    parts.join(", ")
}
