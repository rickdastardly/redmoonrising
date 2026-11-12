use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use serde::Deserialize;

/// The client's own GM roles: ROLE_GML, ROLE_GMH, ROLE_PETITIONEE and
/// ROLE_LEGIONEER, which between them open slash commands and petitions.
pub const GM_ROLES: i64 = 8 | 16 | 256 | 262_144;

/// Staff role bits used by the Blood-era client's chat colour picker.
/// Its priority is QA (blue), GMH (red), GML (yellow), Centurion (green),
/// Legionnaire (cyan). A plain account has none of these bits.
pub const STAFF_ROLE_BITS: i64 = GM_ROLES | 2_048 | 8_192 | 32;

pub fn staff_role(name: &str) -> Option<i64> {
    match name {
        "Player" => Some(0),
        "ISD" => Some(262_144),
        "ISD BH" => Some(2_048),
        "BH" => Some(8),
        "GM" => Some(GM_ROLES),
        "CCP" => Some(GM_ROLES | 8_192 | 32),
        _ => None,
    }
}

pub fn staff_role_name(role: i64) -> &'static str {
    if role & 8_192 != 0 { "CCP" }
    else if role & 16 != 0 { "GM" }
    else if role & 8 != 0 { "BH" }
    else if role & 2_048 != 0 { "ISD BH" }
    else if role & 262_144 != 0 { "ISD" }
    else { "Player" }
}

/// Server settings: from the YAML config file (`RMR_CONFIG`, else
/// `rmr.yml` in the working directory), with any `RMR_*` environment
/// variable taking precedence over the file.
#[derive(Debug, Clone)]
pub struct Config {
    /// `RMR_BIND`: the address the client connects to.
    pub bind: String,
    /// `RMR_DATABASE`: a PostgreSQL connection URL.
    pub database: String,
    /// `RMR_NODE_ID`: this node's machoNet ID, also the low digits of every clientID.
    pub node_id: i64,
    /// `RMR_STRINGS`: the client's shared marshal string table.
    pub strings: PathBuf,
    /// `RMR_DUMP_DIR`: where undecodable frames are saved.
    pub dump_dir: PathBuf,
    /// `RMR_ALLIANCE_JOIN_DELAY`: seconds from an alliance accepting a
    /// corporation to the membership taking effect (the client says 24 hours).
    pub alliance_join_delay_secs: i64,
    /// `RMR_STARBASE_SPEEDUP`: how many times faster than the client's own
    /// timers structures anchor, online and unanchor (1: as the client says).
    pub starbase_speedup: f64,
    /// `RMR_SOV_CLAIM_DELAY`: seconds a tower's claim must stand before it
    /// counts toward sovereignty (the claim tab says up to six days).
    pub sov_claim_delay_secs: i64,
    /// `RMR_BOTS`: how many bot characters to play (0: none). They are made
    /// the first time and kept.
    pub bots: usize,
    /// `RMR_BOTS_SPEED`: how many times faster than real time the bots live.
    pub bots_speed: f64,
    /// `RMR_BOTS_SHOWN`: the most bot ships shown in one system at once.
    pub bots_shown: usize,
    /// `RMR_BOTS_PIRATES_ATTACK`: whether bot pirates attack players on
    /// sight outside high-sec (those a player has angered do anyway).
    pub bots_pirates_attack: bool,
    /// `RMR_BOTS_CONTROLS`: whether the web dashboard may steer the bots
    /// (start events and wars, set region density).
    pub bots_controls: bool,
    /// `RMR_BOTS_AGGRESSION`: whom bots attack on sight (calm, some or all);
    /// the dashboard can change it while the server runs.
    pub bots_aggression: crate::bots::Aggression,
    /// The rest of the bots' behaviour the dashboard can change live.
    pub bots_settings: crate::bots::Settings,
    /// `RMR_WEB`: where the web dashboard listens (empty: no dashboard).
    pub web: String,
    /// `RMR_STARTING_ISK`: what a new character starts with.
    pub starting_isk: f64,
    /// `RMR_AUTO_CREATE_ACCOUNTS`: whether logging in with an unknown name
    /// makes that account, with the password given.
    pub auto_create_accounts: bool,
    /// `RMR_LOG` (or `RUST_LOG`): off, error, warn, info, debug or trace, or a
    /// filter of its own (see `logging`).
    pub log_level: String,
    /// `RMR_LOG_FILE`: a file that also gets every line logged.
    pub log_file: Option<PathBuf>,
    /// Accounts the file defines; they are made or brought into line with it
    /// at every start. Accounts it does not name are left as they are.
    pub accounts: Vec<Account>,
}

/// An account as the config file gives it.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Account {
    pub username: String,
    pub password: String,
    /// GM powers: slash commands, petitions.
    #[serde(default)]
    pub gm: bool,
    /// A banned account cannot log in.
    #[serde(default)]
    pub banned: bool,
    /// What this account's new characters start with, instead of the
    /// server's `startingIsk`.
    #[serde(default)]
    pub starting_isk: Option<f64>,
    /// Extra role bits, for those who know the client's roles; added to the
    /// GM ones if `gm` is set.
    #[serde(default)]
    pub role: Option<i64>,
}

impl Account {
    /// The account's role column.
    pub fn role(&self) -> i64 {
        self.role.unwrap_or(0) | if self.gm { GM_ROLES } else { 0 }
    }
}

// ---- the file --------------------------------------------------------------------------

#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
struct File {
    server: ServerSection,
    log: LogSection,
    game: GameSection,
    bots: BotsSection,
    accounts: Vec<Account>,
}

#[derive(Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
struct LogSection {
    level: String,
    /// Empty: no log file.
    file: String,
}

impl Default for LogSection {
    fn default() -> Self {
        LogSection { level: "debug".into(), file: String::new() }
    }
}

#[derive(Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
struct ServerSection {
    bind: String,
    web: String,
    database: String,
    node_id: i64,
    auto_create_accounts: bool,
    strings: Option<PathBuf>,
    dump_dir: PathBuf,
}

#[derive(Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
struct GameSection {
    starting_isk: f64,
    alliance_join_delay_seconds: i64,
    starbase_speedup: f64,
    sov_claim_delay_seconds: i64,
}

#[derive(Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
struct BotsSection {
    count: usize,
    speed: f64,
    shown_per_system: usize,
    pirates_attack_players: bool,
    dashboard_controls: bool,
    aggression: String,
    chat: String,
    trash_talk: bool,
    join_player_corps: String,
    join_player_alliances: String,
    accept_applications: String,
    online_activity: f64,
}

impl Default for ServerSection {
    fn default() -> Self {
        ServerSection {
            bind: "0.0.0.0:26000".into(),
            web: "0.0.0.0:8080".into(),
            database: "postgres://rmr:rmr@127.0.0.1:55432/rmr".into(),
            node_id: 1,
            auto_create_accounts: false,
            strings: None,
            dump_dir: ".".into(),
        }
    }
}

impl Default for GameSection {
    fn default() -> Self {
        GameSection { starting_isk: 5_000.0, alliance_join_delay_seconds: 86_400, starbase_speedup: 1.0, sov_claim_delay_seconds: 518_400 }
    }
}

impl Default for BotsSection {
    fn default() -> Self {
        BotsSection { count: 0, speed: 1.0, shown_per_system: 120, pirates_attack_players: true, dashboard_controls: false, aggression: "some".into(),
            chat: "normal".into(), trash_talk: true, join_player_corps: "normal".into(), join_player_alliances: "normal".into(),
            accept_applications: "selective".into(), online_activity: 1.0 }
    }
}

impl Config {
    /// Reads the config file, writing one with the defaults if there is
    /// none, then lets the environment override it. Returns the config and
    /// what happened to the file, for the log.
    pub fn load() -> Result<(Self, String), String> {
        let path = file_path();
        let (file, note) = read(&path)?;
        let bots_aggression = crate::bots::Aggression::parse(&file.bots.aggression)
            .ok_or_else(|| format!("{}: bots.aggression must be calm, some or all, got {:?}", path.display(), file.bots.aggression))?;
        let bad = |key: &str, choices: &str, got: &str| format!("{}: bots.{key} must be {choices}, got {got:?}", path.display());
        let bots_settings = crate::bots::Settings {
            chatter: crate::bots::Chatter::parse(&file.bots.chat).ok_or_else(|| bad("chat", "off, quiet, normal or chatty", &file.bots.chat))?,
            trash_talk: file.bots.trash_talk,
            recruiting: crate::bots::Rate::parse(&file.bots.join_player_corps)
                .ok_or_else(|| bad("joinPlayerCorps", "off, low, normal or high", &file.bots.join_player_corps))?,
            alliance_applying: crate::bots::Rate::parse(&file.bots.join_player_alliances)
                .ok_or_else(|| bad("joinPlayerAlliances", "off, low, normal or high", &file.bots.join_player_alliances))?,
            door: crate::bots::Door::parse(&file.bots.accept_applications)
                .ok_or_else(|| bad("acceptApplications", "closed, selective or open", &file.bots.accept_applications))?,
            activity: file.bots.online_activity.clamp(crate::bots::ACTIVITY_RANGE.0, crate::bots::ACTIVITY_RANGE.1),
        };
        let mut c = Config {
            bind: file.server.bind,
            database: file.server.database,
            node_id: file.server.node_id,
            strings: file.server.strings.unwrap_or_else(|| concat!(env!("CARGO_MANIFEST_DIR"), "/../eve-marshal/tests/data/strings.txt").into()),
            dump_dir: file.server.dump_dir,
            alliance_join_delay_secs: file.game.alliance_join_delay_seconds,
            starbase_speedup: file.game.starbase_speedup,
            sov_claim_delay_secs: file.game.sov_claim_delay_seconds,
            bots: file.bots.count,
            bots_speed: file.bots.speed,
            bots_shown: file.bots.shown_per_system,
            bots_pirates_attack: file.bots.pirates_attack_players,
            bots_controls: file.bots.dashboard_controls,
            bots_aggression,
            bots_settings,
            web: file.server.web,
            starting_isk: file.game.starting_isk,
            auto_create_accounts: file.server.auto_create_accounts,
            log_level: file.log.level,
            log_file: Some(file.log.file).filter(|f| !f.trim().is_empty()).map(PathBuf::from),
            accounts: file.accounts,
        };
        if let Some(v) = var("RMR_LOG").or_else(|| var("RUST_LOG")) {
            c.log_level = v;
        }
        if let Some(v) = var("RMR_LOG_FILE") {
            c.log_file = Some(v.into());
        }
        if let Some(v) = var("RMR_BIND") {
            c.bind = v;
        }
        if let Some(v) = var("RMR_DATABASE") {
            c.database = v;
        }
        if let Some(v) = var("RMR_STRINGS") {
            c.strings = v.into();
        }
        if let Some(v) = var("RMR_DUMP_DIR") {
            c.dump_dir = v.into();
        }
        if let Some(v) = var("RMR_WEB") {
            c.web = v;
        }
        number("RMR_NODE_ID", &mut c.node_id)?;
        number("RMR_ALLIANCE_JOIN_DELAY", &mut c.alliance_join_delay_secs)?;
        number("RMR_STARBASE_SPEEDUP", &mut c.starbase_speedup)?;
        number("RMR_SOV_CLAIM_DELAY", &mut c.sov_claim_delay_secs)?;
        number("RMR_BOTS", &mut c.bots)?;
        number("RMR_BOTS_SPEED", &mut c.bots_speed)?;
        number("RMR_BOTS_SHOWN", &mut c.bots_shown)?;
        number("RMR_BOTS_PIRATES_ATTACK", &mut c.bots_pirates_attack)?;
        number("RMR_BOTS_CONTROLS", &mut c.bots_controls)?;
        if let Some(v) = var("RMR_BOTS_AGGRESSION") {
            c.bots_aggression = crate::bots::Aggression::parse(&v)
                .ok_or_else(|| format!("RMR_BOTS_AGGRESSION must be calm, some or all, got {v:?}"))?;
        }
        number("RMR_STARTING_ISK", &mut c.starting_isk)?;
        number("RMR_AUTO_CREATE_ACCOUNTS", &mut c.auto_create_accounts)?;
        if !(0..10_000_000).contains(&c.node_id) {
            return Err(format!("the node ID must be in 0..10000000, got {}", c.node_id));
        }
        let mut names: Vec<String> = c.accounts.iter().map(|a| a.username.to_lowercase()).collect();
        names.sort();
        if let Some(w) = names.windows(2).find(|w| w[0] == w[1]) {
            return Err(format!("{}: the account {:?} is listed twice", path.display(), w[0]));
        }
        if let Some(a) = c.accounts.iter().find(|a| a.username.trim().is_empty() || a.password.is_empty()) {
            return Err(format!("{}: every account needs a username and a password (see {:?})", path.display(), a.username));
        }
        Ok((c, note))
    }

    /// The database URL with any password masked, for logging.
    pub fn database_redacted(&self) -> String {
        let url = &self.database;
        let (Some(scheme_end), Some(at)) = (url.find("://"), url.rfind('@')) else {
            return url.clone();
        };
        let creds = &url[scheme_end + 3..at];
        match creds.split_once(':') {
            Some((user, _)) => format!("{}{user}:***{}", &url[..scheme_end + 3], &url[at..]),
            None => url.clone(),
        }
    }
}

/// The file, or the defaults (written out, where that can be done) if
/// there is none.
fn read(path: &Path) -> Result<(File, String), String> {
    match std::fs::read_to_string(path) {
        Ok(text) => {
            let file = parse(&text).map_err(|e| format!("{}: {e}", path.display()))?;
            Ok((file, format!("config from {}", path.display())))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let file = parse(DEFAULT_FILE).map_err(|e| format!("the built-in config: {e}"))?;
            let note = match std::fs::write(path, DEFAULT_FILE) {
                Ok(()) => format!("no config file; wrote the defaults to {}", path.display()),
                Err(e) => format!("no config file at {}, and it cannot be written ({e}); using the defaults", path.display()),
            };
            Ok((file, note))
        }
        Err(e) => Err(format!("cannot read {}: {e}", path.display())),
    }
}

/// The file written where there is none: every setting at its default,
/// each explained.
const DEFAULT_FILE: &str = include_str!("rmr.default.yml");

/// An empty file, or one all comments, is every default.
fn parse(text: &str) -> Result<File, serde_yaml_ng::Error> {
    if text.lines().all(|l| l.trim().is_empty() || l.trim_start().starts_with('#')) {
        return Ok(File::default());
    }
    serde_yaml_ng::from_str(text)
}

/// Where the config file is: `RMR_CONFIG`, else `rmr.yml` here.
pub fn file_path() -> PathBuf {
    PathBuf::from(var("RMR_CONFIG").unwrap_or_else(|| "rmr.yml".into()))
}

/// Whether an environment variable overrides a setting, so a change to
/// the file would not take effect.
pub fn overridden(key: &str) -> bool {
    var(key).is_some_and(|v| !v.trim().is_empty())
}

/// The bot count the config file asks for (what the next start plays,
/// unless `RMR_BOTS` overrides it).
pub fn bots_in_file() -> Result<usize, String> {
    let path = file_path();
    read(&path).map(|(file, _)| file.bots.count)
}

/// Sets one `bots:` setting in the config file (for the dashboard), keeping
/// every comment and other line as it is. A missing setting is added.
pub fn set_bots_setting(key: &str, value: &str) -> Result<(), String> {
    let path = file_path();
    let text = std::fs::read_to_string(&path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let updated = with_bots_setting(&text, key, value);
    parse(&updated).map_err(|e| format!("the updated {} would not be valid: {e}", path.display()))?;
    std::fs::write(&path, updated).map_err(|e| format!("cannot write {}: {e}", path.display()))
}

fn with_bots_setting(text: &str, key: &str, value: &str) -> String {
    let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    let section = lines.iter().position(|l| l.trim_end() == "bots:");
    let Some(start) = section else {
        // No bots section: add one at the end.
        let mut out = text.trim_end().to_string();
        out.push_str(&format!("{newline}{newline}bots:{newline}  {key}: {value}{newline}"));
        return out;
    };
    // The section runs until the next top-level line that is not a comment.
    let end = (start + 1..lines.len())
        .find(|&i| {
            let l = &lines[i];
            !l.is_empty() && !l.starts_with(' ') && !l.starts_with('\t') && !l.starts_with('#')
        })
        .unwrap_or(lines.len());
    let prefix = format!("{key}:");
    match (start + 1..end).find(|&i| lines[i].trim_start().starts_with(&prefix) && lines[i].starts_with(' ')) {
        Some(i) => {
            let indent = lines[i].len() - lines[i].trim_start().len();
            lines[i] = format!("{}{key}: {value}", " ".repeat(indent));
        }
        None => {
            // After the section's last setting, before any trailing comments.
            let last = (start + 1..end).rev().find(|&i| lines[i].starts_with(' ') && !lines[i].trim_start().starts_with('#')).unwrap_or(start);
            lines.insert(last + 1, format!("  {key}: {value}"));
        }
    }
    let mut out = lines.join(newline);
    if text.ends_with('\n') {
        out.push_str(newline);
    }
    out
}

fn var(key: &str) -> Option<String> {
    std::env::var(key).ok()
}

fn number<T: std::str::FromStr>(key: &str, into: &mut T) -> Result<(), String> {
    if let Some(v) = var(key) {
        *into = v.trim().parse().map_err(|_| format!("{key} is not a valid value: {v:?}"))?;
    }
    Ok(())
}

// ---- rules the services read -----------------------------------------------------------

/// What character creation and login need of the config, set once at start.
pub struct Rules {
    pub starting_isk: f64,
    /// By accountID, where an account has its own.
    pub account_isk: HashMap<i64, f64>,
    pub auto_create_accounts: bool,
}

static RULES: OnceLock<Rules> = OnceLock::new();

pub fn set_rules(rules: Rules) {
    let _ = RULES.set(rules);
}

/// What a new character on this account starts with.
pub fn starting_isk(account_id: i64) -> f64 {
    RULES.get().map_or(GameSection::default().starting_isk, |r| r.account_isk.get(&account_id).copied().unwrap_or(r.starting_isk))
}

pub fn auto_create_accounts() -> bool {
    RULES.get().is_some_and(|r| r.auto_create_accounts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dashboard_staff_roles_have_distinct_native_chat_colours() {
        let names = ["Player", "ISD", "ISD BH", "BH", "GM", "CCP"];
        let roles: Vec<i64> = names.iter().map(|name| staff_role(name).unwrap()).collect();
        for (name, role) in names.iter().zip(&roles) {
            assert_eq!(staff_role_name(*role), *name);
        }
        assert_eq!(roles.len(), roles.iter().collect::<std::collections::HashSet<_>>().len());
        assert!(staff_role("Owner").is_none());
    }

    fn with_db(url: &str) -> Config {
        Config {
            bind: String::new(),
            database: url.to_string(),
            node_id: 1,
            strings: PathBuf::new(),
            dump_dir: PathBuf::new(),
            alliance_join_delay_secs: 0,
            starbase_speedup: 1.0,
            sov_claim_delay_secs: 0,
            bots: 0,
            bots_speed: 1.0,
            bots_shown: 120,
            bots_pirates_attack: true,
            bots_controls: false,
            bots_aggression: crate::bots::Aggression::Usual,
            bots_settings: crate::bots::Settings::default(),
            web: String::new(),
            starting_isk: 0.0,
            auto_create_accounts: false,
            log_level: String::new(),
            log_file: None,
            accounts: Vec::new(),
        }
    }

    #[test]
    fn the_password_is_masked_in_the_logged_url() {
        assert_eq!(
            with_db("postgres://rmr:secret@127.0.0.1:5432/rmr").database_redacted(),
            "postgres://rmr:***@127.0.0.1:5432/rmr"
        );
        assert_eq!(
            with_db("postgres://127.0.0.1/rmr").database_redacted(),
            "postgres://127.0.0.1/rmr"
        );
    }

    #[test]
    fn a_partial_file_takes_defaults_for_the_rest() {
        let file = parse("game:\n  startingIsk: 1000000000\naccounts:\n  - { username: a, password: b, gm: true }\n").unwrap();
        assert_eq!(file.game.starting_isk, 1e9);
        assert_eq!(file.game.sov_claim_delay_seconds, 518_400);
        assert_eq!(file.server.bind, "0.0.0.0:26000");
        assert_eq!(file.accounts[0].role(), GM_ROLES);
    }

    #[test]
    fn a_misspelt_setting_is_an_error() {
        assert!(parse("game:\n  startingISK: 1\n").is_err());
    }

    #[test]
    fn the_written_defaults_are_the_defaults() {
        let file = parse(DEFAULT_FILE).unwrap();
        let none = File::default();
        assert_eq!(file.server.bind, none.server.bind);
        assert_eq!(file.server.database, none.server.database);
        assert_eq!(file.log.level, none.log.level);
        assert_eq!(file.game.starting_isk, none.game.starting_isk);
        assert_eq!(file.game.sov_claim_delay_seconds, none.game.sov_claim_delay_seconds);
        assert_eq!(file.bots.count, none.bots.count);
        assert_eq!(file.accounts.len(), 1);
        assert!(parse("# nothing\n\n").is_ok());
    }

    #[test]
    fn a_bots_setting_changes_only_its_own_line() {
        let text = "game:\n  count: 7\n\nbots:\n  # How many.\n  count: 100\n  speed: 1\n\n# Accounts\naccounts: []\n";
        let out = with_bots_setting(text, "count", "250");
        assert_eq!(out, text.replace("  count: 100", "  count: 250"));
        let added = with_bots_setting(text, "aggression", "calm");
        assert!(added.contains("  speed: 1\n  aggression: calm\n\n# Accounts"));
        assert_eq!(parse(&added).unwrap().bots.aggression, "calm");
        let crlf = with_bots_setting(&text.replace('\n', "\r\n"), "count", "5");
        assert!(crlf.contains("  count: 5\r\n") && !crlf.replace("\r\n", "").contains('\n'));
    }

    #[test]
    fn off_is_a_level_not_false() {
        assert_eq!(parse("log:\n  level: off\n").unwrap().log.level, "off");
    }
}
