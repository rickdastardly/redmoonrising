//! How the server is running, for the dashboard: the machine's and this
//! process's CPU and memory, how much of each heartbeat the game loop uses,
//! the database's size and speed, and the bot simulation's load. Sampled
//! every two seconds; the last five minutes are kept for charts.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::{json, Value as Json};
use tracing::warn;

const EVERY: Duration = Duration::from_secs(2);
const KEPT: usize = 150;

// ---- the game loop's own timing ----------------------------------------------------------------

static PULSE_SUM_US: AtomicU64 = AtomicU64::new(0);
static PULSE_COUNT: AtomicU64 = AtomicU64::new(0);
static PULSE_WORST_US: AtomicU64 = AtomicU64::new(0);

/// The heartbeat reports how long one pulse's work took.
pub fn record_pulse(took: Duration) {
    let us = took.as_micros() as u64;
    PULSE_SUM_US.fetch_add(us, Ordering::Relaxed);
    PULSE_COUNT.fetch_add(1, Ordering::Relaxed);
    PULSE_WORST_US.fetch_max(us, Ordering::Relaxed);
}

static BOT_STEP_US_PER_SEC: AtomicU64 = AtomicU64::new(0);
static BOT_WORST_TICK_US: AtomicU64 = AtomicU64::new(0);
static BOT_STEPS_PER_SEC: AtomicU64 = AtomicU64::new(0);

/// The bot simulation reports its last minute: steps a second, time spent
/// stepping each second, and its worst tick.
pub fn record_bots(steps_per_sec: f64, stepping_per_sec: Duration, worst_tick: Duration) {
    BOT_STEPS_PER_SEC.store(steps_per_sec as u64, Ordering::Relaxed);
    BOT_STEP_US_PER_SEC.store(stepping_per_sec.as_micros() as u64, Ordering::Relaxed);
    BOT_WORST_TICK_US.store(worst_tick.as_micros() as u64, Ordering::Relaxed);
}

// ---- the operating system ----------------------------------------------------------------------

/// CPU time so far: the whole machine's busy and total, and this process's,
/// all in the same unit.
#[derive(Clone, Copy, Default)]
struct CpuTimes {
    machine_busy: u64,
    machine_total: u64,
    process: u64,
}

#[derive(Clone, Copy, Default)]
struct Memory {
    total: u64,
    available: u64,
    process: u64,
}

#[cfg(windows)]
mod sys {
    use super::{CpuTimes, Memory};

    #[repr(C)]
    #[derive(Default)]
    struct FileTime {
        low: u32,
        high: u32,
    }

    impl FileTime {
        fn ticks(&self) -> u64 {
            ((self.high as u64) << 32) | self.low as u64
        }
    }

    #[repr(C)]
    #[derive(Default)]
    struct MemoryStatusEx {
        length: u32,
        memory_load: u32,
        total_phys: u64,
        avail_phys: u64,
        total_page_file: u64,
        avail_page_file: u64,
        total_virtual: u64,
        avail_virtual: u64,
        avail_extended_virtual: u64,
    }

    #[repr(C)]
    #[derive(Default)]
    struct ProcessMemoryCounters {
        cb: u32,
        page_fault_count: u32,
        peak_working_set_size: usize,
        working_set_size: usize,
        quota_peak_paged_pool_usage: usize,
        quota_paged_pool_usage: usize,
        quota_peak_non_paged_pool_usage: usize,
        quota_non_paged_pool_usage: usize,
        pagefile_usage: usize,
        peak_pagefile_usage: usize,
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn GetSystemTimes(idle: *mut FileTime, kernel: *mut FileTime, user: *mut FileTime) -> i32;
        fn GetCurrentProcess() -> isize;
        fn GetProcessTimes(process: isize, creation: *mut FileTime, exit: *mut FileTime, kernel: *mut FileTime, user: *mut FileTime) -> i32;
        fn GlobalMemoryStatusEx(status: *mut MemoryStatusEx) -> i32;
        fn K32GetProcessMemoryInfo(process: isize, counters: *mut ProcessMemoryCounters, size: u32) -> i32;
    }

    pub fn cpu() -> Option<CpuTimes> {
        let (mut idle, mut kernel, mut user) = (FileTime::default(), FileTime::default(), FileTime::default());
        let (mut created, mut exited, mut p_kernel, mut p_user) = (FileTime::default(), FileTime::default(), FileTime::default(), FileTime::default());
        // SAFETY: plain out-parameters, sized as the API documents.
        unsafe {
            if GetSystemTimes(&mut idle, &mut kernel, &mut user) == 0 {
                return None;
            }
            if GetProcessTimes(GetCurrentProcess(), &mut created, &mut exited, &mut p_kernel, &mut p_user) == 0 {
                return None;
            }
        }
        // The system's kernel time includes its idle time.
        let total = kernel.ticks() + user.ticks();
        Some(CpuTimes { machine_busy: total.saturating_sub(idle.ticks()), machine_total: total, process: p_kernel.ticks() + p_user.ticks() })
    }

    pub fn memory() -> Option<Memory> {
        let mut status = MemoryStatusEx { length: std::mem::size_of::<MemoryStatusEx>() as u32, ..Default::default() };
        let mut counters = ProcessMemoryCounters { cb: std::mem::size_of::<ProcessMemoryCounters>() as u32, ..Default::default() };
        // SAFETY: the structures carry their own sizes, as the API requires.
        unsafe {
            if GlobalMemoryStatusEx(&mut status) == 0 {
                return None;
            }
            if K32GetProcessMemoryInfo(GetCurrentProcess(), &mut counters, counters.cb) == 0 {
                return None;
            }
        }
        Some(Memory { total: status.total_phys, available: status.avail_phys, process: counters.working_set_size as u64 })
    }
}

#[cfg(not(windows))]
mod sys {
    //! Linux (the Docker image): /proc.
    use super::{CpuTimes, Memory};

    pub fn cpu() -> Option<CpuTimes> {
        let stat = std::fs::read_to_string("/proc/stat").ok()?;
        let fields: Vec<u64> = stat.lines().next()?.split_whitespace().skip(1).filter_map(|f| f.parse().ok()).collect();
        let total: u64 = fields.iter().sum();
        let idle = fields.get(3).copied().unwrap_or(0) + fields.get(4).copied().unwrap_or(0);
        let own = std::fs::read_to_string("/proc/self/stat").ok()?;
        let after = own.rsplit_once(')')?.1;
        let parts: Vec<&str> = after.split_whitespace().collect();
        let process = parts.get(11)?.parse::<u64>().ok()? + parts.get(12)?.parse::<u64>().ok()?;
        Some(CpuTimes { machine_busy: total.saturating_sub(idle), machine_total: total, process })
    }

    pub fn memory() -> Option<Memory> {
        let info = std::fs::read_to_string("/proc/meminfo").ok()?;
        let kb = |key: &str| info.lines().find(|l| l.starts_with(key)).and_then(|l| l.split_whitespace().nth(1)?.parse::<u64>().ok()).map(|v| v * 1024);
        let status = std::fs::read_to_string("/proc/self/status").ok()?;
        let rss = status.lines().find(|l| l.starts_with("VmRSS:")).and_then(|l| l.split_whitespace().nth(1)?.parse::<u64>().ok()).unwrap_or(0) * 1024;
        Some(Memory { total: kb("MemTotal:")?, available: kb("MemAvailable:")?, process: rss })
    }
}

// ---- samples -----------------------------------------------------------------------------------

#[derive(Clone, Copy, Default)]
struct Sample {
    at_ms: i64,
    machine_cpu: f64,
    process_cpu: f64,
    memory_used: u64,
    memory_total: u64,
    process_memory: u64,
    /// Share of the heartbeat's period spent working, and the worst pulse.
    tick_load: f64,
    tick_worst_ms: f64,
    db_ms: f64,
}

#[derive(Default)]
struct Health {
    samples: VecDeque<Sample>,
    db_bytes: i64,
    db_connections: i64,
}

static HEALTH: Mutex<Option<Health>> = Mutex::new(None);

fn unix_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

/// Samples for the life of the server.
pub async fn run(db_url: String, pulse: Duration) {
    let db = match crate::db::connect(&db_url).await {
        Ok(c) => Some(c),
        Err(e) => {
            warn!("health: cannot reach the database: {}", crate::logging::chain(&e));
            None
        }
    };
    let mut interval = tokio::time::interval(EVERY);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut last = sys::cpu();
    let mut rounds = 0u64;
    loop {
        interval.tick().await;
        let now = sys::cpu();
        let (machine_cpu, process_cpu) = match (last, now) {
            (Some(a), Some(b)) => {
                let total = b.machine_total.saturating_sub(a.machine_total).max(1) as f64;
                let busy = b.machine_busy.saturating_sub(a.machine_busy) as f64;
                // Both in the same unit (100 ns on Windows, clock ticks on
                // Linux), the machine's summed over every core.
                let own = b.process.saturating_sub(a.process) as f64;
                (100.0 * busy / total, (100.0 * own / total).min(100.0))
            }
            _ => (0.0, 0.0),
        };
        last = now;
        let memory = sys::memory().unwrap_or_default();
        let count = PULSE_COUNT.swap(0, Ordering::Relaxed);
        let sum = PULSE_SUM_US.swap(0, Ordering::Relaxed);
        let worst = PULSE_WORST_US.swap(0, Ordering::Relaxed);
        let tick_load = if count > 0 { 100.0 * (sum as f64 / count as f64) / pulse.as_micros() as f64 } else { 0.0 };
        let mut db_ms = 0.0;
        let (mut db_bytes, mut db_connections) = (None, None);
        if let Some(db) = &db {
            let started = Instant::now();
            if db.query_one("SELECT 1", &[]).await.is_ok() {
                db_ms = started.elapsed().as_secs_f64() * 1000.0;
            }
            if rounds % 15 == 0 {
                if let Ok(row) = db.query_one("SELECT pg_database_size(current_database()), (SELECT count(*) FROM pg_stat_activity WHERE datname = current_database())", &[]).await {
                    db_bytes = Some(row.get::<_, i64>(0));
                    db_connections = Some(row.get::<_, i64>(1));
                }
            }
        }
        rounds += 1;
        let sample = Sample {
            at_ms: unix_ms(),
            machine_cpu,
            process_cpu,
            memory_used: memory.total.saturating_sub(memory.available),
            memory_total: memory.total,
            process_memory: memory.process,
            tick_load,
            tick_worst_ms: worst as f64 / 1000.0,
            db_ms,
        };
        if let Ok(mut guard) = HEALTH.lock() {
            let h = guard.get_or_insert_with(Health::default);
            h.samples.push_back(sample);
            while h.samples.len() > KEPT {
                h.samples.pop_front();
            }
            if let Some(b) = db_bytes {
                h.db_bytes = b;
            }
            if let Some(c) = db_connections {
                h.db_connections = c;
            }
        }
    }
}

fn round1(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}

/// The latest sample and the history, for `/api/health`.
pub fn report() -> Json {
    let guard = HEALTH.lock().ok();
    let Some(h) = guard.as_ref().and_then(|g| g.as_ref()) else { return json!({ "ready": false }) };
    let Some(now) = h.samples.back().copied() else { return json!({ "ready": false }) };
    let mb = |b: u64| (b as f64 / 1_048_576.0).round();
    let history: Vec<Json> = h
        .samples
        .iter()
        .map(|s| json!([s.at_ms, round1(s.machine_cpu), round1(s.process_cpu), mb(s.process_memory), round1(s.tick_load), round1(s.db_ms)]))
        .collect();
    let bot_load = BOT_STEP_US_PER_SEC.load(Ordering::Relaxed) as f64 / 10_000.0;
    json!({
        "ready": true,
        "cpu": { "machine": round1(now.machine_cpu), "server": round1(now.process_cpu),
                 "cores": std::thread::available_parallelism().map_or(1, |n| n.get()) },
        "memory": { "serverMb": mb(now.process_memory), "usedMb": mb(now.memory_used), "totalMb": mb(now.memory_total),
                    "usedPercent": if now.memory_total > 0 { round1(100.0 * now.memory_used as f64 / now.memory_total as f64) } else { 0.0 } },
        "tick": { "loadPercent": round1(now.tick_load), "worstMs": round1(now.tick_worst_ms) },
        "database": { "pingMs": round1(now.db_ms), "sizeMb": mb(h.db_bytes.max(0) as u64), "connections": h.db_connections },
        "bots": { "loadPercent": round1(bot_load), "stepsPerSecond": BOT_STEPS_PER_SEC.load(Ordering::Relaxed),
                  "worstTickMs": round1(BOT_WORST_TICK_US.load(Ordering::Relaxed) as f64 / 1000.0) },
        "history": history,
    })
}
