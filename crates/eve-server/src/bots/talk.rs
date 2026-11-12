//! Local chat: bots talking among themselves, answering players and
//! reacting to kills, the way Local reads in EVE: greetings, questions,
//! trolling, trash talk and "gf" after a fight.
//!
//! Each bot has a persona, fixed by who it is and what it does: a pirate
//! gloats, a miner grumbles about gankers, a troll baits. A player who
//! speaks gets answers after a moment's "typing"; the bot that answers keeps
//! the conversation for a few minutes, and remembers what it asked. Lines
//! wait in a queue and are only said if the bot is still in that Local.
//! How much they talk, and whether they trash talk, are dashboard settings
//! (`super::chatter`, `super::trash_talk`).

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use eve_marshal::Value;
use tracing::warn;

use super::behaviour::Role;
use super::Chatter;
use crate::combat::Rng;
use crate::world::{LocalBot, World, WorldState};

// ---- who a bot is ------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Persona {
    /// Polite, helpful, says o7.
    Friendly,
    /// Been around for years, dry, knows everything.
    Veteran,
    /// New, asks questions, excited about everything.
    Newbie,
    /// Baits, lies, says "lol".
    Troll,
    /// Hunts, gloats, threatens.
    Pirate,
    /// Mines and runs missions, hates gankers.
    Carebear,
    /// Buys, sells, spams Local with offers.
    Trader,
    /// Talks in character about empires and capsuleers.
    Roleplayer,
    /// Complains about everything.
    Grumpy,
}

/// A bot's persona: its role sets the likely ones, its ID picks one.
pub fn persona(char_id: i64, role: u8) -> Persona {
    use Persona::*;
    let pool: &[Persona] = match role {
        r if r == Role::Pirate as u8 => &[Pirate, Pirate, Pirate, Troll, Troll, Grumpy, Veteran],
        r if r == Role::Ratter as u8 => &[Veteran, Grumpy, Friendly, Troll, Pirate, Carebear, Newbie],
        r if r == Role::Miner as u8 => &[Carebear, Carebear, Friendly, Newbie, Grumpy, Troll, Roleplayer],
        r if r == Role::Hauler as u8 => &[Friendly, Grumpy, Trader, Newbie, Carebear, Veteran],
        r if r == Role::Trader as u8 => &[Trader, Trader, Troll, Friendly, Grumpy, Veteran],
        r if r == Role::Explorer as u8 => &[Roleplayer, Friendly, Veteran, Newbie, Troll, Grumpy],
        _ => &[Friendly, Veteran, Newbie, Troll, Grumpy, Carebear],
    };
    let mut h = (char_id as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    h ^= h >> 29;
    let p = pool[(h % pool.len() as u64) as usize];
    // With trash talk off, the rough ones keep their topics but mind their words.
    if !super::trash_talk() {
        return match p {
            Troll => Friendly,
            Pirate => Veteran,
            other => other,
        };
    }
    p
}

/// How a persona types: some write properly, some never use capitals.
fn styled(p: Persona, line: &str, rng: &mut Rng) -> String {
    match p {
        Persona::Troll | Persona::Newbie if rng.unit() < 0.6 => line.to_lowercase().trim_end_matches('.').to_string(),
        Persona::Pirate | Persona::Grumpy if rng.unit() < 0.35 => line.to_lowercase().trim_end_matches('.').to_string(),
        _ => line.to_string(),
    }
}

// ---- what is said ------------------------------------------------------------------------------

/// What a player's line is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Intent {
    Greeting,
    HowAreYou,
    Doing,
    Ship,
    Fit,
    Isk,
    Help,
    Fight,
    Insult,
    Trade,
    Recruit,
    Thanks,
    Bye,
    Laugh,
    Lag,
    Local,
    Mission,
    Gf,
    Question,
    Statement,
}

fn has_word(text: &str, words: &[&str]) -> bool {
    text.split(|c: char| !c.is_alphanumeric() && c != '/' && c != '\'')
        .any(|w| words.iter().any(|x| w == *x))
}

fn intent(text: &str) -> Intent {
    let t = text.to_lowercase();
    let has = |needles: &[&str]| needles.iter().any(|n| t.contains(n));
    if has_word(&t, &["gf", "gg", "gfs"]) || has(&["good fight"]) {
        Intent::Gf
    } else if has(&["how are you", "how r u", "how's it going", "hows it going", "how you doing", "how are things", "what's up", "whats up", "wassup", "sup?"]) {
        Intent::HowAreYou
    } else if has(&["what are you doing", "what you doing", "what u doing", "wyd", "what are you up to", "what you up to", "doing anything"]) {
        Intent::Doing
    } else if has(&["you fly", "u fly", "your ship", "ur ship", "what ship", "flying"]) {
        Intent::Ship
    } else if has(&["fit", "setup", "loadout"]) {
        Intent::Fit
    } else if has_word(&t, &["isk", "money", "rich", "poor", "broke", "plex", "isk/h", "wallet"]) {
        Intent::Isk
    } else if has(&["how do i", "how do you", "any tips", "tutorial", "advice", "i'm new", "im new", "new player", "help me", "can someone help", "need help"]) {
        Intent::Help
    } else if has_word(&t, &["fight", "1v1", "duel", "pvp", "gank", "come at me", "kill", "brawl"]) {
        Intent::Fight
    } else if has_word(&t, &["idiot", "stupid", "trash", "suck", "sucks", "loser", "noob", "noobs", "nub", "scrub", "carebear", "bad", "ez", "rekt", "cry", "baby", "clown", "dumb"]) {
        Intent::Insult
    } else if has_word(&t, &["wts", "wtb", "sell", "selling", "buy", "buying", "trade", "contract", "price", "scam"]) {
        Intent::Trade
    } else if has_word(&t, &["corp", "recruit", "recruiting", "alliance", "join", "corporation"]) {
        Intent::Recruit
    } else if has_word(&t, &["thanks", "thx", "ty", "cheers", "thank"]) {
        Intent::Thanks
    } else if has_word(&t, &["bye", "cya", "gn", "goodnight", "later", "laters", "logging", "afk"]) {
        Intent::Bye
    } else if has_word(&t, &["lol", "lmao", "haha", "hahaha", "rofl", "kek", "xd", "lul"]) {
        Intent::Laugh
    } else if has_word(&t, &["lag", "ccp", "server", "bug", "tidi", "dc", "disconnected"]) {
        Intent::Lag
    } else if has_word(&t, &["mission", "missions", "agent", "agents", "lp"]) {
        Intent::Mission
    } else if has_word(&t, &["hi", "hello", "hey", "o7", "o/", "\\o", "yo", "greetings", "howdy", "morning", "evening", "hiya", "heya", "sup"]) {
        Intent::Greeting
    } else if has_word(&t, &["local", "neut", "neutral", "neuts", "red", "reds", "anyone", "belt", "belts", "where", "here"]) {
        Intent::Local
    } else if t.trim_end().ends_with('?') {
        Intent::Question
    } else {
        Intent::Statement
    }
}

/// What a bot last asked a player, so the answer gets a fitting reply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Asked {
    Nothing,
    Ship,
    Activity,
    Time,
}

fn pick<'a>(rng: &mut Rng, lines: &[&'a str]) -> &'a str {
    lines[rng.below(lines.len())]
}

/// A reply from `persona` to a player's line, and what it asked back.
fn reply(p: Persona, i: Intent, rng: &mut Rng) -> (&'static str, Asked) {
    use Persona::*;
    let rough = super::trash_talk();
    let ask = |line: &'static str, asked: Asked| (line, asked);
    match i {
        Intent::Greeting => match p {
            Friendly | Roleplayer => ask(pick(rng, &["o7 {p}", "Hey {p}, fly safe.", "o/ {p}. What brings you out here?", "Evening {p}.", "o7. Welcome to local, {p}."]), Asked::Activity),
            Veteran => (pick(rng, &["o7", "o7 {p}", "{p}.", "o/"]), Asked::Nothing),
            Newbie => ask(pick(rng, &["hi!!", "hey {p}! o/", "hello :)", "hi {p} what are you up to?"]), Asked::Activity),
            Troll => (pick(rng, &["no", "who are you", "hi {p}, can i have your stuff", "bye", "o7 {p} (this is a trap)", "\\o/ {p} we've been expecting you"]), Asked::Nothing),
            Pirate if rough => (pick(rng, &["hi {p}. nice ship. would be a shame if something happened to it", "o7 {p}. aligned yet?", "hello target", "{p}. don't undock.", "hey {p} what are you flying, asking for a friend"]), Asked::Ship),
            Pirate => (pick(rng, &["o7 {p}", "{p}. careful out there."]), Asked::Nothing),
            Carebear => ask(pick(rng, &["hi {p}, you aren't a ganker are you?", "o7 {p}", "hello! peaceful today so far", "hi {p}. mining or ratting?"]), Asked::Activity),
            Trader => (pick(rng, &["o7 {p}, need anything? I've got cheap ammo", "Hi {p}. Buying or selling?", "hey {p}, prices are good today"]), Asked::Nothing),
            Grumpy => (pick(rng, &["what", "hi. don't make it weird", "yeah hi", "{p}.", "o7 I guess"]), Asked::Nothing),
        },
        Intent::HowAreYou => match p {
            Troll => (pick(rng, &["terrible, my cat undocked my titan", "better than your killboard", "how am I what", "fine until you showed up"]), Asked::Nothing),
            Grumpy => (pick(rng, &["lost a ship to gate guns, so great", "tired. CCP nerfed my isk again", "same as always, broke", "don't ask"]), Asked::Nothing),
            Pirate if rough => (pick(rng, &["bored. no targets. you volunteering?", "good, killed three miners this morning", "hungry"]), Asked::Nothing),
            Newbie => ask(pick(rng, &["good! just learned how to warp to 0 lol", "great, made my first million isk today!", "pretty good, still figuring this game out. you?"]), Asked::Time),
            Roleplayer => (pick(rng, &["The void is quiet today, capsuleer. I am well.", "Well enough, by the grace of the Empress.", "Better than the Sansha would like."]), Asked::Nothing),
            _ => ask(pick(rng, &["good, you?", "not bad, just chilling in local", "can't complain. you?", "alright, slow night", "doing alright, {p}. how about you?"]), Asked::Activity),
        },
        Intent::Doing => match p {
            Pirate if rough => (pick(rng, &["watching you", "waiting for someone to undock in a shiny ship", "camping the gate, why", "scanning down your mission"]), Asked::Nothing),
            Troll => (pick(rng, &["afk mining in a titan", "nothing, same as you", "reporting you", "spinning my ship"]), Asked::Nothing),
            Carebear => ask(pick(rng, &["mining, hoping nobody ganks me", "running level 2s", "hauling ore back to station"]), Asked::Activity),
            Trader => (pick(rng, &["undercutting everyone by 0.01", "station trading, the true endgame", "hauling a freighter of tritanium"]), Asked::Nothing),
            Newbie => ask(pick(rng, &["trying to find an asteroid belt lol", "doing the tutorials", "skilling for a cruiser!"]), Asked::Nothing),
            Roleplayer => (pick(rng, &["Charting signatures for the Arch Angels... or against them.", "Contemplating the stars, as all capsuleers should."]), Asked::Nothing),
            _ => ask(pick(rng, &["ratting in the belts", "nothing much, about to undock", "just docked up, you?", "sitting in station spinning my ship"]), Asked::Activity),
        },
        Intent::Ship => match p {
            Pirate if rough => ("something faster than you", Asked::Ship),
            Troll => (pick(rng, &["a titan, obviously", "a shuttle with 8 warp core stabs", "your mom's ship"]), Asked::Nothing),
            _ => ask(pick(rng, &["{ship} right now. what are you in?", "a {ship}, nothing fancy", "{ship}. you?", "flying a {ship} today"]), Asked::Ship),
        },
        Intent::Fit => match p {
            Troll => (pick(rng, &["just fit cargo expanders, trust me", "warp core stabs in every slot", "one of everything"]), Asked::Nothing),
            Veteran | Grumpy => (pick(rng, &["tank first, then guns. always", "check the fitting window, you'll be over CPU anyway", "whatever you do don't fit it like a newbie"]), Asked::Nothing),
            _ => (pick(rng, &["depends what you're doing with it", "check what the guys in corp fly", "more tank never hurt anyone"]), Asked::Nothing),
        },
        Intent::Isk => match p {
            Troll if rough => (pick(rng, &["send me 10 mil and I'll send 20 back", "isk doubling, best deal in local", "just sell your ship and buy a better one"]), Asked::Nothing),
            Trader => (pick(rng, &["buy low, sell high. that's the whole secret", "station trading in a hub beats any mission", "watch the market, people panic sell after wars"]), Asked::Nothing),
            Carebear => (pick(rng, &["mining slowly but it's safe-ish", "missions pay alright if you salvage", "I just mine and sell to buy orders"]), Asked::Nothing),
            Grumpy => (pick(rng, &["what isk", "I'm broke after the last loss", "isk? never heard of it"]), Asked::Nothing),
            Pirate if rough => (pick(rng, &["killing miners and taking the loot", "ransom. 50 mil and you keep your ship"]), Asked::Nothing),
            _ => (pick(rng, &["ratting in low sec pays better than missions", "exploration's good isk if you're careful", "missions and salvage. boring but it works"]), Asked::Nothing),
        },
        Intent::Help => match p {
            Troll if rough => (pick(rng, &["press alt+f4 for a free ship", "self destruct, then ask again", "have you tried turning it off and on"]), Asked::Nothing),
            Grumpy => (pick(rng, &["the tutorials exist for a reason", "read the guide like the rest of us", "fine. what do you need"]), Asked::Nothing),
            Pirate if rough => ("step one: undock. step two: stay still", Asked::Nothing),
            _ => (pick(rng, &["sure, what do you need?", "don't fly what you can't afford to lose", "the agents in station give you a lot to start with", "right-click in space for most things", "insure your ship before you undock", "what's the problem {p}?"]), Asked::Nothing),
        },
        Intent::Fight => match p {
            Pirate if rough => (pick(rng, &["sure, undock", "I'll be at the gate", "bring friends, you'll need them", "come to the belt, I'm waiting", "you sure about that {p}?"]), Asked::Nothing),
            Troll => (pick(rng, &["can't, my mom said no", "fight me in the station", "only if you bring a titan"]), Asked::Nothing),
            Carebear => (pick(rng, &["please no, I just want to mine", "not interested, I'm in a barge", "I'll pass"]), Asked::Nothing),
            Veteran => (pick(rng, &["what are you flying", "maybe later, reshipping", "bring a cruiser and we'll see"]), Asked::Ship),
            _ => (pick(rng, &["not today", "maybe, what are you in?", "I'm not fitted for that"]), Asked::Ship),
        },
        Intent::Insult => match p {
            Pirate if rough => (pick(rng, &["big words from someone in a high sec station", "undock and say that", "noted. you're on the list", "your killboard says otherwise"]), Asked::Nothing),
            Troll if rough => (pick(rng, &["ok", "cry more", "lol", "I'm telling CCP", "reported for being mean", "who asked"]), Asked::Nothing),
            Grumpy => (pick(rng, &["yeah yeah", "whatever", "you're not wrong, this game sucks"]), Asked::Nothing),
            Roleplayer => ("Such words are beneath a capsuleer.", Asked::Nothing),
            Newbie => (pick(rng, &["hey I'm still learning :(", "rude"]), Asked::Nothing),
            _ => (pick(rng, &["calm down", "no need for that {p}", "ok then", "chill"]), Asked::Nothing),
        },
        Intent::Trade => match p {
            Trader => (pick(rng, &["what are you after? I can contract it", "pm me with a price", "I sell for less than Jita, promise", "got ammo, drones and minerals"]), Asked::Nothing),
            Troll if rough => (pick(rng, &["selling a titan for 100 mil, cash only", "wts my soul, 5 isk", "trust me bro, contract is legit"]), Asked::Nothing),
            _ => (pick(rng, &["check the market, it's usually cheaper", "no thanks", "not buying right now"]), Asked::Nothing),
        },
        Intent::Recruit => match p {
            Friendly | Veteran | Roleplayer => (pick(rng, &["my corp's recruiting, apply if you want", "look up our corp, applications are open", "join a corp, it's way more fun with people"]), Asked::Nothing),
            Troll => (pick(rng, &["join my corp, 100% tax, no refunds", "we're recruiting. requirements: titan"]), Asked::Nothing),
            Grumpy => ("corps are just drama with extra steps", Asked::Nothing),
            _ => (pick(rng, &["what are you looking for in a corp?", "lots of corps recruiting around here"]), Asked::Activity),
        },
        Intent::Thanks => (pick(rng, &["np", "anytime", "o7", "no problem {p}", "glad to help"]), Asked::Nothing),
        Intent::Bye => match p {
            Troll => (pick(rng, &["finally", "don't come back", "bye, leaving my stuff to me?"]), Asked::Nothing),
            _ => (pick(rng, &["o7", "fly safe {p}", "cya", "night {p}", "o7 take care"]), Asked::Nothing),
        },
        Intent::Laugh => (pick(rng, &["lol", "haha", "lmao", "XD", "kek"]), Asked::Nothing),
        Intent::Lag => match p {
            Grumpy => (pick(rng, &["blame CCP", "server's held together with tape", "every time"]), Asked::Nothing),
            Troll => ("lag is a skill issue", Asked::Nothing),
            _ => (pick(rng, &["yeah it's bit laggy", "seems fine for me", "server hamsters are tired"]), Asked::Nothing),
        },
        Intent::Local => match p {
            Pirate if rough => (pick(rng, &["nobody here but me", "local's clear, totally safe, undock now", "lots of targets around"]), Asked::Nothing),
            Carebear => (pick(rng, &["watch out, there's a pirate in local", "think it's quiet", "keep an eye on local"]), Asked::Nothing),
            _ => (pick(rng, &["quiet so far", "a few neutrals came through", "belts are busy today", "check the overview"]), Asked::Nothing),
        },
        Intent::Mission => (pick(rng, &["level 2s are easy isk in a cruiser", "the agent in station has a good one up", "read the mission briefing for the damage types", "the bookmark's in your journal"]), Asked::Nothing),
        Intent::Gf => (pick(rng, &["gf", "o7", "gf gf", "gg"]), Asked::Nothing),
        Intent::Question => match p {
            Troll => (pick(rng, &["yes", "no", "maybe", "ask in local"]), Asked::Nothing),
            Veteran => (pick(rng, &["depends", "short answer: yes", "short answer: no, not really", "been like that since 2003"]), Asked::Nothing),
            Newbie => ("no idea, I'm new too lol", Asked::Nothing),
            Grumpy => ("why are you asking me", Asked::Nothing),
            _ => (pick(rng, &["good question", "not sure tbh", "I think so?", "probably", "no idea {p}"]), Asked::Nothing),
        },
        Intent::Statement => match p {
            Troll => (pick(rng, &["ok", "cool story", "source?", "lol", "and?"]), Asked::Nothing),
            Grumpy => (pick(rng, &["sure", "yeah", "whatever you say", "mhm"]), Asked::Nothing),
            Newbie => (pick(rng, &["cool!", "nice", "oh really?"]), Asked::Nothing),
            Roleplayer => (pick(rng, &["Indeed.", "As the stars will it.", "Hm."]), Asked::Nothing),
            _ => (pick(rng, &["true", "yeah", "fair enough", "nice", "heh", "makes sense"]), Asked::Nothing),
        },
    }
}

/// The reply when a player answers what the bot asked.
fn follow_up(asked: Asked, rng: &mut Rng) -> &'static str {
    match asked {
        Asked::Ship => pick(rng, &["nice, solid ship", "good choice. fit it right and it'll last", "careful with that out here", "heh, I lost one of those last week", "respect"]),
        Asked::Activity => pick(rng, &["nice. good luck out there", "fair, watch local while you do", "cool. fly safe", "sounds good"]),
        Asked::Time => pick(rng, &["nice, welcome to the grind", "you'll get there", "plenty to learn, take it slow"]),
        Asked::Nothing => pick(rng, &["yeah", "true", "heh"]),
    }
}

/// Something a bot says with nobody asking: (opener, replies another bot may give).
fn ambient(p: Persona, rng: &mut Rng) -> (&'static str, &'static [&'static str]) {
    use Persona::*;
    let rough = super::trash_talk();
    const ASK_BACK: &[&str] = &["same", "lol", "true", "nah", "yeah", "o7", "agreed", "probably"];
    let options: &[(&str, &[&str])] = match p {
        Friendly => &[
            ("o7 local", &["o7", "o/", "o7 o7"]),
            ("anyone know a good spot to rat around here?", &["belts two jumps out are decent", "try the outer belts", "nope, it's been dead"]),
            ("fly safe everyone", &["o7", "you too"]),
            ("quiet night", &["too quiet", "enjoy it while it lasts", "yeah"]),
            ("anyone want to fleet up for some ratting?", &["sure, x me", "maybe later", "I'm in"]),
        ],
        Veteran => &[
            ("back when I started we didn't have warp to 0", &["ok grandpa", "and you liked it", "here we go"]),
            ("this place used to be a lot busier", &["it'll pick up", "everyone moved to null", "good riddance"]),
            ("never fly what you can't afford to lose", &["words to live by", "tell that to my wallet", "o7"]),
            ("the market's about to move, mark my words", &["sure it is", "on what?", "you said that last week"]),
        ],
        Newbie => &[
            ("how do I get to the belts?", &["right click in space, asteroid belts", "use the overview", "the warp menu, right-click space"]),
            ("is low sec dangerous?", &["yes", "only if you're in it", "very. go anyway, it's fun"]),
            ("just bought my first cruiser!!", &["nice, don't lose it", "gz", "o7 welcome", "insure it"]),
            ("what skills should I train first?", &["navigation and engineering", "drones, always drones", "whatever you want to fly"]),
            ("what does align mean", &["it means getting ready to warp", "point your ship at where you want to go", "it means you're about to die lol"]),
        ],
        Troll => &[
            ("wts kidney, 50 mil, slightly used", &["lol", "how slightly", "I'll give 10"]),
            ("free isk at the gate, just bring your ship", &["sure", "nice try", "lol"]),
            ("local is a lie", &["what", "ok", "deep"]),
            ("who here has the most isk, asking for a scam", &["not me", "lol", "reported"]),
            ("I'm actually CCP, nobody tell anyone", &["sure you are", "reported", "can I have a titan"]),
            ("anyone else's ship made of cardboard?", &["mine is", "only on tuesdays", "lol"]),
        ],
        Pirate if rough => &[
            ("this belt's taxed now. 10 mil per miner", &["who made you boss", "lol no", "pay up I guess"]),
            ("nice of you all to bring ships for me", &["calm down", "lol", "not today"]),
            ("anyone undocking in a barge? asking for a friend", &["nope", "lol", "nice try"]),
            ("gate camp's open for business", &["thanks for the warning", "I'll take another route", "lol"]),
            ("smell that? fresh targets", &["that's just your fit", "ok", "lol"]),
        ],
        Pirate => &[
            ("anyone around for a fight?", &["maybe later", "what are you in?", "no thanks"]),
            ("watch the gates, they're busy", &["noted", "o7", "thanks"]),
        ],
        Carebear => &[
            ("please don't gank me I'm just mining", &["no promises", "you're fine", "lol"]),
            ("ore prices are terrible", &["they always are", "sell to buy orders", "yeah"]),
            ("anyone else running missions for the corp?", &["yeah, level 3s", "not today", "always"]),
            ("is it safe in the belts?", &["for now", "no", "should be"]),
        ],
        Trader => &[
            ("WTS cheap ammo and drones, contract me", &["how cheap", "lol spam", "pass"]),
            ("WTB tritanium, paying above market", &["how much above", "sold out sorry", "I've got some"]),
            ("prices in the hub are crazy today", &["always are", "what's up?", "buy low"]),
            ("selling ships, cheaper than the market, x me", &["what ships", "sure you are", "maybe"]),
        ],
        Roleplayer => &[
            ("The Empire watches over us all.", &["sure it does", "Amarr victor", "lol"]),
            ("May the stars guide your path, capsuleers.", &["o7", "and yours", "ok"]),
            ("The Sansha grow bold in these parts.", &["they always do", "kill them all", "ok"]),
            ("Another day in the void. Another day immortal.", &["deep", "lol", "o7"]),
        ],
        Grumpy => &[
            ("why is everything so expensive", &["inflation", "CCP", "get gud"]),
            ("lost another ship to gate guns", &["rip", "f", "lol"]),
            ("this game was better before", &["it was", "ok grandpa", "every year"]),
            ("CCP pls", &["lol", "never", "same"]),
            ("I hate this system", &["then leave", "same", "lol"]),
        ],
    };
    if options.is_empty() {
        return ("o7", ASK_BACK);
    }
    options[rng.below(options.len())]
}

/// What a bot says in a nearby fight.
fn battle(p: Persona, rng: &mut Rng) -> &'static str {
    let rough = super::trash_talk();
    match p {
        Persona::Pirate | Persona::Troll if rough => pick(rng, &["primary the logi!", "lol run", "you're tackled", "should have aligned", "get rekt", "say goodbye to that ship"]),
        Persona::Carebear | Persona::Newbie => pick(rng, &["help!!", "who's shooting?", "warp out warp out", "oh no", "I'm in structure!"]),
        Persona::Roleplayer => pick(rng, &["For the Empire!", "Stand fast, capsuleers!", "Hold the line!"]),
        Persona::Grumpy => pick(rng, &["of course this happens now", "great, just great", "why is it always me"]),
        _ => pick(rng, &["break their lock and push in!", "that armor is folding, stay on target!", "someone's about to lose a ship", "keep talking in local, I can still hear you over the guns", "nice shots", "watch your cap"]),
    }
}

/// What the loser of a fight says.
fn lost_line(p: Persona, rng: &mut Rng) -> &'static str {
    let rough = super::trash_talk();
    match p {
        Persona::Pirate if rough => pick(rng, &["gf. lucky shot", "gf, I'll be back", "whatever, I have ten more", "gf. enjoy it while it lasts"]),
        Persona::Troll if rough => pick(rng, &["gf, I let you win", "lag", "I was afk", "nice, you killed a t1 frig, congrats", "reported"]),
        Persona::Grumpy => pick(rng, &["ffs", "lag", "of course", "gf I guess", "why do I even play"]),
        Persona::Newbie => pick(rng, &["noooo my ship", "gf? that was fast", "what just happened", "I just bought that :("]),
        Persona::Carebear => pick(rng, &["why would you do that", "gf... I was just mining", "seriously?"]),
        Persona::Roleplayer => pick(rng, &["My clone awaits. Well fought.", "Death is but a doorway for a capsuleer. o7"]),
        _ => pick(rng, &["gf", "gf o7", "gf, you caught me there", "gf. well flown", "gf, nice fit"]),
    }
}

/// What the winner says.
fn won_line(p: Persona, rng: &mut Rng) -> &'static str {
    let rough = super::trash_talk();
    match p {
        Persona::Pirate if rough => pick(rng, &["gf", "ez", "should have aligned", "thanks for the loot", "your fit was bad and you should feel bad", "o7, convo me for fitting advice"]),
        Persona::Troll if rough => pick(rng, &["ez", "lol", "gf (not really)", "get good", "rekt", "thanks for the killmail"]),
        Persona::Grumpy => pick(rng, &["gf", "about time something went right", "finally"]),
        Persona::Roleplayer => pick(rng, &["The Empire's justice is swift.", "Fly well in your next life, capsuleer."]),
        _ => pick(rng, &["gf", "gf o7", "gf, that was close", "good fight", "gf, close one"]),
    }
}

/// What others in Local say about a kill.
fn bystander_line(p: Persona, rng: &mut Rng) -> &'static str {
    let rough = super::trash_talk();
    match p {
        Persona::Troll | Persona::Pirate if rough => pick(rng, &["lol", "rip", "F", "lmao", "ouch", "imagine"]),
        _ => pick(rng, &["o7", "rip", "F", "gf", "oof", "nice kill"]),
    }
}

// ---- memory and the queue ----------------------------------------------------------------------

struct Line {
    due: Instant,
    system: i64,
    bot: i64,
    text: String,
}

struct Partner {
    bot: i64,
    asked: Asked,
    until: Instant,
}

#[derive(Default)]
struct SystemTalk {
    next_ambient: Option<Instant>,
    /// Lines said lately, to avoid saying the same twice in a row.
    recent: VecDeque<String>,
    /// The last time each bot spoke here.
    spoke: HashMap<i64, Instant>,
    last_battle: Option<Instant>,
}

struct Talk {
    rng: Rng,
    queue: Vec<Line>,
    systems: HashMap<i64, SystemTalk>,
    /// Players in conversation with a bot: (system, player) -> bot.
    partners: HashMap<(i64, i64), Partner>,
    /// Names of players who spoke, for bots to address them.
    names: HashMap<i64, String>,
    ships: HashMap<i64, String>,
}

static TALK: Mutex<Option<Talk>> = Mutex::new(None);

fn with_talk<R>(f: impl FnOnce(&mut Talk) -> R) -> Option<R> {
    let mut guard = TALK.lock().ok()?;
    let talk = guard.get_or_insert_with(|| Talk {
        rng: Rng::seeded(crate::bulkdata::filetime_now() as u64 ^ 0x7a1c),
        queue: Vec::new(),
        systems: HashMap::new(),
        partners: HashMap::new(),
        names: HashMap::new(),
        ships: HashMap::new(),
    });
    Some(f(talk))
}

/// How long a line takes to "type".
fn typing(text: &str, rng: &mut Rng) -> Duration {
    let secs = 1.2 + text.chars().count() as f64 * 0.07 * rng.range(0.6, 1.4);
    Duration::from_secs_f64(secs.min(9.0))
}

impl Talk {
    fn say(&mut self, system: i64, bot: i64, persona: Persona, line: &str, after: Duration, player: Option<&str>, ship: &str) {
        let text = line.replace("{p}", player.unwrap_or("mate")).replace("{ship}", ship);
        let text = styled(persona, &text, &mut self.rng);
        let due = Instant::now() + after + typing(&text, &mut self.rng);
        self.queue.push(Line { due, system, bot, text });
    }

    fn ship_of(&self, w: &WorldState, system: i64, bot: i64) -> String {
        w.parks
            .get(&system)
            .and_then(|p| p.ships.values().find(|s| s.pilot.char_id == bot))
            .and_then(|s| self.ships.get(&s.pilot.type_id).cloned())
            .unwrap_or_else(|| ["Rifter", "Vexor", "Caracal", "Drake", "Thorax", "Merlin", "Hurricane"][(bot as usize / 3) % 7].to_string())
    }
}

/// The bots in a system's Local, as Local shows them.
fn bots_in(w: &WorldState, system: i64) -> Vec<LocalBot> {
    let Some(list) = w.bot_local.get(&system) else { return Vec::new() };
    list.iter().filter_map(|i| w.bot_roster.get(*i as usize).cloned()).collect()
}

fn first_name(name: &str) -> &str {
    name.split_whitespace().next().unwrap_or(name)
}

/// The bot a player named in their line, if any.
fn named<'a>(bots: &'a [LocalBot], text: &str) -> Option<&'a LocalBot> {
    let t = text.to_lowercase();
    bots.iter().find(|b| t.contains(&b.name.to_lowercase()))
        .or_else(|| bots.iter().find(|b| {
            let first = first_name(&b.name).to_lowercase();
            first.chars().count() >= 4 && has_word(&t, &[first.as_str()])
        }))
}

// ---- fight calls -------------------------------------------------------------------------------

const GROUP_SUN: i64 = 6;
const GROUP_BELT: i64 = 9;

fn roman(n: usize) -> String {
    let parts = [(10, "X"), (9, "IX"), (5, "V"), (4, "IV"), (1, "I")];
    let (mut n, mut out) = (n, String::new());
    for (v, s) in parts {
        while n >= v {
            out.push_str(s);
            n -= v;
        }
    }
    out
}

/// Whether a line calls for a fight somewhere: "meet at top belt to fight",
/// "1v1 me at belt 3", "come fight me here".
fn fight_call(text: &str) -> bool {
    let t = text.to_lowercase();
    has_word(&t, &["fight", "fights", "pvp", "1v1", "2v2", "duel", "brawl", "gank", "scrap", "kill", "gf", "fite"])
        && (has_word(&t, &["meet", "at", "come", "warp", "belt", "here", "sun"]) || t.contains("belt"))
}

/// Where a fight call points, in the caller's system: a belt (top, last,
/// "belt 3", "IV-2" or its name), the sun, or the caller's ship ("here").
/// Returns the anchor to orbit, the point to warp to, and what to call it.
fn meeting_place(w: &WorldState, system: i64, player: i64, text: &str) -> Option<(i64, crate::ballpark::Vec3, String)> {
    let park = w.parks.get(&system)?;
    let t = text.to_lowercase();
    let mut belts: Vec<&crate::ballpark::Celestial> = park.celestials.iter().filter(|c| c.group_id == GROUP_BELT).collect();
    // The order the overview lists them in: by planet, then belt number
    // ("Jita IV - Asteroid Belt 2").
    belts.sort_by_key(|c| {
        let w: Vec<&str> = c.name.split_whitespace().collect();
        let n = w.last().and_then(|x| x.parse::<u32>().ok()).unwrap_or(0);
        let planet = w.len().checked_sub(5).and_then(|i| (1..=30u32).find(|k| roman(*k as usize).eq_ignore_ascii_case(w[i]))).unwrap_or(0);
        (planet, n, c.id)
    });
    let named = |c: &crate::ballpark::Celestial| (c.id, c.pos, c.name.clone());
    if has_word(&t, &["here", "me"]) && !t.contains("belt") {
        let ship = park.ships.values().find(|s| s.pilot.char_id == player && s.is_player())?;
        return Some((ship.id, ship.pos, "you".into()));
    }
    if has_word(&t, &["sun", "star"]) {
        return park.celestials.iter().find(|c| c.group_id == GROUP_SUN).map(named);
    }
    if !t.contains("belt") && !t.contains("roid") {
        return None;
    }
    // By name: "IV - Asteroid Belt 2", "iv-2", "4-2".
    for b in &belts {
        if t.contains(&b.name.to_lowercase()) {
            return Some(named(b));
        }
        let mut words = b.name.split_whitespace().rev();
        let (Some(belt), Some(planet)) = (words.next(), words.nth(3)) else { continue };
        let numeral = planet.to_lowercase();
        let digits = (1..=30).find(|n| roman(*n).eq_ignore_ascii_case(planet)).map(|n| n.to_string()).unwrap_or_default();
        for p in [numeral.as_str(), digits.as_str()] {
            if !p.is_empty() && (t.contains(&format!("{p}-{belt}")) || t.contains(&format!("{p} - {belt}")) || t.contains(&format!("{p}/{belt}"))) {
                return Some(named(b));
            }
        }
    }
    if belts.is_empty() {
        return None;
    }
    if has_word(&t, &["last", "bottom", "final"]) {
        return belts.last().map(|b| named(b));
    }
    // "belt 3", "belt #3", "3rd belt".
    let words: Vec<&str> = t.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect();
    for (k, w) in words.iter().enumerate() {
        let n = w.trim_end_matches(|c: char| c.is_alphabetic()).parse::<usize>().ok().filter(|_| w.chars().next().is_some_and(|c| c.is_ascii_digit()));
        let near_belt = words.get(k + 1).is_some_and(|x| x.starts_with("belt")) || k > 0 && words[k - 1].starts_with("belt");
        if let Some(n) = n.filter(|n| *n >= 1 && near_belt) {
            return belts.get(n - 1).map(|b| named(b));
        }
    }
    // "top belt", "first belt", or just "the belt".
    belts.first().map(|b| named(b))
}

/// How likely a persona is to answer a fight call.
fn comes_to_fight(p: Persona) -> f64 {
    match p {
        Persona::Pirate => 0.75,
        Persona::Troll => 0.45,
        Persona::Veteran => 0.4,
        Persona::Roleplayer => 0.3,
        Persona::Grumpy => 0.25,
        Persona::Friendly => 0.2,
        Persona::Newbie => 0.15,
        Persona::Trader => 0.05,
        Persona::Carebear => 0.03,
    }
}

/// A player called for a fight: some bots in Local say they're coming and
/// go there; others decline, or laugh. True if the line was a fight call.
fn answer_fight_call(t: &mut Talk, w: &WorldState, system: i64, player: i64, name: &str, text: &str, bots: &[LocalBot]) -> bool {
    if !fight_call(text) {
        return false;
    }
    let Some((anchor, point, place)) = meeting_place(w, system, player, text) else { return false };
    let speak = super::chatter() != Chatter::Off;
    let calm = super::aggression() == super::Aggression::Calm;
    let mut coming: Vec<i64> = Vec::new();
    let mut delay = Duration::from_millis(800);
    let mut order: Vec<&LocalBot> = bots.iter().collect();
    for k in (1..order.len()).rev() {
        order.swap(k, t.rng.below(k + 1));
    }
    let mut spoke = 0;
    for b in order.into_iter().take(40) {
        let p = persona(b.char_id, b.role);
        let rough = super::trash_talk();
        if !calm && coming.len() < 8 && t.rng.unit() < comes_to_fight(p) {
            coming.push(b.char_id);
            if speak && spoke < 4 {
                let line = match p {
                    Persona::Pirate if rough => pick(&mut t.rng, &["omw", "warping. hope you insured it", "on my way, don't run", "coming. bring friends", "be there in a sec {p}"]),
                    Persona::Troll if rough => pick(&mut t.rng, &["omw (not)", "coming, but only to watch", "brb getting popcorn. ok omw", "say less"]),
                    Persona::Roleplayer => pick(&mut t.rng, &["I accept your challenge, capsuleer.", "Then let the stars bear witness. Warping."]),
                    _ => pick(&mut t.rng, &["omw", "x", "warping", "on my way", "see you there {p}", "in"]),
                };
                let ship = t.ship_of(w, system, b.char_id);
                t.say(system, b.char_id, p, line, delay, Some(name), &ship);
                delay += Duration::from_secs(2);
                spoke += 1;
            }
        } else if speak && spoke < 6 && t.rng.unit() < 0.12 {
            let line = match p {
                Persona::Carebear => pick(&mut t.rng, &["no thanks, I'm mining", "please don't fight near my barge"]),
                Persona::Troll if rough => pick(&mut t.rng, &["lol good luck", "nobody's coming", "I'll watch the killmail"]),
                _ => pick(&mut t.rng, &["not today", "can't, docked up", "maybe next time", "pass"]),
            };
            t.say(system, b.char_id, p, line, delay, Some(name), "");
            delay += Duration::from_secs(2);
            spoke += 1;
        }
    }
    if !coming.is_empty() {
        tracing::info!("bots: {} answered {name}'s fight call at {place}", coming.len());
        super::control::ask(super::control::Request::Meet { system: system as i32, anchor, point: (point.x, point.y, point.z), bots: coming });
    } else if speak && spoke == 0 && !bots.is_empty() {
        let b = &bots[t.rng.below(bots.len())];
        let p = persona(b.char_id, b.role);
        let line = if calm { "everyone's calm today, nobody wants a fight" } else { pick(&mut t.rng, &["nobody's biting", "lol no", "not today"]) };
        t.say(system, b.char_id, p, line, delay, Some(name), "");
    }
    t.partners.remove(&(system, player));
    true
}

// ---- what the rest of the server calls ---------------------------------------------------------

/// A player spoke in a system's Local: bots answer, after a moment. A call
/// to fight somewhere brings some of them there.
pub fn player_said(w: &WorldState, system: i64, player: i64, name: &str, text: &str) {
    if !super::is_running() {
        return;
    }
    let bots = bots_in(w, system);
    if bots.is_empty() {
        return;
    }
    let chatter = super::chatter();
    with_talk(|t| {
        t.names.insert(player, name.to_string());
        if answer_fight_call(t, w, system, player, name, text, &bots) || chatter == Chatter::Off {
            return;
        }
        let now = Instant::now();
        let kind = intent(text);
        let addressed = named(&bots, text).cloned();
        let partner = t.partners.get(&(system, player)).filter(|p| p.until > now).map(|p| (p.bot, p.asked));
        let partner_bot = partner.and_then(|(bot, _)| bots.iter().find(|b| b.char_id == bot).cloned());
        // Who answers: the bot named, else the one already talking with them.
        let (speaker, asked_before) = match (addressed, partner_bot) {
            (Some(b), _) => (Some(b), Asked::Nothing),
            (None, Some(b)) if t.rng.unit() < 0.85 => (Some(b), partner.map_or(Asked::Nothing, |p| p.1)),
            _ => {
                let chance = match kind {
                    Intent::Greeting | Intent::HowAreYou | Intent::Help | Intent::Fight | Intent::Insult | Intent::Gf => 0.95,
                    Intent::Question | Intent::Doing | Intent::Ship | Intent::Recruit | Intent::Trade | Intent::Isk | Intent::Mission | Intent::Local => 0.8,
                    Intent::Laugh => 0.25,
                    _ => 0.4,
                } * if chatter == Chatter::Quiet { 0.5 } else if chatter == Chatter::Chatty { 1.3 } else { 1.0 };
                let b = (t.rng.unit() < chance).then(|| bots[t.rng.below(bots.len())].clone());
                (b, Asked::Nothing)
            }
        };
        let Some(speaker) = speaker else { return };
        let p = persona(speaker.char_id, speaker.role);
        let ship = t.ship_of(w, system, speaker.char_id);
        // An answer to what the bot asked gets a fitting reply.
        let (line, asked) = if asked_before != Asked::Nothing && matches!(kind, Intent::Statement | Intent::Ship | Intent::Doing) {
            (follow_up(asked_before, &mut t.rng), Asked::Nothing)
        } else {
            reply(p, kind, &mut t.rng)
        };
        t.say(system, speaker.char_id, p, line, Duration::from_millis(300), Some(name), &ship);
        t.partners.insert((system, player), Partner { bot: speaker.char_id, asked, until: now + Duration::from_secs(240) });
        // Greetings, insults and fights draw a crowd.
        let crowd = match kind {
            Intent::Greeting => 0.55,
            Intent::Insult | Intent::Fight => 0.5,
            Intent::Bye | Intent::Gf => 0.4,
            Intent::Laugh | Intent::Statement => 0.12,
            _ => 0.15,
        } * chatter.rate().min(1.5);
        let mut delay = Duration::from_secs(2);
        for _ in 0..2 {
            if bots.len() < 2 || t.rng.unit() >= crowd {
                break;
            }
            let other = &bots[t.rng.below(bots.len())];
            if other.char_id == speaker.char_id {
                continue;
            }
            let op = persona(other.char_id, other.role);
            let (line, _) = match kind {
                Intent::Statement | Intent::Question => (pick(&mut t.rng, &["lol", "true", "this", "^", "+1", "nah"]), Asked::Nothing),
                _ => reply(op, kind, &mut t.rng),
            };
            let ship = t.ship_of(w, system, other.char_id);
            t.say(system, other.char_id, op, line, delay, Some(name), &ship);
            delay += Duration::from_secs(2);
        }
    });
}

/// A ship was destroyed in `system`: the winner and loser talk, and maybe
/// a bystander. Names and the hull are the caller's to look up.
pub fn ship_lost(w: &WorldState, system: i64, victim: i64, victim_name: &str, killer: Option<i64>, killer_name: Option<&str>, capsule: bool) {
    let chatter = super::chatter();
    if chatter == Chatter::Off || !super::is_running() {
        return;
    }
    let bots = bots_in(w, system);
    if bots.is_empty() {
        return;
    }
    with_talk(|t| {
        let rate = chatter.rate().min(1.5);
        let victim_bot = bots.iter().find(|b| b.char_id == victim).cloned();
        let killer_bot = killer.and_then(|k| bots.iter().find(|b| b.char_id == k)).cloned();
        let mut delay = Duration::from_secs(2);
        // Pods only get a word now and then.
        let speak = if capsule { 0.3 } else { 0.9 };
        if let Some(v) = &victim_bot {
            if t.rng.unit() < speak * rate {
                let p = persona(v.char_id, v.role);
                let line = lost_line(p, &mut t.rng);
                t.say(system, v.char_id, p, line, delay, killer_name, "");
                delay += Duration::from_secs(3);
            }
        }
        if let Some(k) = &killer_bot {
            if t.rng.unit() < speak * rate {
                let p = persona(k.char_id, k.role);
                let line = won_line(p, &mut t.rng);
                t.say(system, k.char_id, p, line, delay, Some(victim_name), "");
                delay += Duration::from_secs(3);
            }
        }
        if t.rng.unit() < 0.45 * rate {
            let b = &bots[t.rng.below(bots.len())];
            if Some(b.char_id) != killer && b.char_id != victim {
                let p = persona(b.char_id, b.role);
                let line = bystander_line(p, &mut t.rng);
                t.say(system, b.char_id, p, line, delay, Some(victim_name), "");
            }
        }
    });
}

/// Delivers due lines, and starts conversations in watched Local channels.
/// Runs for the life of the server.
pub async fn run(world: World, db_url: String) {
    match crate::db::connect(&db_url).await {
        Ok(db) => {
            let sql = r#"SELECT t."typeID", t."typeName" FROM "invTypes" t JOIN "invGroups" g ON g."groupID" = t."groupID" WHERE g."categoryID" = 6"#;
            match db.query(sql, &[]).await {
                Ok(rows) => {
                    let ships: HashMap<i64, String> = rows.iter().map(|r| (r.get::<_, i32>(0) as i64, r.get::<_, String>(1))).collect();
                    with_talk(|t| t.ships = ships);
                }
                Err(e) => warn!("bots: reading ship names for Local chat: {}", crate::logging::chain(&e)),
            }
        }
        Err(e) => warn!("bots: Local chat cannot reach the database: {}", crate::logging::chain(&e)),
    }
    let mut interval = tokio::time::interval(Duration::from_millis(500));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        interval.tick().await;
        tick(&world);
    }
}

fn tick(world: &World) {
    use crate::svc::chat;
    let w = world.lock();
    let now = Instant::now();
    let channels = w.local_channels();
    let due: Vec<Line> = with_talk(|t| {
        let (due, later): (Vec<Line>, Vec<Line>) = std::mem::take(&mut t.queue).into_iter().partition(|l| l.due <= now);
        t.queue = later;
        t.partners.retain(|_, p| p.until > now);
        due
    })
    .unwrap_or_default();
    for line in due {
        let Some((_, channel, members)) = channels.iter().find(|(s, _, m)| *s == line.system && !m.is_empty()) else { continue };
        // Said only by a bot still in that Local.
        let Some(bot) = bots_in(&w, line.system).into_iter().find(|b| b.char_id == line.bot) else { continue };
        let count = chat::channel_population(&w, channel, members.len());
        chat::announce_counted(&w, members, count, channel, "SendMessage", &chat::bot_who(&bot), vec![Value::text(line.text.clone())]);
        with_talk(|t| {
            let s = t.systems.entry(line.system).or_default();
            s.spoke.insert(bot.char_id, now);
            s.recent.push_back(line.text);
            while s.recent.len() > 30 {
                s.recent.pop_front();
            }
        });
    }
    let chatter = super::chatter();
    if chatter == Chatter::Off || !super::is_running() {
        return;
    }
    for (system, _, members) in &channels {
        if members.is_empty() {
            continue;
        }
        let bots = bots_in(&w, *system);
        if bots.len() < 2 {
            continue;
        }
        let fighting = w.parks.get(system).is_some_and(|park| {
            let active: Vec<_> = park.ships.values().filter(|s| s.fight.as_ref().is_some_and(|f| f.modules.iter().any(|m| m.offensive))).collect();
            active.iter().any(|a| active.iter().filter(|b| (a.pos - b.pos).len() <= 200_000.0).count() >= 4)
        });
        with_talk(|t| {
            let rate = chatter.rate();
            let s = t.systems.entry(*system).or_default();
            // A fight nearby: someone shouts now and then.
            if fighting && s.last_battle.is_none_or(|at| now.duration_since(at) > Duration::from_secs_f64(14.0 / rate.max(0.3))) {
                s.last_battle = Some(now);
                let b = bots[t.rng.below(bots.len())].clone();
                let p = persona(b.char_id, b.role);
                let line = battle(p, &mut t.rng);
                t.say(*system, b.char_id, p, line, Duration::ZERO, None, "");
                return;
            }
            let s = t.systems.entry(*system).or_default();
            let next = *s.next_ambient.get_or_insert_with(|| now + Duration::from_secs(20));
            if now < next {
                return;
            }
            // Between one and three minutes at normal chatter, more or less.
            let gap = t.rng.range(60.0, 180.0) / rate.max(0.1);
            t.systems.entry(*system).or_default().next_ambient = Some(now + Duration::from_secs_f64(gap));
            if chatter == Chatter::Quiet && t.rng.unit() < 0.5 {
                return;
            }
            let a = bots[t.rng.below(bots.len())].clone();
            let pa = persona(a.char_id, a.role);
            let (mut opener, mut answers) = ambient(pa, &mut t.rng);
            for _ in 0..3 {
                if !t.systems.get(system).is_some_and(|s| s.recent.iter().any(|r| r.eq_ignore_ascii_case(opener))) {
                    break;
                }
                (opener, answers) = ambient(pa, &mut t.rng);
            }
            t.say(*system, a.char_id, pa, opener, Duration::ZERO, None, "");
            // Sometimes it becomes a conversation.
            let mut delay = Duration::from_secs(3);
            let replies = if t.rng.unit() < 0.65 { 1 } else if t.rng.unit() < 0.5 { 2 } else { 0 };
            for _ in 0..replies {
                let b = bots[t.rng.below(bots.len())].clone();
                if b.char_id == a.char_id {
                    continue;
                }
                let pb = persona(b.char_id, b.role);
                let line = pick(&mut t.rng, answers);
                t.say(*system, b.char_id, pb, line, delay, Some(&a.name), "");
                delay += Duration::from_secs(4);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn players_lines_are_read_for_what_they_mean() {
        assert_eq!(intent("hi"), Intent::Greeting);
        assert_eq!(intent("o7 local"), Intent::Greeting);
        assert_eq!(intent("how are you?"), Intent::HowAreYou);
        assert_eq!(intent("what are you flying"), Intent::Ship);
        assert_eq!(intent("gf"), Intent::Gf);
        assert_eq!(intent("you're all noobs"), Intent::Insult);
        assert_eq!(intent("wts rifter"), Intent::Trade);
        assert_eq!(intent("anyone recruiting? looking for a corp"), Intent::Recruit);
        assert_eq!(intent("lol"), Intent::Laugh);
        assert_eq!(intent("is the market up today?"), Intent::Question);
        assert_eq!(intent("this game is great"), Intent::Statement);
        assert_eq!(intent("this is not a hint"), Intent::Statement);
    }

    #[test]
    fn fight_calls_are_told_from_chatter() {
        assert!(fight_call("meet at top belt to fight"));
        assert!(fight_call("1v1 me at belt 3"));
        assert!(fight_call("come fight me here"));
        assert!(!fight_call("fight me"));
        assert!(!fight_call("the belt is busy"));
        assert_eq!(roman(4), "IV");
        assert_eq!(roman(9), "IX");
    }

    #[test]
    fn a_bot_named_in_a_line_is_found_by_its_first_name() {
        let bot = |id, name: &str| LocalBot { char_id: id, corp_id: 1, alliance_id: None, name: name.into(), type_id: 1373, role: 1 };
        let bots = vec![bot(1, "Kael Darkstar"), bot(2, "Ana Vey")];
        assert_eq!(named(&bots, "hey kael, what's up").map(|b| b.char_id), Some(1));
        assert_eq!(named(&bots, "ana vey you there?").map(|b| b.char_id), Some(2));
        // "Ana" is too short to match on its own; "banana" is not a name.
        assert!(named(&bots, "banana").is_none());
    }

    #[test]
    fn every_persona_has_something_to_say() {
        let mut rng = Rng::seeded(3);
        for p in [Persona::Friendly, Persona::Veteran, Persona::Newbie, Persona::Troll, Persona::Pirate, Persona::Carebear, Persona::Trader, Persona::Roleplayer, Persona::Grumpy] {
            for i in [Intent::Greeting, Intent::HowAreYou, Intent::Insult, Intent::Question, Intent::Statement, Intent::Fight] {
                assert!(!reply(p, i, &mut rng).0.is_empty());
            }
            assert!(!ambient(p, &mut rng).0.is_empty());
            assert!(!lost_line(p, &mut rng).is_empty() && !won_line(p, &mut rng).is_empty());
        }
    }
}
