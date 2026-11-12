// The dashboard: polls the server's JSON and fills the windows in, in the
// manner of the EVE client: overview-style lists sorted by any column, the
// game's security colours, [CORP] and <ALLIANCE> tickers, EVE time.

interface Summary {
  players: number;
  bots: { total: number; online: number; inSpace: number; running: boolean };
  corporations: number;
  alliances: number;
  wars: number;
  lostInFights: number;
  systemsHeld: number;
  uptimeSeconds: number;
}

interface War {
  declaredBy: string;
  declaredByTicker: string;
  against: string;
  againstTicker: string;
  declared: number | null;
  ends: number | null;
  mutual: boolean;
  bots: boolean;
  losses: number;
}

interface Player {
  id: number;
  name: string;
  ticker: string | null;
  alliance: string | null;
  ship: string | null;
  system?: string;
  systemSecurity?: number;
  docked?: boolean;
}

interface UserAccount {
  id: number;
  name: string;
  role: number;
  staffRole: string;
  banned: number;
}

interface SystemRow {
  name: string;
  security: number;
  region: string;
  inSpace: number;
  docked: number;
}

interface AllianceRow {
  id: number;
  name: string;
  ticker: string;
  corporations: number;
  pilots: number;
  kind: string;
  region: string | null;
  systems: number;
}

interface SiegeRow {
  attacker: string;
  defender: string;
  system: string;
  region: string;
  security: number;
  starts: number;
  fighting: boolean;
  attackers: number | null;
  defenders: number | null;
}

interface NewsItem {
  time: number;
  text: string;
}

interface Article { id: number; title: string; body: string; html?: string; time: number }
interface DowntimeState {
  active: boolean;
  deadline: number | null;
  message: string;
  restart: boolean;
  restartAvailable: boolean;
  now: number;
}

interface CorporationRow {
  id: number;
  allianceID: number | null;
  name: string;
  ticker: string;
  members: number;
  alliance: string | null;
  bots: boolean;
}

interface Kill {
  id: number;
  time: number | null;
  victim: string | null;
  victimID: number | null;
  victimCorporation: string | null;
  victimCorpTicker: string | null;
  victimAlliance: string | null;
  victimAllianceTicker: string | null;
  ship: string | null;
  shipTypeID: number | null;
  shipClass: string | null;
  hullValue: number | null;
  totalValue: number | null;
  system: string | null;
  security: number | null;
  region: string | null;
  finalBlow: string | null;
  finalBlowID: number | null;
  finalBlowNpc: boolean | null;
  finalBlowCorporation: string | null;
  finalBlowCorpTicker: string | null;
  finalBlowAlliance: string | null;
  finalBlowAllianceTicker: string | null;
  involved: number | null;
  pod: boolean;
  victimCorpID: number | null;
  victimAllianceID: number | null;
  finalBlowCorpID: number | null;
  finalBlowAllianceID: number | null;
}

interface FeaturedKill {
  id: number;
  time: number;
  victimID: number | null;
  victim: string;
  victimCorporation: string;
  ship: string;
  shipTypeID: number;
  system: string;
  hullValue: number;
  totalValue: number;
}

interface TopEntry { id: number; name: string; ticker: string; kills: number }

interface Killboard {
  total: number;
  day: number;
  week: number;
  pods: number;
  ships: { ship: string; lost: number }[];
  featured: FeaturedKill[];
  structures: FeaturedKill[];
  activity: { characters: number; corporations: number; alliances: number; ships: number; systems: number; regions: number; kills: number };
  topCharacters: TopEntry[];
  topCorporations: TopEntry[];
  topAlliances: TopEntry[];
}

interface MarketLine {
  name: string;
  kind: string;
  stock: number;
  ask: number;
  bid: number;
  npcAsk: number;
}

interface Market {
  minedM3: number;
  refinedUnits: number;
  hauledM3: number;
  hullsBuilt: number;
  hullsBought: number;
  hullsFromNpcs: number;
  soldToPlayers: number;
  boughtFromPlayers: number;
  playerOrdersFilled: number;
  orders: number;
  corpTax: number;
  hubs: { station: string; worth: number; goods: MarketLine[] }[];
  shortages: { name: string; wanted: number }[];
}

interface Relations {
  standings: { side: string; player: string; heat: number; favor: number; flags: string[] }[];
  wingmen: { bot: string; player: string; rate: number }[];
  hunting: number;
  grudges: number;
}

interface Followed {
  id: number;
  name: string;
  role: string;
  corporation: string;
  alliance: string | null;
  ship: string;
  tier: number;
  isk: number;
  online: boolean;
  activity: string;
  system: string;
  security: number;
  station: string | null;
  history: NewsItem[];
}

interface Events {
  raids: { kind?: string; system: string; raiders: number; kills: number; lost: number }[];
  log: NewsItem[];
  said: NewsItem[];
  controls: boolean;
}

/// One column of a list: its heading, what it sorts by, and its cell.
interface Column<T> {
  title: string;
  num?: boolean;
  key: (item: T) => string | number;
  cell: (item: T) => (Node | string)[];
}

// A module, so nothing here is mistaken for the window's own globals.
export {};

const REFRESH_MS = 15_000;
const numbers = new Intl.NumberFormat("en-US");
let killPage = 0;
let killTotal = 0;
let killRows: Kill[] = [];

async function get<T>(path: string): Promise<T> {
  const response = await fetch(path, { cache: "no-store" });
  if (!response.ok) {
    throw new Error(`${path}: ${response.status}`);
  }
  return (await response.json()) as T;
}

async function postAdmin(path: string, data: object): Promise<{ ok?: string; error?: string }> {
  const response = await fetch(path, {
    method: "POST", headers: { "Content-Type": "application/json", "X-RMR-Admin": "1" },
    body: JSON.stringify(data), cache: "no-store",
  });
  const result = (await response.json()) as { ok?: string; error?: string };
  if (!response.ok) throw new Error(result.error ?? `HTTP ${response.status}`);
  return result;
}

const STAFF_ROLES = ["Player", "ISD", "ISD BH", "BH", "GM", "CCP"];

async function refreshUsers(): Promise<void> {
  const reply = document.getElementById("users-reply") as HTMLElement;
  try {
    const accounts = await get<UserAccount[]>("/api/users");
    list<UserAccount>("users", [
      { title: "Account", key: (a) => a.name.toLowerCase(), cell: (a) => [a.name, a.banned ? span(" (banned)", "hostile") : ""] },
      { title: "Chat colour", key: (a) => a.staffRole, cell: (a) => [span(a.staffRole, `staff-${a.staffRole.toLowerCase().replace(/\s/g, "-")}`)] },
      { title: "Role", key: (a) => a.staffRole, cell: (a) => {
        const select = document.createElement("select");
        select.setAttribute("aria-label", `Role for ${a.name}`);
        for (const name of STAFF_ROLES) {
          const option = document.createElement("option");
          option.value = name;
          option.textContent = name;
          select.append(option);
        }
        select.value = a.staffRole;
        select.addEventListener("change", () => void saveUserRole(a, select));
        return [select];
      } },
    ], accounts, "No player accounts found.");
    reply.textContent = `${accounts.length} accounts. Changes take effect after the player logs in again.`;
    reply.className = "note";
  } catch (e) {
    reply.textContent = e instanceof Error && e.message.includes("403")
      ? "Account management is available only on the server machine."
      : `Could not load accounts: ${String(e)}`;
    reply.className = "note hostile";
  }
}

async function saveUserRole(account: UserAccount, select: HTMLSelectElement): Promise<void> {
  const reply = document.getElementById("users-reply") as HTMLElement;
  select.disabled = true;
  try {
    const response = await fetch(`/api/users/role?id=${account.id}&role=${encodeURIComponent(select.value)}`, {
      method: "POST", headers: { "X-RMR-Admin": "1" }, cache: "no-store",
    });
    const answer = (await response.json()) as { ok?: string; error?: string };
    if (!response.ok) throw new Error(answer.error ?? `HTTP ${response.status}`);
    await refreshUsers();
    reply.textContent = answer.ok ?? "Role saved.";
    reply.className = "note";
  } catch (e) {
    select.value = account.staffRole;
    reply.textContent = `Could not change role: ${String(e)}`;
    reply.className = "note hostile";
  } finally {
    select.disabled = false;
  }
}

async function refreshArticles(): Promise<void> {
  const out = document.getElementById("articles") as HTMLElement;
  try {
    const articles = await get<Article[]>("/api/articles");
    out.replaceChildren(...articles.map((article) => {
      const item = document.createElement("article"); item.className = "news-article";
      const title = document.createElement("h3"); title.textContent = article.title;
      const time = document.createElement("time"); time.textContent = `${eveTime(article.time)} EVE`;
      // The server renders the body's formatting tags and escapes the rest.
      const body = document.createElement("div"); body.className = "article-body";
      if (article.html !== undefined) body.innerHTML = article.html; else body.textContent = article.body;
      item.append(time, title, body); return item;
    }));
    if (!articles.length) out.replaceChildren(span("No articles published yet.", "dim"));
  } catch (e) { out.replaceChildren(span(`Could not load news: ${String(e)}`, "hostile")); }
}

async function publishArticle(event: SubmitEvent): Promise<void> {
  event.preventDefault();
  const form = event.currentTarget as HTMLFormElement;
  const reply = document.getElementById("article-reply") as HTMLElement;
  const fields = new FormData(form);
  try {
    const result = await postAdmin("/api/articles", { title: fields.get("title"), body: fields.get("body") });
    reply.textContent = result.ok ?? "Published."; reply.className = "dropped";
    form.reset();
    void previewArticle();
    await refreshArticles();
  } catch (e) { reply.textContent = String(e); reply.className = "hostile"; }
}

let previewTimer: number | undefined;

async function previewArticle(): Promise<void> {
  const body = (document.querySelector("#article-form textarea[name=body]") as HTMLTextAreaElement).value;
  const out = document.getElementById("article-preview") as HTMLElement;
  if (!body.trim()) {
    out.replaceChildren(span("The formatted article appears here as you type.", "dim"));
    return;
  }
  try {
    const result = await postAdmin("/api/articles/preview", { body }) as { html?: string };
    const inner = document.createElement("div"); inner.className = "article-body";
    inner.innerHTML = result.html ?? "";
    out.replaceChildren(inner);
  } catch (e) { out.replaceChildren(span(`Preview unavailable: ${String(e)}`, "hostile")); }
}

function schedulePreview(): void {
  window.clearTimeout(previewTimer);
  previewTimer = window.setTimeout(() => void previewArticle(), 250);
}

/** Wraps the selection in a formatting tag, or inserts the tag pair at the cursor. */
function formatArticle(button: HTMLButtonElement): void {
  const area = document.querySelector("#article-form textarea[name=body]") as HTMLTextAreaElement;
  const { selectionStart: start, selectionEnd: end, value } = area;
  const selected = value.slice(start, end);
  const lineStart = start > 0 && value[start - 1] !== "\n" ? "\n" : "";
  const lineEnd = end < value.length && value[end] !== "\n" ? "\n" : "";
  let insert: string;
  let caret: number | null = null;
  const d = button.dataset;
  if (d.list) {
    const lines = (selected || "").split("\n").map((l) => l.trim()).filter((l) => l);
    const items = (lines.length ? lines : [""]).map((l) => `<li>${l}</li>`).join("\n");
    insert = `${lineStart}<${d.list}>\n${items}\n</${d.list}>${lineEnd}`;
    if (!lines.length) caret = start + lineStart.length + d.list.length + 3 + 4;
  } else if (d.link) {
    const url = window.prompt("Link address (https://…)", "https://");
    if (!url) return;
    insert = `<a href="${url.replace(/"/g, "")}">${selected || url}</a>`;
  } else if (d.insert) {
    insert = `${lineStart}${d.insert}${lineEnd}`;
  } else {
    const open = d.open ?? "", close = d.close ?? "";
    const before = d.line ? lineStart : "", after = d.line ? lineEnd : "";
    insert = `${before}${open}${selected}${close}${after}`;
    if (!selected) caret = start + before.length + open.length;
  }
  area.focus();
  area.setRangeText(insert, start, end, "end");
  if (caret !== null) area.setSelectionRange(caret, caret);
  schedulePreview();
}

interface Health {
  ready: boolean;
  cpu?: { machine: number; server: number; cores: number };
  memory?: { serverMb: number; usedMb: number; totalMb: number; usedPercent: number };
  tick?: { loadPercent: number; worstMs: number };
  database?: { pingMs: number; sizeMb: number; connections: number };
  bots?: { loadPercent: number; stepsPerSecond: number; worstTickMs: number };
  history?: [number, number, number, number, number, number][];
}

function healthTile(label: string, value: string, detail: string, percent: number | null): HTMLElement {
  const tile = document.createElement("div");
  tile.className = "health-tile" + (percent === null ? "" : percent >= 90 ? " bad" : percent >= 70 ? " warn" : "");
  const small = document.createElement("small"); small.textContent = label;
  const strong = document.createElement("strong"); strong.textContent = value;
  const span = document.createElement("span"); span.textContent = detail;
  tile.append(small, strong, span);
  if (percent !== null) {
    const meter = document.createElement("div"); meter.className = "meter";
    const fill = document.createElement("i"); fill.style.width = `${Math.max(0, Math.min(100, percent))}%`;
    meter.append(fill); tile.append(meter);
  }
  return tile;
}

/** A line chart of one or two series in an SVG of viewBox 300x80. */
function drawChart(id: string, series: number[][], colours: string[], max?: number): void {
  const svg = document.getElementById(id);
  if (!svg) return;
  const top = Math.max(max ?? 0, ...series.flat(), 1);
  const ns = "http://www.w3.org/2000/svg";
  const grid = document.createElementNS(ns, "path");
  grid.setAttribute("d", "M0 20H300M0 40H300M0 60H300");
  grid.setAttribute("stroke", "rgba(255,255,255,0.07)");
  const parts: Element[] = [grid];
  series.forEach((values, n) => {
    if (values.length < 2) return;
    const step = 300 / (values.length - 1);
    const d = values.map((v, i) => `${i ? "L" : "M"}${(i * step).toFixed(1)} ${(78 - (v / top) * 74).toFixed(1)}`).join("");
    const line = document.createElementNS(ns, "path");
    line.setAttribute("d", d);
    line.setAttribute("fill", "none");
    line.setAttribute("stroke", colours[n]);
    line.setAttribute("stroke-width", "1.6");
    line.setAttribute("vector-effect", "non-scaling-stroke");
    parts.push(line);
  });
  const label = document.createElementNS(ns, "text");
  label.setAttribute("x", "296"); label.setAttribute("y", "10"); label.setAttribute("text-anchor", "end");
  label.setAttribute("fill", "rgba(220,230,240,0.5)"); label.setAttribute("font-size", "9");
  label.textContent = `${Math.round(top)}`;
  parts.push(label);
  svg.replaceChildren(...parts);
}

async function refreshHealth(): Promise<void> {
  const out = document.getElementById("health-tiles");
  if (!out || (document.getElementById("w-health") as HTMLElement).classList.contains("page-hidden")) return;
  let h: Health;
  try { h = await get<Health>("/api/health"); } catch { out.replaceChildren(span("Health figures are unavailable.", "hostile")); return; }
  if (!h.ready || !h.cpu || !h.memory || !h.tick || !h.database || !h.bots) { out.replaceChildren(span("Measuring… figures appear within a few seconds.", "dim")); return; }
  const gb = (mb: number) => mb >= 1024 ? `${(mb / 1024).toFixed(1)} GB` : `${mb} MB`;
  out.replaceChildren(
    healthTile("Machine CPU", `${h.cpu.machine.toFixed(0)}%`, `${h.cpu.cores} logical cores`, h.cpu.machine),
    healthTile("Server CPU", `${h.cpu.server.toFixed(1)}%`, "eve-server's share of the machine", h.cpu.server),
    healthTile("Server memory", gb(h.memory.serverMb), "eve-server working set", null),
    healthTile("Machine memory", `${h.memory.usedPercent.toFixed(0)}%`, `${gb(h.memory.usedMb)} of ${gb(h.memory.totalMb)} used`, h.memory.usedPercent),
    healthTile("Game loop load", `${h.tick.loadPercent.toFixed(1)}%`, `of each 250 ms heartbeat; worst ${h.tick.worstMs.toFixed(0)} ms`, h.tick.loadPercent),
    healthTile("Bot simulation", `${h.bots.loadPercent.toFixed(1)}%`, `${h.bots.stepsPerSecond} steps/s; worst tick ${h.bots.worstTickMs.toFixed(0)} ms`, h.bots.loadPercent),
    healthTile("Database", `${h.database.pingMs.toFixed(1)} ms`, `${gb(h.database.sizeMb)}, ${h.database.connections} connections`, null),
  );
  const history = h.history ?? [];
  drawChart("chart-cpu", [history.map((r) => r[1]), history.map((r) => r[2])], ["#5ab8d6", "#d9a441"], 100);
  drawChart("chart-ram", [history.map((r) => r[3])], ["#5ab8d6"]);
  drawChart("chart-tick", [history.map((r) => r[4])], ["#d9a441"], 10);
}

let downtimeState: DowntimeState | null = null;
let downtimeOffset = 0;

function renderDowntime(): void {
  const clock = document.getElementById("downtime-clock") as HTMLElement;
  const label = clock.querySelector("small") as HTMLElement;
  const value = clock.querySelector("strong") as HTMLElement;
  const restart = !!downtimeState?.restart;
  clock.classList.toggle("restart", restart);
  if (!downtimeState?.active || !downtimeState.deadline) {
    label.textContent = "CLUSTER STATE";
    value.textContent = "No downtime scheduled";
    clock.classList.remove("running");
    return;
  }
  const seconds = Math.max(0, Math.ceil((downtimeState.deadline - Date.now() - downtimeOffset) / 1000));
  clock.classList.add("running");
  label.textContent = restart ? "SERVER RESTART IN" : "SERVER SHUTDOWN IN";
  value.textContent = seconds > 0 ? `${pad(Math.floor(seconds / 60))}:${pad(seconds % 60)}` : restart ? "Restarting…" : "Shutting down…";
}

async function refreshDowntime(): Promise<void> {
  try {
    const state = await get<DowntimeState>("/api/downtime");
    downtimeOffset = state.now - Date.now();
    downtimeState = state;
    for (const id of ["restart-countdown", "restart-now"]) {
      (document.getElementById(id) as HTMLButtonElement).disabled = !state.restartAvailable;
    }
    setText("restart-note", state.restartAvailable
      ? "Saves the universe, stops the server and starts it again. Pilots can log back in once it is up."
      : "Restart needs the server to be run by the RMR launcher (Start.bat), which starts it again.");
    renderDowntime();
  } catch {
    if (downtimeState?.active) {
      setText("downtime-reply", downtimeState.restart
        ? "The server is restarting. This page reconnects when it is back."
        : "Server connection closed after the countdown.");
    }
  }
}

interface BotSettings {
  running: number;
  configured: number | null;
  aggression: "calm" | "some" | "all";
  chat: string;
  trashTalk: boolean;
  joinPlayerCorps: string;
  joinPlayerAlliances: string;
  acceptApplications: string;
  onlineActivity: number;
  shownPerSystem: number;
  piratesAttackPlayers: boolean;
  roamingGangs?: string;
  huntPlayers?: boolean;
  miningFleets?: boolean;
  clearBelts?: boolean;
  wingmen?: string;
  botMarket?: string;
  countOverridden: boolean;
  aggressionOverridden: boolean;
  restartAvailable: boolean;
  max: number;
}

const AGGRESSION_NOTES: Record<BotSettings["aggression"], string> = {
  calm: "Now: all bots are calm. They attack no player unless shot first.",
  some: "Now: some bots are aggressive. Pirates outside high-sec, sides a player has angered and their hunters attack players.",
  all: "Now: all bots are aggressive. Every bot attacks any player it does not count as a friend.",
};

let botCountEdited = false;
let botActivityEdited = false;

function renderBots(settings: BotSettings): void {
  const input = document.getElementById("bots-count") as HTMLInputElement;
  input.max = String(settings.max);
  if (!botCountEdited) {
    input.value = String(settings.configured ?? settings.running);
  }
  const playing = settings.running.toLocaleString();
  const next = settings.configured === null ? "unknown" : settings.configured.toLocaleString();
  let note = `Playing now: ${playing}. At the next restart: ${next}.`;
  if (settings.countOverridden) note += " The RMR_BOTS environment variable overrides the saved number.";
  setText("bots-count-note", note);
  (document.getElementById("bots-count-restart") as HTMLButtonElement).disabled = !settings.restartAvailable;
  // Every choice group shows the setting it changes.
  for (const group of document.querySelectorAll<HTMLElement>(".bot-choice[data-setting]")) {
    const value = String((settings as unknown as Record<string, unknown>)[group.dataset.setting ?? ""]);
    for (const button of group.querySelectorAll<HTMLButtonElement>("button[data-value]")) {
      button.setAttribute("aria-pressed", String(button.dataset.value === value));
    }
  }
  const activity = document.getElementById("bots-activity") as HTMLInputElement;
  if (!botActivityEdited) {
    activity.value = String(settings.onlineActivity);
    setText("bots-activity-value", `${settings.onlineActivity.toFixed(1)}x`);
  }
  const shown = document.getElementById("bots-shown") as HTMLInputElement;
  if (document.activeElement !== shown && settings.shownPerSystem > 0) shown.value = String(settings.shownPerSystem);
  let aggression = AGGRESSION_NOTES[settings.aggression];
  if (settings.aggressionOverridden) aggression += " RMR_BOTS_AGGRESSION sets the mode again at the next start.";
  setText("bots-aggression-note", aggression);
}

async function refreshBotSettings(): Promise<void> {
  try {
    renderBots(await get<BotSettings>("/api/bots"));
  } catch {
    setText("bots-count-note", "Bot settings are unavailable.");
  }
}

async function changeBots(change: Record<string, string | number | boolean | undefined>): Promise<void> {
  const reply = document.getElementById("bots-reply") as HTMLElement;
  if (change.restart && !window.confirm("Save the bot count and restart the server now? Connected players will disconnect and can log back in once it is up.")) return;
  reply.classList.remove("error");
  try {
    const result = await postAdmin("/api/bots", change) as { ok?: string; settings?: BotSettings };
    reply.textContent = result.ok ?? "Saved.";
    botCountEdited = false;
    botActivityEdited = false;
    if (result.settings) renderBots(result.settings);
    if (change.restart) await refreshDowntime();
  } catch (e) {
    reply.textContent = e instanceof Error ? e.message : String(e);
    reply.classList.add("error");
  }
}

function botCountInput(): number | null {
  const input = document.getElementById("bots-count") as HTMLInputElement;
  const count = Number(input.value);
  if (!input.checkValidity() || !Number.isInteger(count)) {
    input.reportValidity();
    return null;
  }
  return count;
}

type DowntimeAction = "schedule" | "cancel" | "now" | "restart" | "restart-now";

async function changeDowntime(action: DowntimeAction): Promise<void> {
  const reply = document.getElementById("downtime-reply") as HTMLElement;
  if (action === "now" && !window.confirm("Shut down the server now? Connected players will disconnect.")) return;
  if (action === "restart-now" && !window.confirm("Restart the server now? Connected players will disconnect and can log back in once it is up.")) return;
  const message = (document.getElementById("downtime-message") as HTMLTextAreaElement).value;
  try {
    const result = await postAdmin("/api/downtime", { action, message });
    reply.textContent = result.ok ?? "Saved."; reply.className = "note dropped";
    await refreshDowntime();
  } catch (e) { reply.textContent = String(e); reply.className = "note hostile"; }
}

interface SeedStation {
  id: number;
  name: string;
  system: string;
  region: string;
  security: number;
  owner: string;
  playerOwned: boolean;
  seeded: boolean;
  sellOrders: number;
}

async function refreshSeeding(): Promise<void> {
  const reply = document.getElementById("seeding-reply") as HTMLElement;
  const q = (document.getElementById("seeding-query") as HTMLInputElement).value.trim();
  try {
    const stations = await get<SeedStation[]>(`/api/stations?q=${encodeURIComponent(q)}`);
    list<SeedStation>("seeding", [
      { title: "Station", key: (s) => s.name.toLowerCase(), cell: (s) => [s.name, s.playerOwned ? span(" (player-owned)", "dropped") : ""] },
      { title: "System", key: (s) => s.system.toLowerCase(), cell: (s) => [`${s.system} (${s.security.toFixed(1)})`] },
      { title: "Region", key: (s) => s.region.toLowerCase(), cell: (s) => [s.region] },
      { title: "Owner", key: (s) => s.owner.toLowerCase(), cell: (s) => [s.owner] },
      { title: "Sell orders", num: true, key: (s) => s.sellOrders, cell: (s) => [numbers.format(s.sellOrders)] },
      { title: "Seeding", key: (s) => (s.seeded ? 1 : 0), cell: (s) => {
        const seed = document.createElement("button");
        seed.type = "button";
        seed.textContent = s.seeded ? "Restock" : "Seed market";
        seed.addEventListener("click", () => void changeSeeding(s, "seed", seed));
        if (!s.seeded) return [seed];
        const stop = document.createElement("button");
        stop.type = "button";
        stop.className = "danger-button";
        stop.textContent = "Stop seeding";
        stop.addEventListener("click", () => void changeSeeding(s, "unseed", stop));
        return [span("Seeded ", "dropped"), seed, stop];
      } },
    ], stations, q ? "No stations match that search." : "No player-owned or seeded stations yet. Search for any station by name.");
    if (!reply.classList.contains("busy")) {
      reply.textContent = `${stations.length} station${stations.length === 1 ? "" : "s"} listed.`;
      reply.className = "note";
    }
  } catch (e) {
    reply.textContent = `Could not load stations: ${String(e)}`;
    reply.className = "note hostile";
  }
}

async function changeSeeding(station: SeedStation, action: "seed" | "unseed", button: HTMLButtonElement): Promise<void> {
  const reply = document.getElementById("seeding-reply") as HTMLElement;
  if (action === "unseed" && !window.confirm(`Stop seeding ${station.name}?` +
    (station.playerOwned ? " Its NPC sell orders will be removed." : " Existing orders stay until bought."))) return;
  button.disabled = true;
  reply.textContent = action === "seed" ? `Seeding ${station.name}…` : `Removing seeding from ${station.name}…`;
  reply.className = "note busy";
  try {
    const result = await postAdmin("/api/seed", { action, stationID: station.id });
    reply.className = "note";
    await refreshSeeding();
    reply.textContent = result.ok ?? "Done.";
    reply.className = "note dropped";
  } catch (e) {
    reply.textContent = e instanceof Error && e.message.includes("dashboard")
      ? "Market seeding is available only on the server machine."
      : `Could not change seeding: ${String(e)}`;
    reply.className = "note hostile";
  } finally {
    button.disabled = false;
  }
}

function span(text: string, className: string): HTMLElement {
  const e = document.createElement("span");
  e.textContent = text;
  e.className = className;
  return e;
}

function pilotLink(id: number | null | undefined, name: string | null | undefined): HTMLElement {
  if (!id || id <= 0) return span(name || "Unknown pilot", "pilot-name");
  const a = document.createElement("a");
  a.href = `#pilot-${id}`;
  a.className = "pilot-link";
  a.textContent = name || "Unknown pilot";
  a.addEventListener("click", (event) => event.stopPropagation());
  return a;
}

/// A link to a corporation's or alliance's killboard page.
function groupLink(kind: "corporation" | "alliance", id: number | null | undefined, name: string | null | undefined, fallback = ""): HTMLElement {
  if (!name) return span(fallback, "");
  if (!id || id <= 0) return span(name, "");
  const a = document.createElement("a");
  a.href = `#${kind}-${id}`;
  a.className = "pilot-link";
  a.textContent = name;
  a.addEventListener("click", (event) => event.stopPropagation());
  return a;
}
const corpLink = (id: number | null | undefined, name: string | null | undefined, fallback = "") => groupLink("corporation", id, name, fallback);
const allianceLink = (id: number | null | undefined, name: string | null | undefined, fallback = "") => groupLink("alliance", id, name, fallback);

/// A badge that opens the alliance's page, else the corporation's.
function groupBadge(allianceID: number | null | undefined, alliance: string | null | undefined,
  corpID: number | null | undefined, corp: string | null | undefined, ticker: string | null | undefined, small = false): HTMLElement {
  const [kind, id] = alliance && allianceID ? ["alliance", allianceID] as const : ["corporation", corpID] as const;
  const b = badge(alliance || corp, ticker, small, { kind, id });
  if (!id || id <= 0) return b;
  const a = document.createElement("a"); a.href = `#${kind}-${id}`; a.className = "badge-link";
  a.addEventListener("click", (event) => event.stopPropagation());
  a.append(b);
  return a;
}

const corpTicker = (t: string | null | undefined) => span(t ? ` [${t}]` : "", "tick");
const allianceTicker = (t: string | null | undefined) => span(t ? ` <${t}>` : "", "tick");

/// The security status colours of the client's map and overview, 1.0 to 0.0.
const SECURITY_COLOURS = ["#f00000", "#d73000", "#f04800", "#f06000", "#d77700", "#efef00", "#8fef2f", "#00f000", "#00ef47", "#48f0c0", "#2fefef"];

function security(s: number): HTMLElement {
  const rounded = Math.max(-1, Math.min(1, Math.round(s * 10) / 10));
  const e = span(rounded.toFixed(1), "sec");
  e.style.color = SECURITY_COLOURS[Math.max(0, Math.round(rounded * 10))];
  return e;
}

const pad = (n: number) => String(n).padStart(2, "0");

/// EVE time is UTC, written YYYY.MM.DD HH:MM.
function eveTime(ms: number | null): string {
  if (!ms) {
    return "";
  }
  const d = new Date(ms);
  return `${d.getUTCFullYear()}.${pad(d.getUTCMonth() + 1)}.${pad(d.getUTCDate())} ${pad(d.getUTCHours())}:${pad(d.getUTCMinutes())}`;
}

// ---- lists ------------------------------------------------------------------------

interface List {
  columns: Column<any>[];
  items: any[];
  sortBy: number;
  descending: boolean;
  empty: string;
}

const lists = new Map<string, List>();

/// Shows a list, sorted as its reader last chose (or by its first column).
function list<T>(id: string, columns: Column<T>[], items: T[], empty: string, sortBy = 0, descending = false): void {
  const known = lists.get(id);
  lists.set(id, {
    columns,
    items,
    empty,
    sortBy: known?.sortBy ?? sortBy,
    descending: known?.descending ?? descending,
  });
  draw(id);
}

function draw(id: string): void {
  const l = lists.get(id);
  const table = document.getElementById(id) as HTMLTableElement | null;
  if (!l || !table) {
    return;
  }
  const column = l.columns[l.sortBy];
  const items = [...l.items].sort((a, b) => {
    const x = column.key(a);
    const y = column.key(b);
    const order = typeof x === "number" && typeof y === "number" ? x - y : String(x).localeCompare(String(y));
    return l.descending ? -order : order;
  });
  table.replaceChildren();
  const head = table.createTHead().insertRow();
  l.columns.forEach((c, i) => {
    const th = document.createElement("th");
    th.textContent = c.title;
    th.className = [c.num ? "num" : "", i === l.sortBy ? "sorted" : "", i === l.sortBy && !l.descending ? "up" : ""].join(" ").trim();
    th.addEventListener("click", () => {
      l.descending = l.sortBy === i ? !l.descending : !!c.num;
      l.sortBy = i;
      draw(id);
    });
    head.append(th);
  });
  const body = table.createTBody();
  if (items.length === 0) {
    const td = body.insertRow().insertCell();
    td.colSpan = l.columns.length;
    td.className = "empty";
    td.textContent = l.empty;
    return;
  }
  for (const item of items) {
    const tr = body.insertRow();
    if (id === "kills" || id === "profile-kills") {
      tr.className = "selectable-kill";
      tr.tabIndex = 0;
      tr.setAttribute("role", "link");
      tr.setAttribute("aria-label", `Open kill report ${item.id}`);
      tr.addEventListener("click", () => openKill(item.id));
      tr.addEventListener("keydown", (event) => {
        if (event.key === "Enter" || event.key === " ") { event.preventDefault(); openKill(item.id); }
      });
    }
    for (const c of l.columns) {
      const td = tr.insertCell();
      if (c.num) {
        td.className = "num";
      }
      td.append(...c.cell(item));
    }
  }
}

interface KillDetail {
  error?: string;
  kill?: { id: number; time: number; victimID: number; victim: string; victimCorporation: string; victimAlliance: string; ship: string; shipTypeID: number; shipClass: string; hullValue: number; system: string; security: number; region: string; damage: number; pod: boolean; victimCorpID: number | null; victimAllianceID: number | null };
  participants?: { id: number; pilot: string; npc: boolean; corporation: string; alliance: string; corporationID: number | null; allianceID: number | null; ship: string; shipTypeID: number; weapon: string; damage: number; finalBlow: boolean }[];
  items?: { typeID: number; name: string; flag: number; quantity: number; dropped: boolean; unitValue: number }[];
}

function openKill(id: number): void {
  window.location.hash = `kill-${id}`;
}

function typeImage(typeID: number, variation: "render" | "icon", size: number): HTMLImageElement {
  const img = document.createElement("img");
  img.src = `https://images.evetech.net/types/${typeID}/${variation}?size=${size}`;
  img.alt = "";
  img.loading = "lazy";
  img.addEventListener("error", () => { img.style.display = "none"; });
  return img;
}

/// A stable colour for an alliance or corporation, used by badges and the map.
function colourFor(key: string | number): string {
  const text = String(key);
  let hash = 2166136261;
  for (let i = 0; i < text.length; i++) {
    hash ^= text.charCodeAt(i);
    hash = Math.imul(hash, 16777619);
  }
  const hue = (hash >>> 0) % 360;
  const sat = 55 + ((hash >>> 9) % 25);
  const light = 46 + ((hash >>> 17) % 14);
  return `hsl(${hue} ${sat}% ${light}%)`;
}

/// Images the server has none of, so each is asked for once per visit.
const missingPictures = new Set<string>();

/// Loads a server picture (/img/<kind>/<id>) and hands it over once it
/// arrives; nothing happens if there is none.
function loadPicture(kind: "portrait" | "corporation" | "alliance", id: number | null | undefined, size: number, done: (img: HTMLImageElement) => void): void {
  if (!id || id <= 0) return;
  const key = `${kind}/${id}`;
  if (missingPictures.has(key)) return;
  const img = document.createElement("img");
  img.alt = "";
  img.decoding = "async";
  img.addEventListener("load", () => done(img));
  img.addEventListener("error", () => missingPictures.add(key));
  img.src = `/img/${key}?size=${size}`;
}

/// A hexagonal emblem for a corporation or alliance: its real logo where the
/// server has one (alliance logos, NPC corporations), else its ticker.
function badge(name: string | null | undefined, ticker: string | null | undefined, small = false,
  logo?: { kind: "corporation" | "alliance"; id: number | null | undefined }): HTMLElement {
  const e = document.createElement("span");
  e.className = small ? "badge small-badge" : "badge";
  const label = (ticker || name || "?").replace(/[^A-Za-z0-9.\-]/g, "").slice(0, small ? 3 : 5) || "?";
  e.textContent = label.toUpperCase();
  e.style.setProperty("--badge", colourFor(name || ticker || "?"));
  e.title = name || ticker || "";
  if (logo) loadPicture(logo.kind, logo.id, small ? 64 : 128, (img) => { e.classList.add("has-logo"); e.replaceChildren(img); });
  return e;
}

/// A pilot's portrait from the game client's cache, else their initial.
function pilotPortrait(id: number | null | undefined, name: string | null | undefined, size: number, className = ""): HTMLElement {
  const e = span((name || "?").slice(0, 1).toUpperCase(), `pilot-avatar ${className}`.trim());
  e.title = name || "";
  loadPicture("portrait", id, size, (img) => { img.alt = name || ""; e.classList.add("has-portrait"); e.replaceChildren(img); });
  return e;
}

/// zKillboard's ISK figures: 64.72m, 1.77b, 10.00k.
function iskShort(n: number): string {
  const units: [number, string][] = [[1e12, "t"], [1e9, "b"], [1e6, "m"], [1e3, "k"]];
  for (const [size, unit] of units) if (n >= size) return `${(n / size).toFixed(2)}${unit}`;
  return n.toFixed(2);
}

const iskFull = (n: number) => `${new Intl.NumberFormat("en-US", { minimumFractionDigits: 2, maximumFractionDigits: 2 }).format(n)} ISK`;

function line(...content: (Node | string)[]): HTMLElement {
  const e = document.createElement("span");
  e.className = "line";
  e.append(...content);
  return e;
}

function securityLabel(s: number | null | undefined): HTMLElement {
  return security(s ?? 0);
}

/// The client's inventory flags, grouped into zKillboard's item sections.
const SLOT_SECTIONS: { title: string; test: (flag: number) => boolean }[] = [
  { title: "High Slots", test: (f) => f >= 27 && f <= 34 },
  { title: "Medium Slots", test: (f) => f >= 19 && f <= 26 },
  { title: "Low Slots", test: (f) => f >= 11 && f <= 18 },
  { title: "Rigs", test: (f) => f >= 92 && f <= 99 },
  { title: "Drone Bay", test: (f) => f === 87 },
  { title: "Cargo", test: (f) => f === 5 },
];

/// Where each slot sits on the fitting wheel, in degrees clockwise from the right.
function slotAngle(flag: number): number | null {
  if (flag >= 27 && flag <= 34) return -150 + (flag - 27) * 14;
  if (flag >= 19 && flag <= 26) return -35 + (flag - 19) * 14;
  if (flag >= 11 && flag <= 18) return 113 + (flag - 11) * 14;
  if (flag >= 92 && flag <= 99) return 72 + (flag - 92) * 13;
  return null;
}

async function showKill(id: number): Promise<void> {
  const section = document.getElementById("w-kill-detail") as HTMLElement;
  const output = document.getElementById("kill-detail") as HTMLElement;
  section.hidden = false;
  output.replaceChildren(span("Loading kill report…", "dim"));
  try {
    const detail = await get<KillDetail>(`/api/kill?id=${id}`);
    if (!detail.kill) { output.replaceChildren(span(detail.error ?? "Report unavailable", "hostile")); return; }
    const k = detail.kill;
    const items = detail.items ?? [];
    const parties = detail.participants ?? [];

    // The fitting wheel: modules on the rim by slot, their charges just inside.
    const stage = document.createElement("div"); stage.className = "fitting-stage";
    const wheel = document.createElement("div"); wheel.className = "fitting-wheel";
    const portrait = document.createElement("div"); portrait.className = "ship-portrait";
    portrait.append(typeImage(k.shipTypeID, "render", 512));
    const ring = document.createElement("div"); ring.className = "fitting-ring";
    const seen = new Set<number>();
    for (const item of items) {
      const angle = slotAngle(item.flag);
      if (angle === null) continue;
      const charge = seen.has(item.flag);
      seen.add(item.flag);
      const radius = charge ? 33 : 42.5;
      const icon = document.createElement("div");
      icon.className = charge ? "fitting-icon charge" : "fitting-icon";
      icon.style.left = `${50 + radius * Math.cos((angle * Math.PI) / 180)}%`;
      icon.style.top = `${50 + radius * Math.sin((angle * Math.PI) / 180)}%`;
      icon.title = item.name;
      icon.append(typeImage(item.typeID, "icon", 64));
      ring.append(icon);
    }
    stage.append(wheel, portrait, ring);
    const left = document.createElement("div");
    left.append(stage);
    if (!items.length) left.append(span("Fitting and cargo were not recorded for this killmail.", "kill-history-note"));

    // Values: the hull and everything destroyed with it, and what dropped.
    const dropped = items.filter((i) => i.dropped).reduce((sum, i) => sum + i.unitValue * i.quantity, 0);
    const destroyed = (k.hullValue || 0) + items.filter((i) => !i.dropped).reduce((sum, i) => sum + i.unitValue * i.quantity, 0);
    const total = destroyed + dropped;
    const pct = (n: number) => (total > 0 ? `${((n / total) * 100).toFixed(1)}%` : "");

    const info = document.createElement("table"); info.className = "km-info";
    const body = info.createTBody();
    const row = (label: string, ...content: (Node | string)[]) => {
      const tr = body.insertRow();
      const th = document.createElement("th"); th.textContent = label;
      const td = tr.insertCell(); td.append(...content);
      tr.prepend(th);
      return tr;
    };
    const victimBlock = document.createElement("div"); victimBlock.className = "km-victim";
    const victimNames = document.createElement("div");
    victimNames.append(pilotLink(k.victimID, k.victim), corpLink(k.victimCorpID, k.victimCorporation), allianceLink(k.victimAllianceID, k.victimAlliance));
    victimBlock.append(pilotPortrait(k.victimID, k.victim, 128, "km-portrait"), groupBadge(k.victimAllianceID, k.victimAlliance, k.victimCorpID, k.victimCorporation, null), victimNames);
    const victimRow = body.insertRow();
    const victimCell = victimRow.insertCell(); victimCell.colSpan = 2; victimCell.append(victimBlock);
    row("Ship:", span(k.ship, "ship-name"), span(k.shipClass ? ` ( ${k.shipClass} )` : "", "dim"));
    row("System:", span(k.system, ""), " (", securityLabel(k.security), ") / ", span(k.region, ""));
    row("Time:", `${eveTime(k.time)}`);
    row("Damage:", numbers.format(Math.round(k.damage)));
    row("Destroyed:", span(iskFull(destroyed), "val-destroyed"), span(pct(destroyed), "pct val-destroyed"));
    row("Dropped:", span(iskFull(dropped), "val-dropped"), span(pct(dropped), "pct val-dropped")).className = "row-dropped";
    row("Total:", span(iskFull(total), "val-total"));

    // The pilots involved, final blow and top damage first.
    const people = document.createElement("div"); people.className = "km-people";
    const finalBlow = parties.find((p) => p.finalBlow);
    const topDamage = [...parties].sort((a, b) => b.damage - a.damage)[0];
    const highlights = document.createElement("div"); highlights.className = "km-highlights";
    for (const [label, p] of [["Final Blow", finalBlow], ["Top Damage", topDamage]] as [string, typeof finalBlow][]) {
      const cell = document.createElement("div");
      cell.append(span(label, ""));
      if (p) cell.append(p.npc || p.id <= 0 ? badge(p.pilot, null) : pilotPortrait(p.id, p.pilot, 64, "km-mini-portrait"), partyName(p));
      else cell.append(span("—", "dim"));
      highlights.append(cell);
    }
    const involved = document.createElement("div"); involved.className = "km-involved";
    const heading = document.createElement("h4");
    heading.append(span(`${parties.length} Involved`, ""), span("Damage", ""));
    involved.append(heading);
    const totalDamage = parties.reduce((sum, p) => sum + p.damage, 0) || 1;
    for (const p of parties) {
      const party = document.createElement("div");
      party.className = p.finalBlow ? "party final-blow" : "party";
      if (!p.npc && p.id > 0) party.append(pilotPortrait(p.id, p.pilot, 64, "party-portrait"));
      if (p.shipTypeID) party.append(typeImage(p.shipTypeID, "icon", 64));
      const names = document.createElement("div");
      const corpName = corpLink(p.corporationID, p.corporation); corpName.classList.add("small");
      const allianceName = allianceLink(p.allianceID, p.alliance); allianceName.classList.add("small");
      names.append(partyName(p), corpName, allianceName);
      const dmg = document.createElement("div"); dmg.className = "dmg";
      dmg.append(numbers.format(Math.round(p.damage)));
      const share = document.createElement("small"); share.textContent = `${((p.damage / totalDamage) * 100).toFixed(1)}%`;
      dmg.append(share);
      party.append(names, dmg);
      involved.append(party);
    }
    people.append(highlights, involved);

    const top = document.createElement("div"); top.className = "km-top";
    top.append(left, info, people);

    // Items, grouped by where they were fitted or carried.
    const itemsHead = document.createElement("div"); itemsHead.className = "km-items-head";
    const itemsTitle = document.createElement("h2"); itemsTitle.textContent = "Item(s)";
    itemsHead.append(itemsTitle, span(`${items.length} recorded`, "dim"));
    const itemTable = document.createElement("table"); itemTable.className = "kill-items";
    const head = itemTable.createTHead().insertRow();
    for (const [h, cls] of [["Item Name", ""], ["Qty", "num"], ["Value", "num"]]) {
      const th = document.createElement("th"); th.textContent = h; th.className = cls; head.append(th);
    }
    const itemBody = itemTable.createTBody();
    const placed = new Set<number>();
    const sections = [...SLOT_SECTIONS, { title: "Other", test: (_flag: number) => true }];
    for (const { title, test } of sections) {
      const group = items.map((item, index) => ({ item, index })).filter(({ item, index }) => !placed.has(index) && test(item.flag));
      if (!group.length) continue;
      const slotRow = itemBody.insertRow(); slotRow.className = "slot-row";
      const slotCell = slotRow.insertCell(); slotCell.colSpan = 3; slotCell.textContent = title;
      for (const { item, index } of group) {
        placed.add(index);
        const tr = itemBody.insertRow();
        tr.className = item.dropped ? "item-dropped" : "item-destroyed";
        const name = document.createElement("div"); name.className = "item-name";
        name.append(typeImage(item.typeID, "icon", 64), span(item.name, ""));
        tr.insertCell().append(name);
        const qty = tr.insertCell(); qty.className = "num qty";
        qty.textContent = `${numbers.format(item.quantity)} ${item.dropped ? "✓" : "✗"}`;
        const value = tr.insertCell(); value.className = "val";
        value.textContent = item.unitValue > 0 ? numbers.format(Math.round(item.unitValue * item.quantity * 100) / 100) : "—";
      }
    }
    if (!items.length) {
      const empty = itemBody.insertRow().insertCell(); empty.colSpan = 3; empty.className = "empty";
      empty.textContent = "No items were recorded.";
    }
    output.replaceChildren(top, itemsHead, itemTable);
  } catch (e) { output.replaceChildren(span(`Could not load report: ${String(e)}`, "hostile")); }
}

function partyName(p: { id: number; pilot: string; npc?: boolean }): HTMLElement {
  if (p.npc || p.id <= 0) {
    const e = span(p.pilot || "NPC", "");
    e.append(span("NPC", "npc-tag"));
    return e;
  }
  return pilotLink(p.id, p.pilot);
}

interface PilotProfile {
  error?: string;
  pilot?: { id: number; name: string; corporation: string; ticker: string; alliance: string; allianceTicker: string;
    corporationID: number | null; allianceID: number | null;
    security: number; birthday: number | null; bot: boolean };
  destroyed: number;
  lost: number;
  iskDestroyed: number;
  iskLost: number;
  soloKills: number;
  avgInvolved: number;
  finalBlows: number;
  podKills: number;
  topKills: FeaturedKill[];
  /// [ISO weekday 1-7, UTC hour, kills and losses] over the last 90 days.
  activity: [number, number, number][];
  week: { ships: number; systems: number; regions: number; kills: number };
  ships: { ship: string; typeID: number; kills: number }[];
  systems: { name: string; security: number; kills: number }[];
  together: { id: number; name: string; kills: number }[];
}

let pilotView = { id: 0, mode: "all", page: 0 };

const pct1 = (a: number, b: number) => (a + b > 0 ? (a / (a + b)) * 100 : 0);

/// A two-coloured bar, as zKillboard's Dangerous / Snuggly and gang bars.
function ratioBar(left: number, leftLabel: string, rightLabel: string, title: string): HTMLElement {
  const wrap = document.createElement("div"); wrap.className = "zp-ratio"; wrap.title = title;
  const bar = document.createElement("div"); bar.className = "zp-bar";
  const a = document.createElement("span"); a.className = "zp-bar-left"; a.style.width = `${left}%`;
  a.textContent = left >= 12 ? `${Math.round(left)}%` : "";
  const b = document.createElement("span"); b.className = "zp-bar-right"; b.style.width = `${100 - left}%`;
  b.textContent = 100 - left >= 12 ? `${Math.round(100 - left)}%` : "";
  bar.append(a, b);
  const labels = document.createElement("div"); labels.className = "zp-bar-labels";
  labels.append(span(leftLabel, "hostile"), span(rightLabel, "dropped"));
  wrap.append(bar, labels);
  return wrap;
}

function sideBox(title: string, ...content: Node[]): HTMLElement {
  const box = document.createElement("section"); box.className = "zp-side-box";
  const h = document.createElement("h3"); h.textContent = title;
  box.append(h, ...content);
  return box;
}

async function showPilot(id: number): Promise<void> {
  const out = document.getElementById("pilot-profile") as HTMLElement;
  out.replaceChildren(span("Loading pilot record…", "dim"));
  try {
    const profile = await get<PilotProfile>(`/api/pilot?id=${id}`);
    if (!profile.pilot) { out.replaceChildren(span(profile.error || "Pilot unavailable", "hostile")); return; }
    const p = profile.pilot;
    pilotView = { id, mode: "all", page: 0 };

    // Who: portrait and identity, as zKillboard's character block.
    const head = document.createElement("div"); head.className = "zp-head";
    const who = document.createElement("div"); who.className = "zp-who";
    const avatar = pilotPortrait(p.id, p.name, 256, "zp-portrait");
    const facts = document.createElement("table"); facts.className = "zp-facts";
    const fact = (label: string, ...value: (Node | string)[]) => {
      const tr = facts.insertRow();
      const th = document.createElement("th"); th.textContent = label; tr.append(th);
      tr.insertCell().append(...value);
    };
    fact("Character:", span(p.name, "zp-name"));
    fact("Corporation:", corpLink(p.corporationID, p.corporation, "None"), p.ticker ? span(` [${p.ticker}]`, "dim") : "");
    fact("Alliance:", allianceLink(p.allianceID, p.alliance, "None"), p.allianceTicker ? span(` <${p.allianceTicker}>`, "dim") : "");
    fact("Sec. Status:", security(p.security));
    if (p.birthday) {
      const b = new Date(p.birthday);
      fact("Birthday:", `${b.getUTCFullYear()}-${pad(b.getUTCMonth() + 1)}-${pad(b.getUTCDate())}`);
    }
    fact("Pilot:", p.bot ? span("Simulated capsuleer", "dim") : span("Player", "dropped"));
    who.append(avatar, facts);

    // The numbers: ships and ISK destroyed against lost, with efficiency.
    const numbersTable = document.createElement("table"); numbersTable.className = "zp-numbers";
    const nh = numbersTable.createTHead().insertRow();
    for (const t of ["", "Destroyed", "Lost", "Eff. %"]) { const th = document.createElement("th"); th.textContent = t; nh.append(th); }
    const nb = numbersTable.createTBody();
    const numberRow = (label: string, destroyed: string, lost: string, eff: number) => {
      const tr = nb.insertRow();
      const th = document.createElement("th"); th.textContent = label; tr.append(th);
      tr.insertCell().append(span(destroyed, "dropped"));
      tr.insertCell().append(span(lost, "hostile"));
      tr.insertCell().append(span(eff.toFixed(1), eff >= 50 ? "dropped" : "hostile"));
    };
    numberRow("Ships", numbers.format(profile.destroyed), numbers.format(profile.lost), pct1(profile.destroyed, profile.lost));
    numberRow("ISK", iskShort(profile.iskDestroyed), iskShort(profile.iskLost), pct1(profile.iskDestroyed, profile.iskLost));
    numberRow("Final blows", numbers.format(profile.finalBlows), "", profile.destroyed ? (profile.finalBlows / profile.destroyed) * 100 : 0);
    const soloShare = profile.destroyed ? (profile.soloKills / profile.destroyed) * 100 : 0;
    const bars = document.createElement("div"); bars.className = "zp-bars";
    bars.append(
      ratioBar(pct1(profile.destroyed, profile.lost), "Dangerous", "Snuggly", "Ships destroyed against ships lost"),
      ratioBar(100 - soloShare, profile.avgInvolved ? `${profile.avgInvolved.toFixed(1)} avg gang` : "Gang", `${numbers.format(profile.soloKills)} solo kills`, "Kills with others against solo kills"),
    );
    const right = document.createElement("div"); right.className = "zp-stats";
    right.append(numbersTable, bars);
    head.append(who, right);

    // Overview / Kills / Losses tabs, then the most valuable kills.
    const tabs = document.createElement("nav"); tabs.className = "zp-tabs";
    for (const [mode, label] of [["all", "Overview"], ["kills", "Kills"], ["losses", "Losses"]] as [string, string][]) {
      const tab = document.createElement("button"); tab.type = "button"; tab.textContent = label;
      tab.dataset.mode = mode;
      tab.className = mode === "all" ? "active" : "";
      tab.addEventListener("click", () => {
        for (const t of tabs.querySelectorAll("button")) t.classList.toggle("active", t === tab);
        pilotView = { id, mode, page: 0 };
        void loadPilotKills();
      });
      tabs.append(tab);
    }
    const tiles = document.createElement("div"); tiles.id = "pilot-top"; tiles.className = "zkb-tiles";

    const main = document.createElement("div"); main.className = "zp-main";
    const listHead = document.createElement("div"); listHead.className = "zp-list-head";
    const listTitle = document.createElement("h3"); listTitle.id = "pilot-list-title"; listTitle.textContent = "Kills and losses";
    const pager = document.createElement("div"); pager.className = "zp-pager";
    const prev = document.createElement("button"); prev.type = "button"; prev.id = "pilot-prev"; prev.textContent = "‹";
    const page = span("", "dim"); page.id = "pilot-page";
    const next = document.createElement("button"); next.type = "button"; next.id = "pilot-next"; next.textContent = "›";
    prev.addEventListener("click", () => { if (pilotView.page > 0) { pilotView.page--; void loadPilotKills(); } });
    next.addEventListener("click", () => { pilotView.page++; void loadPilotKills(); });
    pager.append(prev, page, next);
    listHead.append(listTitle, pager);
    const table = document.createElement("table"); table.id = "pilot-kills"; table.className = "zkb-kills";
    main.append(listHead, table);

    // The sidebar: when they fight, the last week, ships, systems, wingmen.
    const side = document.createElement("aside"); side.className = "zp-side";
    const heat = document.createElement("div"); heat.className = "zp-heat";
    const max = Math.max(1, ...profile.activity.map((a) => a[2]));
    const cells = new Map(profile.activity.map((a) => [`${a[0]}-${a[1]}`, a[2]]));
    const days = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
    for (let d = 1; d <= 7; d++) {
      heat.append(span(days[d - 1], "zp-heat-day"));
      for (let h = 0; h < 24; h++) {
        const n = cells.get(`${d}-${h}`) ?? 0;
        const cell = document.createElement("span"); cell.className = "zp-heat-cell";
        cell.style.setProperty("--heat", String(n / max));
        cell.title = `${days[d - 1]} ${pad(h)}:00 EVE: ${n} kill${n === 1 ? "" : "s"} and losses`;
        heat.append(cell);
      }
    }
    const week = document.createElement("dl"); week.className = "zp-week";
    for (const [label, value] of [["Ships", profile.week.ships], ["Systems", profile.week.systems], ["Regions", profile.week.regions], ["Total kills", profile.week.kills]] as [string, number][]) {
      const dt = document.createElement("dt"); dt.textContent = label;
      const dd = document.createElement("dd"); dd.textContent = numbers.format(value);
      week.append(dt, dd);
    }
    const ranked = (entries: { label: Node; count: number }[], empty: string) => {
      const ol = document.createElement("ol"); ol.className = "zp-ranked";
      for (const e of entries) {
        const li = document.createElement("li");
        li.append(e.label, span(numbers.format(e.count), "count"));
        ol.append(li);
      }
      return entries.length ? ol : span(empty, "dim");
    };
    const shipLabel = (s: { ship: string; typeID: number }) => {
      const e = document.createElement("span"); e.className = "zp-ship"; e.append(typeImage(s.typeID, "icon", 32), s.ship); return e;
    };
    side.append(
      sideBox("Activity (last 90 days)", heat, span("EVE time, Monday to Sunday", "zp-note")),
      sideBox("Active PvP (last 7 days)", week),
      sideBox("Top ships", ranked(profile.ships.map((s) => ({ label: shipLabel(s), count: s.kills })), "No kills yet.")),
      sideBox("Top systems", ranked(profile.systems.map((s) => {
        const e = document.createElement("span"); e.append(securityLabel(s.security), ` ${s.name}`);
        return { label: e, count: s.kills };
      }), "No kills yet.")),
      sideBox("Flies with", ranked(profile.together.map((t) => ({ label: pilotLink(t.id, t.name), count: t.kills })), "Always solo so far.")),
    );

    const body = document.createElement("div"); body.className = "zp-body";
    const left = document.createElement("div"); left.className = "zp-left";
    left.append(tabs, tiles, main);
    body.append(left, side);
    out.replaceChildren(head, body);
    featuredTiles("pilot-top", profile.topKills, false, "No ships destroyed yet.");
    await loadPilotKills();
  } catch (e) { out.replaceChildren(span(`Could not load pilot: ${String(e)}`, "hostile")); }
}

interface GroupProfile {
  error?: string;
  kind: "corporation" | "alliance";
  info?: { id: number; name: string; ticker: string; description: string; members: number;
    ceoID?: number; ceo?: string; allianceID?: number | null; alliance?: string; allianceTicker?: string; npc?: boolean; outposts?: number;
    executorID?: number | null; executor?: string; corporations?: number; systems?: number; founded?: number | null };
  destroyed: number;
  lost: number;
  iskDestroyed: number;
  iskLost: number;
  soloKills: number;
  avgInvolved: number;
  topKills: FeaturedKill[];
  activity: [number, number, number][];
  week: { ships: number; systems: number; regions: number; kills: number; pilots: number };
  pilots: { id: number; name: string; kills: number }[];
  ships: { ship: string; typeID: number; kills: number }[];
  systems: { name: string; security: number; kills: number }[];
  corporations: { id: number; name: string; ticker: string; members: number }[];
}

let groupView = { kind: "corporation", id: 0, mode: "all", page: 0 };

/// The 7-by-24 grid of when someone fights, as zKillboard's activity chart.
function activityHeat(activity: [number, number, number][]): HTMLElement {
  const heat = document.createElement("div"); heat.className = "zp-heat";
  const max = Math.max(1, ...activity.map((a) => a[2]));
  const cells = new Map(activity.map((a) => [`${a[0]}-${a[1]}`, a[2]]));
  const days = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
  for (let d = 1; d <= 7; d++) {
    heat.append(span(days[d - 1], "zp-heat-day"));
    for (let h = 0; h < 24; h++) {
      const n = cells.get(`${d}-${h}`) ?? 0;
      const cell = document.createElement("span"); cell.className = "zp-heat-cell";
      cell.style.setProperty("--heat", String(n / max));
      cell.title = `${days[d - 1]} ${pad(h)}:00 EVE: ${n} kills and losses`;
      heat.append(cell);
    }
  }
  return heat;
}

function rankedList(entries: { label: Node; count: number }[], empty: string): HTMLElement {
  if (!entries.length) return span(empty, "dim");
  const ol = document.createElement("ol"); ol.className = "zp-ranked";
  for (const e of entries) {
    const li = document.createElement("li");
    li.append(e.label, span(numbers.format(e.count), "count"));
    ol.append(li);
  }
  return ol;
}

async function showGroup(kind: "corporation" | "alliance", id: number): Promise<void> {
  const out = document.getElementById("group-profile") as HTMLElement;
  setText("group-caption", kind === "alliance" ? "Alliance record" : "Corporation record");
  out.replaceChildren(span("Loading…", "dim"));
  try {
    const g = await get<GroupProfile>(`/api/${kind}?id=${id}`);
    if (!g.info) { out.replaceChildren(span(g.error || "Not found", "hostile")); return; }
    const info = g.info;
    groupView = { kind, id, mode: "all", page: 0 };
    setText("page-title", info.name);

    const head = document.createElement("div"); head.className = "zp-head";
    const who = document.createElement("div"); who.className = "zp-who";
    const logo = badge(info.name, info.ticker, false, { kind, id }); logo.classList.add("zp-logo");
    const facts = document.createElement("table"); facts.className = "zp-facts";
    const fact = (label: string, ...value: (Node | string)[]) => {
      const tr = facts.insertRow();
      const th = document.createElement("th"); th.textContent = label; tr.append(th);
      tr.insertCell().append(...value);
    };
    if (kind === "corporation") {
      fact("Corporation:", span(info.name, "zp-name"), span(` [${info.ticker}]`, "dim"));
      fact("Alliance:", allianceLink(info.allianceID, info.alliance, "None"), info.allianceTicker ? span(` <${info.allianceTicker}>`, "dim") : "");
      fact("CEO:", info.ceo ? pilotLink(info.ceoID, info.ceo) : span("None", "dim"));
      fact("Members:", numbers.format(info.members));
      if (info.outposts) fact("Outposts:", numbers.format(info.outposts));
      if (info.npc) fact("Type:", span("NPC corporation", "dim"));
    } else {
      fact("Alliance:", span(info.name, "zp-name"), span(` <${info.ticker}>`, "dim"));
      fact("Executor:", corpLink(info.executorID, info.executor, "None"));
      fact("Corporations:", numbers.format(info.corporations ?? 0));
      fact("Members:", numbers.format(info.members));
      fact("Systems held:", numbers.format(info.systems ?? 0));
      if (info.founded) {
        const f = new Date(info.founded);
        fact("Founded:", `${f.getUTCFullYear()}-${pad(f.getUTCMonth() + 1)}-${pad(f.getUTCDate())}`);
      }
    }
    who.append(logo, facts);

    const numbersTable = document.createElement("table"); numbersTable.className = "zp-numbers";
    const nh = numbersTable.createTHead().insertRow();
    for (const t of ["", "Destroyed", "Lost", "Eff. %"]) { const th = document.createElement("th"); th.textContent = t; nh.append(th); }
    const nb = numbersTable.createTBody();
    const numberRow = (label: string, destroyed: string, lost: string, eff: number) => {
      const tr = nb.insertRow();
      const th = document.createElement("th"); th.textContent = label; tr.append(th);
      tr.insertCell().append(span(destroyed, "dropped"));
      tr.insertCell().append(span(lost, "hostile"));
      tr.insertCell().append(span(eff.toFixed(1), eff >= 50 ? "dropped" : "hostile"));
    };
    numberRow("Ships", numbers.format(g.destroyed), numbers.format(g.lost), pct1(g.destroyed, g.lost));
    numberRow("ISK", iskShort(g.iskDestroyed), iskShort(g.iskLost), pct1(g.iskDestroyed, g.iskLost));
    const soloShare = g.destroyed ? (g.soloKills / g.destroyed) * 100 : 0;
    const bars = document.createElement("div"); bars.className = "zp-bars";
    bars.append(
      ratioBar(pct1(g.destroyed, g.lost), "Dangerous", "Snuggly", "Ships destroyed against ships lost"),
      ratioBar(100 - soloShare, g.avgInvolved ? `${g.avgInvolved.toFixed(1)} avg gang` : "Gang", `${numbers.format(g.soloKills)} solo kills`, "Kills with others against solo kills"),
    );
    const right = document.createElement("div"); right.className = "zp-stats";
    right.append(numbersTable, bars);
    if (info.description) {
      const d = document.createElement("p"); d.className = "zp-description"; d.textContent = info.description.replace(/<[^>]*>/g, " ").slice(0, 400);
      right.append(d);
    }
    head.append(who, right);

    const tabs = document.createElement("nav"); tabs.className = "zp-tabs";
    for (const [mode, label] of [["all", "Overview"], ["kills", "Kills"], ["losses", "Losses"]] as [string, string][]) {
      const tab = document.createElement("button"); tab.type = "button"; tab.textContent = label;
      tab.className = mode === "all" ? "active" : "";
      tab.addEventListener("click", () => {
        for (const t of tabs.querySelectorAll("button")) t.classList.toggle("active", t === tab);
        groupView = { kind, id, mode, page: 0 };
        void loadGroupKills();
      });
      tabs.append(tab);
    }
    const tiles = document.createElement("div"); tiles.id = "group-top"; tiles.className = "zkb-tiles";
    const main = document.createElement("div"); main.className = "zp-main";
    const listHead = document.createElement("div"); listHead.className = "zp-list-head";
    const listTitle = document.createElement("h3"); listTitle.id = "group-list-title"; listTitle.textContent = "Kills and losses";
    const pager = document.createElement("div"); pager.className = "zp-pager";
    const prev = document.createElement("button"); prev.type = "button"; prev.id = "group-prev"; prev.textContent = "‹";
    const page = span("", "dim"); page.id = "group-page";
    const next = document.createElement("button"); next.type = "button"; next.id = "group-next"; next.textContent = "›";
    prev.addEventListener("click", () => { if (groupView.page > 0) { groupView.page--; void loadGroupKills(); } });
    next.addEventListener("click", () => { groupView.page++; void loadGroupKills(); });
    pager.append(prev, page, next);
    listHead.append(listTitle, pager);
    const table = document.createElement("table"); table.id = "group-kills"; table.className = "zkb-kills";
    main.append(listHead, table);

    const side = document.createElement("aside"); side.className = "zp-side";
    const week = document.createElement("dl"); week.className = "zp-week";
    for (const [label, value] of [["Pilots", g.week.pilots], ["Ships", g.week.ships], ["Systems", g.week.systems], ["Regions", g.week.regions], ["Total kills", g.week.kills]] as [string, number][]) {
      const dt = document.createElement("dt"); dt.textContent = label;
      const dd = document.createElement("dd"); dd.textContent = numbers.format(value);
      week.append(dt, dd);
    }
    const shipLabel = (s: { ship: string; typeID: number }) => {
      const e = document.createElement("span"); e.className = "zp-ship"; e.append(typeImage(s.typeID, "icon", 32), s.ship); return e;
    };
    side.append(
      sideBox("Activity (last 90 days)", activityHeat(g.activity), span("EVE time, Monday to Sunday", "zp-note")),
      sideBox("Active PvP (last 7 days)", week),
      sideBox("Top pilots", rankedList(g.pilots.map((p) => ({ label: pilotLink(p.id, p.name), count: p.kills })), "No kills yet.")),
    );
    if (kind === "alliance") {
      side.append(sideBox("Member corporations", rankedList(g.corporations.map((c) => {
        const e = document.createElement("span"); e.append(corpLink(c.id, c.name), corpTicker(c.ticker));
        return { label: e, count: c.members };
      }), "No member corporations.")));
    }
    side.append(
      sideBox("Top ships", rankedList(g.ships.map((s) => ({ label: shipLabel(s), count: s.kills })), "No kills yet.")),
      sideBox("Top systems", rankedList(g.systems.map((s) => {
        const e = document.createElement("span"); e.append(securityLabel(s.security), ` ${s.name}`);
        return { label: e, count: s.kills };
      }), "No kills yet.")),
    );

    const body = document.createElement("div"); body.className = "zp-body";
    const left = document.createElement("div"); left.className = "zp-left";
    left.append(tabs, tiles, main);
    body.append(left, side);
    out.replaceChildren(head, body);
    featuredTiles("group-top", g.topKills, false, "No ships destroyed yet.");
    await loadGroupKills();
  } catch (e) { out.replaceChildren(span(`Could not load: ${String(e)}`, "hostile")); }
}

async function loadGroupKills(): Promise<void> {
  const { kind, id, mode, page } = groupView;
  const table = document.getElementById("group-kills") as HTMLTableElement | null;
  if (!table) return;
  const titles: Record<string, string> = { all: "Kills and losses", kills: "Kills", losses: "Losses" };
  setText("group-list-title", titles[mode] ?? "Kills and losses");
  try {
    const rows = await get<Kill[]>(`/api/group-kills?kind=${kind}&id=${id}&mode=${mode}&page=${page}`);
    if (groupView.id !== id || groupView.mode !== mode || groupView.page !== page) return;
    const empty = mode === "losses" ? "No losses recorded." : mode === "kills" ? "No kills recorded." : "No kills or losses recorded.";
    renderKillTable(table, rows, page > 0 && !rows.length ? "No more on this page." : empty, 0, { kind: kind as "corporation" | "alliance", id });
    setText("group-page", `Page ${page + 1}`);
    (document.getElementById("group-prev") as HTMLButtonElement).disabled = page === 0;
    (document.getElementById("group-next") as HTMLButtonElement).disabled = rows.length < 30;
  } catch (e) {
    table.replaceChildren();
    const td = table.insertRow().insertCell(); td.className = "empty hostile"; td.textContent = `Could not load: ${String(e)}`;
  }
}

async function loadPilotKills(): Promise<void> {
  const { id, mode, page } = pilotView;
  const table = document.getElementById("pilot-kills") as HTMLTableElement | null;
  if (!table) return;
  const titles: Record<string, string> = { all: "Kills and losses", kills: "Kills", losses: "Losses" };
  setText("pilot-list-title", titles[mode] ?? "Kills and losses");
  try {
    const rows = await get<Kill[]>(`/api/pilot-kills?id=${id}&mode=${mode}&page=${page}`);
    if (pilotView.id !== id || pilotView.mode !== mode || pilotView.page !== page) return;
    const empty = mode === "losses" ? "No losses recorded." : mode === "kills" ? "No kills recorded." : "No kills or losses recorded.";
    renderKillTable(table, rows, page > 0 && !rows.length ? "No more on this page." : empty, id);
    setText("pilot-page", `Page ${page + 1}`);
    (document.getElementById("pilot-prev") as HTMLButtonElement).disabled = page === 0;
    (document.getElementById("pilot-next") as HTMLButtonElement).disabled = rows.length < 30;
  } catch (e) {
    table.replaceChildren();
    const td = table.insertRow().insertCell(); td.className = "empty hostile"; td.textContent = `Could not load: ${String(e)}`;
  }
}

async function searchKillboardPilots(): Promise<void> {
  const query = (document.getElementById("killboard-search") as HTMLInputElement).value.trim();
  const out = document.getElementById("killboard-search-results") as HTMLElement;
  if (query.length < 2) return;
  out.replaceChildren(span("Searching pilots…", "dim"));
  try {
    const found = await get<{ id: number; name: string; corporation: string; alliance: string }[]>(`/api/pilot-search?q=${encodeURIComponent(query)}`);
    out.replaceChildren(...found.map((p) => {
      const row = document.createElement("div"); row.className = "search-result";
      row.append(pilotLink(p.id, p.name), span([p.corporation, p.alliance].filter(Boolean).join(" · "), "dim"));
      return row;
    }));
    if (!found.length) out.replaceChildren(span("No pilot found. Try a shorter name.", "dim"));
  } catch (e) { out.replaceChildren(span(`Search failed: ${String(e)}`, "hostile")); }
}

const count = (n: number) => [numbers.format(n)];

const compact = new Intl.NumberFormat("en-US", { notation: "compact", maximumFractionDigits: 1 });
const isk = (n: number) => `${compact.format(n)} ISK`;
const price = (n: number) => (n >= 1000 ? compact.format(n) : n.toFixed(2));

// ---- windows ----------------------------------------------------------------------

function showStatus(s: Summary): void {
  setText("metric-players", numbers.format(s.players));
  setText("metric-bots", numbers.format(s.bots.online));
  setText("metric-wars", numbers.format(s.wars));
  setText("metric-uptime", uptime(s.uptimeSeconds));
  setText("mast-players", numbers.format(s.players));
  setText("mast-bots", numbers.format(s.bots.online));
  const rows: [string, string][] = [
    ["Pilots online", numbers.format(s.players)],
    ["Capsuleers simulated", numbers.format(s.bots.total)],
    ["Simulated online", numbers.format(s.bots.online)],
    ["Simulated in space", numbers.format(s.bots.inSpace)],
    ["Corporations", numbers.format(s.corporations)],
    ["Alliances", numbers.format(s.alliances)],
    ["Wars under way", numbers.format(s.wars)],
    ["Ships lost in war", numbers.format(s.lostInFights)],
    ["Sovereign systems", numbers.format(s.systemsHeld)],
    ["Server up", uptime(s.uptimeSeconds)],
  ];
  const dl = document.getElementById("status") as HTMLElement;
  dl.replaceChildren(
    ...rows.flatMap(([label, value]) => {
      const dt = document.createElement("dt");
      dt.textContent = label;
      const dd = document.createElement("dd");
      dd.textContent = value;
      return [dt, dd];
    }),
  );
  const online = s.players + s.bots.online;
  const line = document.getElementById("statusline") as HTMLElement;
  line.textContent = `Status: OK, ${numbers.format(online)} players${s.bots.running ? "" : " (simulation paused)"}`;
}

function showConnectivity(online: boolean): void {
  const indicator = document.getElementById("server-indicator") as HTMLElement;
  indicator.dataset.state = online ? "online" : "pending";
  indicator.textContent = online ? "Server online" : "Reconnecting";
}

function uptime(seconds: number): string {
  const d = Math.floor(seconds / 86400);
  const h = Math.floor((seconds % 86400) / 3600);
  const m = Math.floor((seconds % 3600) / 60);
  return d > 0 ? `${d}d ${h}h ${m}m` : h > 0 ? `${h}h ${m}m` : `${m}m`;
}

async function refresh(): Promise<void> {
  try {
    const [summary, wars, players, systems, alliances, corporations, kills, news, sieges] = await Promise.all([
      get<Summary>("/api/summary").then((summary) => {
        showConnectivity(true);
        return summary;
      }, (error) => {
        showConnectivity(false);
        throw error;
      }),
      get<War[]>("/api/wars"),
      get<Player[]>("/api/players"),
      get<SystemRow[]>("/api/systems"),
      get<AllianceRow[]>("/api/alliances"),
      get<CorporationRow[]>("/api/corporations"),
      get<Kill[]>(`/api/kills?page=${killPage}`),
      get<NewsItem[]>("/api/news"),
      get<SiegeRow[]>("/api/sieges"),
    ]);
    showStatus(summary);

    list<War>("wars", [
      { title: "Aggressor", key: (w) => w.declaredBy, cell: (w) => [w.declaredBy, allianceTicker(w.declaredByTicker)] },
      { title: "", key: () => "", cell: () => [span("vs", "hostile")] },
      { title: "Defender", key: (w) => w.against, cell: (w) => [w.against, allianceTicker(w.againstTicker)] },
      { title: "Declared", key: (w) => w.declared ?? 0, cell: (w) => [eveTime(w.declared)] },
      { title: "Ends", key: (w) => w.ends ?? 0, cell: (w) => [eveTime(w.ends)] },
      { title: "Losses", num: true, key: (w) => w.losses, cell: (w) => count(w.losses) },
      { title: "Type", key: (w) => (w.mutual ? 1 : 0), cell: (w) => [span(w.bots ? "mutual (simulated)" : w.mutual ? "mutual" : "declared", "dim")] },
    ], wars, "There are no wars.", 3, true);

    list<Player>("players", [
      { title: "Name", key: (p) => p.name, cell: (p) => [pilotLink(p.id, p.name)] },
      { title: "Corporation", key: (p) => p.ticker ?? "", cell: (p) => [corpTicker(p.ticker), p.alliance ? span(` ${p.alliance}`, "dim") : ""] },
      { title: "Location", key: (p) => p.system ?? "", cell: (p) => [security(p.systemSecurity ?? 0), ` ${p.system ?? ""}`, p.docked ? span(" (docked)", "dim") : ""] },
      { title: "Ship", key: (p) => p.ship ?? "", cell: (p) => [p.ship ?? ""] },
    ], players, "No pilots are logged in.");

    list<SystemRow>("systems", [
      { title: "System", key: (s) => s.name, cell: (s) => [security(s.security), ` ${s.name}`] },
      { title: "Region", key: (s) => s.region, cell: (s) => [span(s.region, "dim")] },
      { title: "In space", num: true, key: (s) => s.inSpace, cell: (s) => count(s.inSpace) },
      { title: "Docked", num: true, key: (s) => s.docked, cell: (s) => count(s.docked) },
      { title: "Total", num: true, key: (s) => s.inSpace + s.docked, cell: (s) => count(s.inSpace + s.docked) },
    ], systems, "No one is about.", 4, true);

    list<AllianceRow>("alliances", [
      { title: "Alliance", key: (a) => a.name, cell: (a) => [allianceLink(a.id, a.name), allianceTicker(a.ticker)] },
      { title: "Holds", key: (a) => a.region ?? "", cell: (a) => [a.region ?? span(a.kind === "players" ? "" : a.kind, "dim")] },
      { title: "Corps", num: true, key: (a) => a.corporations, cell: (a) => count(a.corporations) },
      { title: "Pilots", num: true, key: (a) => a.pilots, cell: (a) => count(a.pilots) },
      { title: "Systems", num: true, key: (a) => a.systems, cell: (a) => (a.systems > 0 ? count(a.systems) : [span("-", "dim")]) },
    ], alliances, "No alliances have been founded.", 3, true);

    list<CorporationRow>("corporations", [
      { title: "Corporation", key: (c) => c.name, cell: (c) => [corpLink(c.id, c.name), corpTicker(c.ticker), c.bots ? "" : span(" (players)", "dim")] },
      { title: "Alliance", key: (c) => c.alliance ?? "", cell: (c) => [c.alliance ? allianceLink(c.allianceID, c.alliance) : ""] },
      { title: "Members", num: true, key: (c) => c.members, cell: (c) => count(c.members) },
    ], corporations, "No corporations have been founded.", 2, true);

    list<SiegeRow>("sieges", [
      { title: "System", key: (s) => s.system, cell: (s) => [security(s.security), ` ${s.system}`, span(` ${s.region}`, "dim")] },
      { title: "Attacker", key: (s) => s.attacker, cell: (s) => [s.attacker] },
      { title: "Defender", key: (s) => s.defender, cell: (s) => [s.defender] },
      {
        title: "When",
        key: (s) => s.starts,
        cell: (s) =>
          s.fighting
            ? [span(`BATTLE ${s.attackers ?? 0} vs ${s.defenders ?? 0}`, "hostile")]
            : [eveTime(s.starts)],
      },
    ], sieges, "No sieges are planned.", 3, false);

    list<NewsItem>("news", [
      { title: "Time", key: (n) => n.time, cell: (n) => [eveTime(n.time)] },
      { title: "Report", key: (n) => n.text, cell: (n) => [n.text] },
    ], news, "Nothing has happened in null-sec yet.", 0, true);

    killRows = kills;
    showKillList();
    await refreshBots();
  } catch (e) {
    const line = document.getElementById("statusline") as HTMLElement;
    line.textContent = `Status: unreachable, retrying (${e instanceof Error ? e.message : String(e)})`;
  }
}

function showKillList(): void {
  const query = (document.getElementById("kill-search") as HTMLInputElement).value.trim().toLowerCase();
  const kills = query
    ? killRows.filter((k) => [k.victim, k.ship, k.system, k.region, k.finalBlow, k.victimCorporation, k.victimAlliance, k.finalBlowCorporation, k.finalBlowAlliance]
      .some((s) => s?.toLowerCase().includes(query)))
    : killRows;
  renderKillTable(document.getElementById("kills") as HTMLTableElement, kills,
    query ? "No kills on this page match." : "No kills recorded yet.");
  setText("kills-page", `Page ${killPage + 1} of ${Math.max(1, Math.ceil(killTotal / 30))}`);
  (document.getElementById("kills-prev") as HTMLButtonElement).disabled = killPage === 0;
  (document.getElementById("kills-next") as HTMLButtonElement).disabled = (killPage + 1) * 30 >= killTotal;
}

/// zKillboard's kill list, grouped by day. On a pilot's page (`pilotID`) the
/// pilot's kills are green and losses red.
function renderKillTable(table: HTMLTableElement, kills: Kill[], empty: string, pilotID = 0,
  group?: { kind: "corporation" | "alliance"; id: number }): void {
  table.replaceChildren();
  const head = table.createTHead().insertRow();
  for (const [title, span2] of [["Time", 1], ["Ship", 1], ["Location", 1], ["Victim", 2], ["Final Blow", 2]] as [string, number][]) {
    const th = document.createElement("th"); th.textContent = title; th.colSpan = span2; head.append(th);
  }
  const body = table.createTBody();
  if (!kills.length) {
    const td = body.insertRow().insertCell(); td.colSpan = 7; td.className = "empty";
    td.textContent = empty;
  }
  let day = "";
  for (const k of kills) {
    const date = k.time ? new Date(k.time) : null;
    const dayLabel = date ? `${date.getUTCFullYear()}-${pad(date.getUTCMonth() + 1)}-${pad(date.getUTCDate())}` : "Unknown date";
    if (dayLabel !== day) {
      day = dayLabel;
      const dateRow = body.insertRow(); dateRow.className = "date-row";
      const cell = dateRow.insertCell(); cell.colSpan = 7; cell.textContent = dayLabel;
    }
    const tr = body.insertRow();
    const mine = pilotID > 0 || !!group;
    const loss = (pilotID > 0 && k.victimID === pilotID)
      || (!!group && (group.kind === "alliance" ? k.victimAllianceID : k.victimCorpID) === group.id);
    tr.className = ["kill-row", k.pod ? "is-pod" : "", mine ? (loss ? "is-loss" : "is-kill") : ""].filter(Boolean).join(" ");
    tr.tabIndex = 0;
    tr.setAttribute("role", "link");
    tr.setAttribute("aria-label", `Open kill report ${k.id}`);
    tr.addEventListener("click", () => openKill(k.id));
    tr.addEventListener("keydown", (event) => {
      if (event.key === "Enter" || event.key === " ") { event.preventDefault(); openKill(k.id); }
    });
    const time = tr.insertCell(); time.className = "k-time";
    if (mine) time.append(span(loss ? "✕ " : "✓ ", loss ? "hostile" : "dropped"));
    time.append(date ? `${pad(date.getUTCHours())}:${pad(date.getUTCMinutes())}` : "");
    const value = document.createElement("small"); value.textContent = iskShort(k.totalValue ?? k.hullValue ?? 0); time.append(value);
    const ship = tr.insertCell(); ship.className = "k-ship";
    if (k.shipTypeID) { const img = typeImage(k.shipTypeID, "render", 64); img.title = k.ship ?? ""; ship.append(img); }
    const loc = tr.insertCell(); loc.className = "k-loc";
    loc.append(line(securityLabel(k.security), " ", k.system ?? "Unknown"), line(span(k.region ?? "", "")));
    const victimBadge = tr.insertCell(); victimBadge.className = "k-badge-cell";
    victimBadge.append(groupBadge(k.victimAllianceID, k.victimAlliance, k.victimCorpID, k.victimCorporation, k.victimAllianceTicker || k.victimCorpTicker));
    const victim = tr.insertCell();
    victim.append(
      line(pilotLink(k.victimID, k.victim), ` (${k.ship ?? "Unknown ship"})`),
      line(corpLink(k.victimCorpID, k.victimCorporation)),
      line(allianceLink(k.victimAllianceID, k.victimAlliance)),
    );
    const finalBadge = tr.insertCell(); finalBadge.className = "k-badge-cell";
    const finalCell = tr.insertCell();
    if (k.finalBlowNpc || (k.finalBlowID ?? 0) <= 0) {
      finalBadge.append(badge(k.finalBlow || "NPC", "NPC"));
      const npc = span(k.finalBlow ?? "NPC", ""); npc.append(span("NPC", "npc-tag"));
      finalCell.append(line(npc));
    } else {
      finalBadge.append(groupBadge(k.finalBlowAllianceID, k.finalBlowAlliance, k.finalBlowCorpID, k.finalBlowCorporation, k.finalBlowAllianceTicker || k.finalBlowCorpTicker));
      const who = line(pilotLink(k.finalBlowID, k.finalBlow));
      if ((k.involved ?? 0) === 1) who.append(span("SOLO", "solo-tag"));
      else if (k.involved) who.append(` (${k.involved})`);
      finalCell.append(who, line(corpLink(k.finalBlowCorpID, k.finalBlowCorporation)), line(allianceLink(k.finalBlowAllianceID, k.finalBlowAlliance)));
    }
    for (const a of tr.querySelectorAll("a")) a.addEventListener("click", (event) => event.stopPropagation());
  }
}

function featuredTiles(id: string, kills: FeaturedKill[], structures: boolean, empty?: string): void {
  const out = document.getElementById(id) as HTMLElement;
  out.replaceChildren(...kills.map((kill) => {
    const tile = document.createElement("article"); tile.className = "zkb-tile";
    tile.tabIndex = 0;
    tile.addEventListener("click", () => openKill(kill.id));
    tile.addEventListener("keydown", (e) => { if (e.key === "Enter") openKill(kill.id); });
    const image = document.createElement("div"); image.className = "tile-image";
    image.append(typeImage(kill.shipTypeID, "render", 128));
    const owner = structures ? span(kill.victimCorporation || kill.victim, "tile-victim") : pilotLink(kill.victimID, kill.victim);
    owner.classList.add("tile-victim");
    owner.addEventListener("click", (e) => e.stopPropagation());
    tile.title = `${kill.ship} in ${kill.system}, ${eveTime(kill.time)}`;
    tile.append(span(kill.ship, "tile-ship"), image, span(`${iskShort(kill.totalValue || kill.hullValue)} ISK`, "tile-value"), owner);
    return tile;
  }));
  if (!kills.length) out.replaceChildren(span(empty ?? (structures ? "No structures destroyed this week." : "No ships destroyed this week."), "zkb-empty"));
}

function definitions(id: string, rows: [string, number][]): void {
  const dl = document.getElementById(id) as HTMLElement;
  dl.replaceChildren(...rows.flatMap(([label, value]) => {
    const dt = document.createElement("dt"); dt.textContent = label;
    const dd = document.createElement("dd"); dd.textContent = numbers.format(value);
    return [dt, dd];
  }));
}

function topList(id: string, entries: TopEntry[], kind: "character" | "corporation" | "alliance"): void {
  const ol = document.getElementById(id) as HTMLElement;
  ol.replaceChildren(...entries.map((entry) => {
    const li = document.createElement("li");
    const who = kind === "character" ? pilotLink(entry.id, entry.name) : groupLink(kind, entry.id, entry.name);
    if (entry.ticker) who.title = `${entry.name} [${entry.ticker}]`;
    if (kind !== "character") li.append(badge(entry.name, entry.ticker, true));
    li.append(who, span(numbers.format(entry.kills), "count"));
    return li;
  }));
  if (!entries.length) ol.replaceChildren(span("No kills in the last 7 days.", "empty"));
}

// ---- the bots' windows -------------------------------------------------------------

let market: Market | null = null;
let chosenPilot: number | null = null;
let followedNow: Followed[] = [];

async function refreshBots(): Promise<void> {
  const [board, mkt, rel, followed, events] = await Promise.all([
    get<Killboard>("/api/killboard"),
    get<Market | null>("/api/market"),
    get<Relations | null>("/api/relations"),
    get<Followed[] | null>("/api/followed"),
    get<Events | null>("/api/events"),
  ]);

  killTotal = board.total;
  showKillList();
  featuredTiles("killboard-featured", board.featured, false);
  featuredTiles("killboard-structures", board.structures, true);
  (document.getElementById("zkb-structures-block") as HTMLElement).hidden = !board.structures.length;
  definitions("killboard-activity", [
    ["Characters", board.activity.characters], ["Corporations", board.activity.corporations],
    ["Alliances", board.activity.alliances], ["Ships", board.activity.ships],
    ["Systems", board.activity.systems], ["Regions", board.activity.regions], ["Total Kills", board.activity.kills],
  ]);
  definitions("killboard-stats", [
    ["All-time killmails", board.total], ["Kills (24h)", board.day], ["Kills (7 days)", board.week], ["Pod kills", board.pods],
  ]);
  topList("killboard-top-characters", board.topCharacters, "character");
  topList("killboard-top-corporations", board.topCorporations, "corporation");
  topList("killboard-top-alliances", board.topAlliances, "alliance");
  const ships = document.getElementById("killboard-ships") as HTMLElement;
  ships.replaceChildren(...board.ships.map((s) => {
    const li = document.createElement("li");
    li.append(span(s.ship, ""), span(numbers.format(s.lost), "count"));
    return li;
  }));
  if (!board.ships.length) ships.replaceChildren(span("Nothing lost in the last day.", "empty"));

  if (mkt && mkt.hubs) {
    market = mkt;
    const select = document.getElementById("hub") as HTMLSelectElement;
    const before = select.value;
    if (select.options.length !== mkt.hubs.length) {
      select.replaceChildren(...mkt.hubs.map((h) => new Option(h.station, h.station)));
      select.value = before || (mkt.hubs[0]?.station ?? "");
    }
    showMarket();
    setText(
      "market-summary",
      `${compact.format(mkt.minedM3)} m3 mined, ${compact.format(mkt.hauledM3)} m3 hauled, ${numbers.format(mkt.hullsBuilt)} hulls built, ` +
        `${numbers.format(mkt.hullsBought)} bought (${numbers.format(mkt.hullsFromNpcs)} from NPCs); players bought ${isk(mkt.soldToPlayers)} and sold ${isk(mkt.boughtFromPlayers)}; ` +
        `${numbers.format(mkt.orders)} orders up; ${isk(mkt.corpTax)} in corporation tax`,
    );
    setText("market-shortages", mkt.shortages.length ? `Short of: ${mkt.shortages.map((s) => `${s.name} (${s.wanted})`).join(", ")}` : "");
  }

  if (rel && rel.standings) {
    setText("relations-summary", `${numbers.format(rel.grudges)} pilots hold a grudge; ${numbers.format(rel.hunting)} are out hunting`);
    list("relations", [
      { title: "Side", key: (r) => r.side, cell: (r) => [r.side] },
      { title: "Player", key: (r) => r.player, cell: (r) => [r.player] },
      { title: "Heat", num: true, key: (r) => r.heat, cell: (r) => [r.heat ? span(r.heat.toFixed(1), "hostile") : ""] },
      { title: "Favor", num: true, key: (r) => r.favor, cell: (r) => [r.favor ? r.favor.toFixed(1) : ""] },
      { title: "", key: (r) => r.flags.join(), cell: (r) => [span(r.flags.join(", "), r.flags.includes("hostile") ? "hostile" : "dim")] },
    ], rel.standings, "No bot side has an opinion of any player yet.", 2, true);
    list("wingmen", [
      { title: "Wingman", key: (w) => w.bot, cell: (w) => [w.bot] },
      { title: "Flies with", key: (w) => w.player, cell: (w) => [w.player] },
      { title: "Rate", num: true, key: (w) => w.rate, cell: (w) => [`${isk(w.rate)}/h`] },
    ], rel.wingmen, "Nobody has hired a wingman.");
  }

  if (followed) {
    followedNow = followed;
    if (chosenPilot === null || !followed.some((f) => f.id === chosenPilot)) {
      chosenPilot = followed[0]?.id ?? null;
    }
    showFollowed();
  }

  if (events) {
    const state = events.controls ? "Controls are on." : "Controls are off: set bots.dashboardControls in rmr.yml to use them.";
    setText("controls-state", state);
    for (const b of document.querySelectorAll<HTMLButtonElement>("#w-control button")) {
      b.disabled = !events.controls;
    }
    list("raids", [
      { title: "Fleet", key: (r) => r.kind ?? "Raid", cell: (r) => [span(r.kind ?? "Raid", "")] },
      { title: "Where", key: (r) => r.system, cell: (r) => [span(r.system, "hostile")] },
      { title: "Pilots", num: true, key: (r) => r.raiders, cell: (r) => count(r.raiders) },
      { title: "Kills", num: true, key: (r) => r.kills, cell: (r) => count(r.kills) },
      { title: "Lost", num: true, key: (r) => r.lost, cell: (r) => count(r.lost) },
    ], events.raids, "No raids or roaming gangs out.");
    list("events", [
      { title: "Time", key: (n) => n.time, cell: (n) => [eveTime(n.time)] },
      { title: "What happened", key: (n) => n.text, cell: (n) => [n.text] },
    ], [...events.said, ...events.log], "No upheavals yet.", 0, true);
  }
}

function setText(id: string, text: string): void {
  const e = document.getElementById(id);
  if (e) {
    e.textContent = text;
  }
}

function showMarket(): void {
  if (!market) {
    return;
  }
  const chosen = (document.getElementById("hub") as HTMLSelectElement).value;
  const hub = market.hubs.find((h) => h.station === chosen) ?? market.hubs[0];
  list<MarketLine>("market", [
    { title: "Item", key: (g) => g.name, cell: (g) => [g.name] },
    { title: "Kind", key: (g) => g.kind, cell: (g) => [span(g.kind, "dim")] },
    { title: "In stock", num: true, key: (g) => g.stock, cell: (g) => count(g.stock) },
    { title: "Bots sell at", num: true, key: (g) => g.ask, cell: (g) => [price(g.ask)] },
    { title: "Bots buy at", num: true, key: (g) => (g.kind === "ship" ? 0 : g.bid), cell: (g) => [g.kind === "ship" ? "" : price(g.bid)] },
    { title: "NPC ask", num: true, key: (g) => g.npcAsk, cell: (g) => [span(price(g.npcAsk), "dim")] },
  ], hub ? hub.goods : [], "The bots hold nothing here yet.", 2, true);
}

function showFollowed(): void {
  list<Followed>("followed", [
    { title: "Pilot", key: (f) => f.name, cell: (f) => [pilotButton(f), span(` ${f.role}`, "dim")] },
    { title: "Corporation", key: (f) => f.corporation, cell: (f) => [f.corporation, f.alliance ? span(` ${f.alliance}`, "dim") : ""] },
    { title: "Ship", key: (f) => f.ship, cell: (f) => [f.ship, span(` T${f.tier}`, "dim")] },
    { title: "Where", key: (f) => f.system, cell: (f) => [security(f.security), ` ${f.system}`, f.station ? span(" (docked)", "dim") : ""] },
    { title: "Doing", key: (f) => f.activity, cell: (f) => [f.online ? f.activity : span("offline", "dim")] },
    { title: "Wallet", num: true, key: (f) => f.isk, cell: (f) => [isk(f.isk)] },
    { title: "", key: () => "", cell: (f) => [unfollowButton(f)] },
  ], followedNow, "Nobody followed. Search for a pilot's name above.");
  const chosen = followedNow.find((f) => f.id === chosenPilot);
  const ol = document.getElementById("history") as HTMLOListElement;
  ol.replaceChildren(
    ...(chosen?.history ?? []).map((h) => {
      const li = document.createElement("li");
      const t = document.createElement("time");
      t.textContent = eveTime(h.time);
      li.append(t, h.text);
      return li;
    }),
  );
}

function pilotButton(f: Followed): HTMLElement {
  const b = document.createElement("button");
  b.type = "button";
  b.className = "pick";
  b.textContent = f.name;
  b.addEventListener("click", () => {
    chosenPilot = f.id;
    showFollowed();
  });
  return b;
}

function unfollowButton(f: Followed): HTMLElement {
  const b = document.createElement("button");
  b.type = "button";
  b.className = "pick";
  b.textContent = "Stop";
  b.title = "Stop following";
  b.addEventListener("click", () => void control(`unfollow ${f.id}`));
  return b;
}

async function control(what: string): Promise<void> {
  const reply = document.getElementById("control-reply") as HTMLElement;
  try {
    const response = await fetch(`/api/control?do=${encodeURIComponent(what)}`, { method: "POST" });
    const body = (await response.json()) as { ok?: string; error?: string };
    reply.textContent = body.ok ?? body.error ?? "";
    reply.className = body.ok ? "note" : "note hostile";
  } catch (e) {
    reply.textContent = `The server did not answer (${e instanceof Error ? e.message : String(e)})`;
    reply.className = "note hostile";
  }
  setTimeout(() => void refresh(), 1_500);
}

let searchTimer: number | undefined;

function searchPilots(): void {
  const q = (document.getElementById("pilot-search") as HTMLInputElement).value.trim();
  const out = document.getElementById("pilot-results") as HTMLElement;
  if (q.length < 2) {
    out.replaceChildren();
    return;
  }
  void get<{ id: number; name: string }[]>(`/api/pilots?q=${encodeURIComponent(q)}`).then((found) => {
    out.replaceChildren(
      ...found.map((p) => {
        const b = document.createElement("button");
        b.type = "button";
        b.className = "pick";
        b.textContent = p.name;
        b.title = "Follow";
        b.addEventListener("click", () => {
          chosenPilot = p.id;
          void control(`follow ${p.id}`);
        });
        return b;
      }),
    );
  });
}

(document.getElementById("hub") as HTMLSelectElement).addEventListener("change", showMarket);
(document.getElementById("users-refresh") as HTMLButtonElement).addEventListener("click", () => void refreshUsers());
(document.getElementById("article-form") as HTMLFormElement).addEventListener("submit", (e) => void publishArticle(e as SubmitEvent));
(document.querySelector("#article-form textarea[name=body]") as HTMLTextAreaElement).addEventListener("input", schedulePreview);
for (const button of document.querySelectorAll<HTMLButtonElement>("#article-format button")) {
  button.addEventListener("mousedown", (e) => e.preventDefault());
  button.addEventListener("click", () => formatArticle(button));
}
(document.getElementById("seeding-form") as HTMLFormElement).addEventListener("submit", (e) => { e.preventDefault(); void refreshSeeding(); });
(document.getElementById("seeding-refresh") as HTMLButtonElement).addEventListener("click", () => void refreshSeeding());
(document.getElementById("downtime-form") as HTMLFormElement).addEventListener("submit", (e) => { e.preventDefault(); void changeDowntime("schedule"); });
(document.getElementById("downtime-cancel") as HTMLButtonElement).addEventListener("click", () => void changeDowntime("cancel"));
(document.getElementById("downtime-now") as HTMLButtonElement).addEventListener("click", () => void changeDowntime("now"));
(document.getElementById("restart-countdown") as HTMLButtonElement).addEventListener("click", () => void changeDowntime("restart"));
(document.getElementById("restart-now") as HTMLButtonElement).addEventListener("click", () => void changeDowntime("restart-now"));
(document.getElementById("bots-count") as HTMLInputElement).addEventListener("input", () => { botCountEdited = true; });
(document.getElementById("bots-count-form") as HTMLFormElement).addEventListener("submit", (e) => {
  e.preventDefault();
  const count = botCountInput();
  if (count !== null) void changeBots({ count });
});
(document.getElementById("bots-count-restart") as HTMLButtonElement).addEventListener("click", () => {
  const count = botCountInput();
  if (count !== null) void changeBots({ count, restart: true });
});
for (const group of document.querySelectorAll<HTMLElement>(".bot-choice[data-setting]")) {
  for (const button of group.querySelectorAll<HTMLButtonElement>("button[data-value]")) {
    button.addEventListener("click", () => {
      const raw = button.dataset.value ?? "";
      const value = raw === "true" ? true : raw === "false" ? false : raw;
      void changeBots({ [group.dataset.setting ?? ""]: value });
    });
  }
}
(document.getElementById("bots-activity") as HTMLInputElement).addEventListener("input", (e) => {
  botActivityEdited = true;
  setText("bots-activity-value", `${Number((e.target as HTMLInputElement).value).toFixed(1)}x`);
});
(document.getElementById("bots-activity-form") as HTMLFormElement).addEventListener("submit", (e) => {
  e.preventDefault();
  void changeBots({ onlineActivity: Number((document.getElementById("bots-activity") as HTMLInputElement).value) });
});
(document.getElementById("bots-shown-form") as HTMLFormElement).addEventListener("submit", (e) => {
  e.preventDefault();
  const input = document.getElementById("bots-shown") as HTMLInputElement;
  if (!input.checkValidity()) { input.reportValidity(); return; }
  void changeBots({ shownPerSystem: Number(input.value) });
});
(document.getElementById("bots-refresh") as HTMLButtonElement).addEventListener("click", () => void refreshBotSettings());
(document.getElementById("killboard-search-form") as HTMLFormElement).addEventListener("submit", (e) => { e.preventDefault(); void searchKillboardPilots(); });
(document.getElementById("kill-search") as HTMLInputElement).addEventListener("input", showKillList);
(document.getElementById("kills-prev") as HTMLButtonElement).addEventListener("click", () => { if (killPage > 0) { killPage--; void refresh(); } });
(document.getElementById("kills-next") as HTMLButtonElement).addEventListener("click", () => { if ((killPage + 1) * 30 < killTotal) { killPage++; void refresh(); } });
(document.getElementById("pilot-search") as HTMLInputElement).addEventListener("input", () => {
  clearTimeout(searchTimer);
  searchTimer = window.setTimeout(searchPilots, 250);
});
for (const b of document.querySelectorAll<HTMLButtonElement>("#event-buttons button")) {
  b.addEventListener("click", () => void control(b.dataset.do ?? ""));
}
(document.getElementById("war-form") as HTMLFormElement).addEventListener("submit", (e) => {
  e.preventDefault();
  const f = new FormData(e.target as HTMLFormElement);
  void control(`war ${f.get("a")} | ${f.get("b")}`);
});
(document.getElementById("density-form") as HTMLFormElement).addEventListener("submit", (e) => {
  e.preventDefault();
  const f = new FormData(e.target as HTMLFormElement);
  void control(`density ${f.get("region")} ${f.get("factor")}`);
});

// ---- the sovereignty map ------------------------------------------------------------
// In the manner of verite.space: territory drawn as alliance influence that
// fades with distance from each held system, borders where it meets another
// alliance's, and tickers over each contiguous block. The server's
// sovereignty resolution moves systems between alliances; the map follows.

interface SovStatic {
  systems: [number, string, number, number, number, number][];
  jumps: [number, number][];
  regions: [number, string][];
}

interface SovAlliance { id: number; name: string; ticker: string; systems: number; bots: boolean; pilots: number }

interface SovState {
  systems: [number, number, number | null][];
  alliances: SovAlliance[];
  recent: { id: number; system: string; region: string; alliance: string; ticker: string; since: number | null }[];
  sieges: { system: string; attacker: string; defender: string; starts: number; fighting: boolean }[];
  now: number;
}

interface MapSystem { id: number; name: string; region: number; x: number; y: number; sec: number; holder: number | null; since: number | null }

const SOV_REFRESH_MS = 60_000;

const sov = {
  loaded: false,
  loading: null as Promise<void> | null,
  systems: [] as MapSystem[],
  byId: new Map<number, MapSystem>(),
  byName: new Map<string, MapSystem>(),
  jumps: [] as [MapSystem, MapSystem][],
  regions: new Map<number, { name: string; x: number; y: number; nullsec: boolean }>(),
  alliances: new Map<number, SovAlliance>(),
  order: [] as SovAlliance[],
  rgb: new Map<number, [number, number, number]>(),
  palette: new Map<number, string>(),
  labels: [] as { alliance: number; x: number; y: number; count: number }[],
  sieges: [] as SovState["sieges"],
  influence: null as HTMLCanvasElement | null,
  field: null as { width: number; height: number; winners: Int32Array; strengths: Float32Array } | null,
  bounds: { minX: 0, maxX: 1, minY: 0, maxY: 1 },
  sigma: 1,
  view: { cx: 0, cy: 0, scale: 1 },
  fitted: false,
  focus: null as number | null,
  hover: null as MapSystem | null,
  signature: "",
  timer: 0,
  queued: false,
  stars: [] as [number, number, number, number][],
  marked: null as MapSystem | null,
  markedAt: 0,
};

function hslToRgb(css: string): [number, number, number] {
  const m = css.match(/hsl\((\d+(?:\.\d+)?) (\d+(?:\.\d+)?)% (\d+(?:\.\d+)?)%\)/);
  if (!m) return [128, 128, 128];
  const h = Number(m[1]) / 360, s = Number(m[2]) / 100, l = Number(m[3]) / 100;
  const q = l < 0.5 ? l * (1 + s) : l + s - l * s;
  const p = 2 * l - q;
  const channel = (t: number) => {
    t = (t + 1) % 1;
    if (t < 1 / 6) return p + (q - p) * 6 * t;
    if (t < 1 / 2) return q;
    if (t < 2 / 3) return p + (q - p) * (2 / 3 - t) * 6;
    return p;
  };
  return [Math.round(channel(h + 1 / 3) * 255), Math.round(channel(h) * 255), Math.round(channel(h - 1 / 3) * 255)];
}

function allianceColour(id: number): string {
  return sov.palette.get(id) ?? colourFor(id);
}

/// Well-separated hues for the holding alliances, by the golden angle in the
/// order of their IDs, so a colour stays with its alliance between refreshes.
function buildSovPalette(alliances: SovAlliance[]): void {
  const ids = alliances.map((a) => a.id).sort((a, b) => a - b);
  sov.palette = new Map(ids.map((id, i) => {
    const hue = Math.round((i * 137.508 + 25) % 360);
    const light = [56, 48, 63][Math.floor(i / 8) % 3];
    return [id, `hsl(${hue} 72% ${light}%)`];
  }));
  sov.rgb = new Map(ids.map((id) => [id, hslToRgb(sov.palette.get(id)!)]));
}

async function openSovMap(): Promise<void> {
  if (!sov.loading) sov.loading = loadSovStatic();
  await sov.loading;
  resizeSovCanvas();
  await refreshSov();
  if (!sov.timer) sov.timer = window.setInterval(() => void refreshSov(), SOV_REFRESH_MS);
}

async function loadSovStatic(): Promise<void> {
  try {
    const data = await get<SovStatic>("/api/sovmap/static");
    sov.systems = data.systems.map(([id, name, region, x, z, sec]) => ({ id, name, region, x, y: z, sec, holder: null, since: null }));
    sov.byId = new Map(sov.systems.map((s) => [s.id, s]));
    sov.byName = new Map(sov.systems.map((s) => [s.name, s]));
    sov.jumps = data.jumps.flatMap(([a, b]) => {
      const from = sov.byId.get(a), to = sov.byId.get(b);
      return from && to ? [[from, to] as [MapSystem, MapSystem]] : [];
    });
    const xs = sov.systems.map((s) => s.x), ys = sov.systems.map((s) => s.y);
    const pad = 0.04 * Math.max(Math.max(...xs) - Math.min(...xs), Math.max(...ys) - Math.min(...ys));
    sov.bounds = { minX: Math.min(...xs) - pad, maxX: Math.max(...xs) + pad, minY: Math.min(...ys) - pad, maxY: Math.max(...ys) + pad };
    const names = new Map(data.regions);
    const sums = new Map<number, { x: number; y: number; n: number; nullsec: number }>();
    for (const s of sov.systems) {
      const sum = sums.get(s.region) ?? { x: 0, y: 0, n: 0, nullsec: 0 };
      sum.x += s.x; sum.y += s.y; sum.n++; if (s.sec < 0.05) sum.nullsec++;
      sums.set(s.region, sum);
    }
    for (const [id, sum] of sums) {
      sov.regions.set(id, { name: names.get(id) ?? "", x: sum.x / sum.n, y: sum.y / sum.n, nullsec: sum.nullsec > sum.n / 2 });
    }
    sov.sigma = typicalSpacing() * 2.3;
    const systemNames = document.getElementById("sov-system-names");
    if (systemNames) systemNames.replaceChildren(...sov.systems.map((s) => { const o = document.createElement("option"); o.value = s.name; return o; }).sort((a, b) => a.value.localeCompare(b.value)));
    sov.loaded = true;
  } catch (e) {
    sov.loading = null;
    setText("sov-updated", `Map unavailable: ${String(e)}`);
    throw e;
  }
}

/// The median distance from a system to its nearest neighbour, which sets how
/// far each system's influence reaches.
function typicalSpacing(): number {
  const { minX, maxX } = sov.bounds;
  const size = (maxX - minX) / 160;
  const grid = new Map<string, MapSystem[]>();
  const key = (x: number, y: number) => `${Math.floor(x / size)},${Math.floor(y / size)}`;
  for (const s of sov.systems) {
    const k = key(s.x, s.y);
    (grid.get(k) ?? grid.set(k, []).get(k)!).push(s);
  }
  const nearest: number[] = [];
  for (const s of sov.systems) {
    let best = Infinity;
    const cx = Math.floor(s.x / size), cy = Math.floor(s.y / size);
    for (let dx = -2; dx <= 2; dx++) for (let dy = -2; dy <= 2; dy++) {
      for (const o of grid.get(`${cx + dx},${cy + dy}`) ?? []) {
        if (o === s) continue;
        const d = Math.hypot(o.x - s.x, o.y - s.y);
        if (d < best) best = d;
      }
    }
    if (best < Infinity) nearest.push(best);
  }
  nearest.sort((a, b) => a - b);
  return nearest[Math.floor(nearest.length / 2)] || size;
}

async function refreshSov(): Promise<void> {
  if (!sov.loaded || document.body.dataset.page !== "sovereignty") return;
  try {
    const state = await get<SovState>("/api/sovmap");
    sov.alliances = new Map(state.alliances.map((a) => [a.id, a]));
    sov.order = state.alliances;
    sov.sieges = state.sieges;
    for (const s of sov.systems) { s.holder = null; s.since = null; }
    for (const [id, alliance, since] of state.systems) {
      const s = sov.byId.get(id);
      if (s) { s.holder = alliance; s.since = since; }
    }
    const signature = state.systems.map(([id, a]) => `${id}:${a}`).join(",");
    if (signature !== sov.signature) {
      sov.signature = signature;
      buildSovPalette(state.alliances);
      buildSovLabels();
      buildInfluence();
    }
    showSovPanels(state);
    const at = new Date(state.now);
    setText("sov-updated", `${numbers.format(state.systems.length)} systems held · updated ${pad(at.getUTCHours())}:${pad(at.getUTCMinutes())}:${pad(at.getUTCSeconds())} EVE · refreshes every minute`);
    drawSov();
  } catch (e) {
    setText("sov-updated", `Could not refresh sovereignty: ${String(e)}`);
  }
}

/// Each alliance's contiguous blocks of space (linked by stargates), labelled
/// at the member system nearest the block's centre.
function buildSovLabels(): void {
  const parent = new Map<number, number>();
  const find = (id: number): number => {
    let root = id;
    while (parent.get(root) !== root) root = parent.get(root)!;
    let walk = id;
    while (parent.get(walk) !== root) { const next = parent.get(walk)!; parent.set(walk, root); walk = next; }
    return root;
  };
  for (const s of sov.systems) if (s.holder !== null) parent.set(s.id, s.id);
  for (const [a, b] of sov.jumps) {
    if (a.holder !== null && a.holder === b.holder) parent.set(find(a.id), find(b.id));
  }
  const blocks = new Map<number, MapSystem[]>();
  for (const s of sov.systems) {
    if (s.holder === null) continue;
    const root = find(s.id);
    (blocks.get(root) ?? blocks.set(root, []).get(root)!).push(s);
  }
  const largest = new Map<number, number>();
  for (const members of blocks.values()) {
    const holder = members[0].holder!;
    largest.set(holder, Math.max(largest.get(holder) ?? 0, members.length));
  }
  sov.labels = [];
  for (const members of blocks.values()) {
    const holder = members[0].holder!;
    if (members.length < 4 && members.length < (largest.get(holder) ?? 0)) continue;
    const cx = members.reduce((sum, s) => sum + s.x, 0) / members.length;
    const cy = members.reduce((sum, s) => sum + s.y, 0) / members.length;
    const anchor = members.reduce((best, s) => (Math.hypot(s.x - cx, s.y - cy) < Math.hypot(best.x - cx, best.y - cy) ? s : best));
    sov.labels.push({ alliance: holder, x: anchor.x, y: anchor.y, count: members.length });
  }
  sov.labels.sort((a, b) => b.count - a.count);
}

/// The influence field, computed once per change of hands over a grid
/// covering the whole cluster: which alliance's influence is strongest at
/// each point, and how strong. Panning, zooming and focusing reuse it.
function buildInfluence(): void {
  const { minX, maxX, minY, maxY } = sov.bounds;
  const width = 1000;
  const height = Math.max(1, Math.round((width * (maxY - minY)) / (maxX - minX)));
  const sigma = sov.sigma;
  const reach = sigma * 2.6;
  const twoSigmaSq = 2 * sigma * sigma;
  const grid = new Map<string, MapSystem[]>();
  for (const s of sov.systems) {
    if (s.holder === null) continue;
    const k = `${Math.floor(s.x / reach)},${Math.floor(s.y / reach)}`;
    (grid.get(k) ?? grid.set(k, []).get(k)!).push(s);
  }
  const winners = new Int32Array(width * height).fill(-1);
  const strengths = new Float32Array(width * height);
  const ids: number[] = [], values: number[] = [];
  for (let py = 0; py < height; py++) {
    const wy = minY + ((py + 0.5) / height) * (maxY - minY);
    const gy = Math.floor(wy / reach);
    for (let px = 0; px < width; px++) {
      const wx = minX + ((px + 0.5) / width) * (maxX - minX);
      const gx = Math.floor(wx / reach);
      ids.length = 0; values.length = 0;
      for (let dx = -1; dx <= 1; dx++) for (let dy = -1; dy <= 1; dy++) {
        const cell = grid.get(`${gx + dx},${gy + dy}`);
        if (!cell) continue;
        for (const s of cell) {
          const d2 = (s.x - wx) ** 2 + (s.y - wy) ** 2;
          if (d2 > reach * reach) continue;
          const w = Math.exp(-d2 / twoSigmaSq);
          const at = ids.indexOf(s.holder!);
          if (at < 0) { ids.push(s.holder!); values.push(w); } else values[at] += w;
        }
      }
      if (!ids.length) continue;
      let best = 0;
      for (let i = 1; i < ids.length; i++) if (values[i] > values[best]) best = i;
      const index = py * width + px;
      winners[index] = ids[best];
      strengths[index] = values[best];
    }
  }
  sov.field = { width, height, winners, strengths };
  paintInfluence();
}

/// Colours the influence field: each alliance's territory with a brighter
/// border, the rest faded while one alliance is in focus.
function paintInfluence(): void {
  const field = sov.field;
  if (!field) return;
  const { width, height, winners, strengths } = field;
  const canvas = sov.influence ?? document.createElement("canvas");
  canvas.width = width; canvas.height = height;
  const ctx = canvas.getContext("2d")!;
  const image = ctx.createImageData(width, height);
  const data = image.data;
  const threshold = 0.33;
  for (let py = 0; py < height; py++) {
    for (let px = 0; px < width; px++) {
      const index = py * width + px;
      const owner = winners[index];
      if (owner < 0 || strengths[index] < threshold) continue;
      const rgb = sov.rgb.get(owner) ?? [128, 128, 128];
      const neighbours = [px + 1 < width ? index + 1 : -1, px > 0 ? index - 1 : -1, py + 1 < height ? index + width : -1, py > 0 ? index - width : -1];
      const edge = neighbours.some((n) => n >= 0 && (winners[n] !== owner || strengths[n] < threshold));
      const fill = Math.min(1, (strengths[index] - threshold) / (1.4 - threshold));
      const faded = sov.focus !== null && owner !== sov.focus ? 0.22 : 1;
      const o = index * 4;
      if (edge) {
        data[o] = Math.min(255, rgb[0] + 60); data[o + 1] = Math.min(255, rgb[1] + 60); data[o + 2] = Math.min(255, rgb[2] + 60);
        data[o + 3] = Math.round(235 * faded);
      } else {
        data[o] = rgb[0]; data[o + 1] = rgb[1]; data[o + 2] = rgb[2];
        data[o + 3] = Math.round((70 + fill * 95) * faded);
      }
    }
  }
  ctx.putImageData(image, 0, 0);
  sov.influence = canvas;
}

function setSovFocus(id: number | null): void {
  sov.focus = id;
  paintInfluence();
  showSovAlliances();
  drawSov();
}

function sovCanvas(): HTMLCanvasElement { return document.getElementById("sov-canvas") as HTMLCanvasElement; }

function resizeSovCanvas(): void {
  const canvas = sovCanvas();
  const rect = canvas.getBoundingClientRect();
  if (!rect.width || !rect.height) return;
  const dpr = window.devicePixelRatio || 1;
  canvas.width = Math.round(rect.width * dpr);
  canvas.height = Math.round(rect.height * dpr);
  if (!sov.fitted && sov.loaded) fitSov(sov.bounds);
  drawSov();
}

function fitSov(box: { minX: number; maxX: number; minY: number; maxY: number }): void {
  const rect = sovCanvas().getBoundingClientRect();
  if (!rect.width || !rect.height) return;
  const w = Math.max(box.maxX - box.minX, sov.sigma * 6), h = Math.max(box.maxY - box.minY, sov.sigma * 6);
  sov.view = { cx: (box.minX + box.maxX) / 2, cy: (box.minY + box.maxY) / 2, scale: Math.min(rect.width / w, rect.height / h) * 0.9 };
  sov.fitted = true;
}

function toScreen(x: number, y: number, rect: DOMRect): [number, number] {
  return [(x - sov.view.cx) * sov.view.scale + rect.width / 2, (y - sov.view.cy) * sov.view.scale + rect.height / 2];
}

function drawSov(): void {
  if (sov.queued) return;
  sov.queued = true;
  requestAnimationFrame(() => { sov.queued = false; paintSov(); });
}

function paintSov(): void {
  const canvas = sovCanvas();
  const rect = canvas.getBoundingClientRect();
  if (!sov.loaded || !rect.width) return;
  const ctx = canvas.getContext("2d")!;
  const dpr = canvas.width / rect.width;
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  ctx.clearRect(0, 0, rect.width, rect.height);
  const scale = sov.view.scale;
  const showEmpire = (document.getElementById("sov-empire") as HTMLInputElement).checked;
  const visible = (sx: number, sy: number) => sx > -40 && sy > -40 && sx < rect.width + 40 && sy < rect.height + 40;

  // A faint starfield that drifts slowly as the map pans.
  if (!sov.stars.length) {
    for (let i = 0; i < 420; i++) sov.stars.push([Math.random(), Math.random(), Math.random() * 0.9 + 0.3, Math.random() * 0.45 + 0.08]);
  }
  const driftX = -sov.view.cx * sov.view.scale * 0.02, driftY = -sov.view.cy * sov.view.scale * 0.02;
  for (const [fx, fy, size, alpha] of sov.stars) {
    const x = ((fx * rect.width + driftX) % rect.width + rect.width) % rect.width;
    const y = ((fy * rect.height + driftY) % rect.height + rect.height) % rect.height;
    ctx.fillStyle = `rgba(190,215,235,${alpha})`;
    ctx.fillRect(x, y, size, size);
  }

  if (sov.influence) {
    const { minX, maxX, minY, maxY } = sov.bounds;
    const [x0, y0] = toScreen(minX, minY, rect);
    ctx.imageSmoothingEnabled = true;
    ctx.imageSmoothingQuality = "high";
    ctx.drawImage(sov.influence, x0, y0, (maxX - minX) * scale, (maxY - minY) * scale);
  }

  if ((document.getElementById("sov-gates") as HTMLInputElement).checked) {
    ctx.lineWidth = scale > 40 ? 1 : 0.6;
    for (const empire of [true, false]) {
      ctx.strokeStyle = empire ? (showEmpire ? "rgba(140,160,175,0.22)" : "rgba(140,160,175,0.07)") : "rgba(150,180,200,0.26)";
      ctx.beginPath();
      for (const [a, b] of sov.jumps) {
        if ((a.sec >= 0.05 || b.sec >= 0.05) !== empire) continue;
        const [ax, ay] = toScreen(a.x, a.y, rect), [bx, by] = toScreen(b.x, b.y, rect);
        if (!visible(ax, ay) && !visible(bx, by)) continue;
        ctx.moveTo(ax, ay); ctx.lineTo(bx, by);
      }
      ctx.stroke();
    }
  }

  const dot = Math.max(0.9, Math.min(3.4, scale * 0.045));
  for (const s of sov.systems) {
    const [sx, sy] = toScreen(s.x, s.y, rect);
    if (!visible(sx, sy)) continue;
    let colour: string;
    if (s.holder !== null) {
      const rgb = sov.rgb.get(s.holder) ?? [200, 200, 200];
      const dimmed = sov.focus !== null && s.holder !== sov.focus;
      colour = `rgba(${Math.min(255, rgb[0] + 70)},${Math.min(255, rgb[1] + 70)},${Math.min(255, rgb[2] + 70)},${dimmed ? 0.35 : 1})`;
    } else if (s.sec < 0.05) {
      colour = "rgba(150,158,165,0.55)";
    } else {
      const sc = SECURITY_COLOURS[Math.max(0, Math.round(Math.min(1, s.sec) * 10))];
      colour = showEmpire ? sc : "rgba(110,120,128,0.3)";
    }
    ctx.fillStyle = colour;
    ctx.beginPath();
    ctx.arc(sx, sy, s.holder !== null ? dot * 1.15 : dot * 0.85, 0, Math.PI * 2);
    ctx.fill();
  }

  // Sieges under way or announced.
  const now = Date.now();
  for (const siege of sov.sieges) {
    const s = sov.byName.get(siege.system);
    if (!s) continue;
    const [sx, sy] = toScreen(s.x, s.y, rect);
    if (!visible(sx, sy)) continue;
    const pulse = siege.fighting ? 6 + 3 * Math.sin(now / 250) : 6;
    ctx.strokeStyle = siege.fighting ? "rgba(255,90,70,0.95)" : "rgba(255,190,90,0.85)";
    ctx.lineWidth = 1.4;
    ctx.beginPath(); ctx.arc(sx, sy, pulse, 0, Math.PI * 2); ctx.stroke();
    ctx.beginPath();
    ctx.moveTo(sx - pulse - 4, sy); ctx.lineTo(sx - pulse + 2, sy);
    ctx.moveTo(sx + pulse - 2, sy); ctx.lineTo(sx + pulse + 4, sy);
    ctx.moveTo(sx, sy - pulse - 4); ctx.lineTo(sx, sy - pulse + 2);
    ctx.moveTo(sx, sy + pulse - 2); ctx.lineTo(sx, sy + pulse + 4);
    ctx.stroke();
  }

  ctx.textAlign = "center";
  ctx.textBaseline = "middle";
  if ((document.getElementById("sov-regions") as HTMLInputElement).checked) {
    ctx.font = `600 ${Math.max(10, Math.min(15, scale * 0.16))}px Bahnschrift, "Segoe UI", sans-serif`;
    for (const region of sov.regions.values()) {
      if (!region.nullsec && !showEmpire) continue;
      const [sx, sy] = toScreen(region.x, region.y, rect);
      if (!visible(sx, sy)) continue;
      ctx.fillStyle = region.nullsec ? "rgba(205,220,230,0.42)" : "rgba(205,220,230,0.22)";
      ctx.fillText(region.name.toUpperCase(), sx, sy);
    }
  }

  if ((document.getElementById("sov-names") as HTMLInputElement).checked && scale > 22) {
    ctx.font = `${Math.min(12, 8 + scale * 0.02)}px "Segoe UI", sans-serif`;
    ctx.textAlign = "left";
    for (const s of sov.systems) {
      if (s.sec >= 0.05 && !showEmpire) continue;
      const [sx, sy] = toScreen(s.x, s.y, rect);
      if (!visible(sx, sy)) continue;
      ctx.fillStyle = "rgba(220,228,234,0.75)";
      ctx.fillText(s.name, sx + dot + 3, sy);
    }
    ctx.textAlign = "center";
  }

  // Alliance tickers: the larger blocks always, smaller ones as the view zooms in.
  const placed: [number, number, number, number][] = [];
  for (const label of sov.labels) {
    if (sov.focus !== null && label.alliance !== sov.focus) continue;
    const alliance = sov.alliances.get(label.alliance);
    if (!alliance) continue;
    const size = Math.max(10, Math.min(26, 9 + Math.sqrt(label.count) * 2.4 + scale * 0.02));
    const [sx, sy] = toScreen(label.x, label.y, rect);
    if (!visible(sx, sy)) continue;
    ctx.font = `700 ${size}px Bahnschrift, "Segoe UI", sans-serif`;
    const text = alliance.ticker || alliance.name;
    const w = ctx.measureText(text).width / 2 + 3, h = size / 2 + 2;
    if (placed.some(([x, y, pw, ph]) => Math.abs(x - sx) < pw + w && Math.abs(y - sy) < ph + h)) continue;
    placed.push([sx, sy, w, h]);
    ctx.lineWidth = 3.5;
    ctx.lineJoin = "round";
    ctx.strokeStyle = "rgba(0,0,0,0.85)";
    ctx.strokeText(text, sx, sy);
    ctx.fillStyle = "#ffffff";
    ctx.fillText(text, sx, sy);
  }

  if (sov.marked) {
    const age = Date.now() - sov.markedAt;
    if (age < 6000) {
      const [sx, sy] = toScreen(sov.marked.x, sov.marked.y, rect);
      const r = dot + 6 + 10 * ((age % 1200) / 1200);
      ctx.strokeStyle = `rgba(90,184,214,${1 - (age % 1200) / 1200})`;
      ctx.lineWidth = 2;
      ctx.beginPath(); ctx.arc(sx, sy, r, 0, Math.PI * 2); ctx.stroke();
      ctx.font = `600 13px Bahnschrift, "Segoe UI", sans-serif`;
      ctx.textAlign = "left";
      ctx.lineWidth = 3; ctx.strokeStyle = "rgba(0,0,0,0.85)";
      ctx.strokeText(sov.marked.name, sx + dot + 8, sy - 10);
      ctx.fillStyle = "#bfe9f7";
      ctx.fillText(sov.marked.name, sx + dot + 8, sy - 10);
      ctx.textAlign = "center";
      drawSov();
    } else sov.marked = null;
  }

  if (sov.hover) {
    const [sx, sy] = toScreen(sov.hover.x, sov.hover.y, rect);
    ctx.strokeStyle = "#ffffff";
    ctx.lineWidth = 1.5;
    ctx.beginPath(); ctx.arc(sx, sy, dot + 5, 0, Math.PI * 2); ctx.stroke();
  }
  if (sov.sieges.some((s) => s.fighting)) drawSov();
}

function showSovPanels(state: SovState): void {
  const nullsec = sov.systems.filter((s) => s.sec < 0.05);
  const heldNull = nullsec.filter((s) => s.holder !== null).length;
  const stats = document.getElementById("sov-stats") as HTMLElement;
  stats.replaceChildren(...([
    [numbers.format(state.systems.length), "Systems held"],
    [numbers.format(state.alliances.length), "Alliances"],
    [nullsec.length ? `${Math.round((heldNull / nullsec.length) * 100)}%` : "—", "Null-sec claimed"],
  ] as [string, string][]).map(([value, label]) => {
    const tile = document.createElement("div");
    const strong = document.createElement("strong"); strong.textContent = value;
    const small = document.createElement("small"); small.textContent = label;
    tile.append(strong, small);
    return tile;
  }));
  showSovAlliances();

  const recent = document.getElementById("sov-recent") as HTMLElement;
  recent.replaceChildren(...state.recent.map((r) => {
    const li = document.createElement("li");
    const time = document.createElement("time"); time.textContent = r.since ? `${eveTime(r.since)} EVE` : "";
    li.append(time, `${r.system} (${r.region}) → ${r.alliance}${r.ticker ? ` <${r.ticker}>` : ""}`);
    li.addEventListener("click", () => centreSovOn(r.id));
    return li;
  }));
  if (!state.recent.length) recent.replaceChildren(span("No changes of hands recorded yet.", "empty"));

  const sieges = document.getElementById("sov-sieges") as HTMLElement;
  sieges.replaceChildren(...state.sieges.map((s) => {
    const li = document.createElement("li");
    const time = document.createElement("time");
    time.textContent = s.fighting ? "BATTLE UNDER WAY" : `${eveTime(s.starts)} EVE`;
    if (s.fighting) time.className = "hostile";
    li.append(time, `${s.attacker} vs ${s.defender} · ${s.system}`);
    li.addEventListener("click", () => { const target = sov.byName.get(s.system); if (target) centreSovOn(target.id); });
    return li;
  }));
  if (!state.sieges.length) sieges.replaceChildren(span("No sieges announced.", "empty"));
}

function showSovAlliances(): void {
  const filter = (document.getElementById("sov-filter") as HTMLInputElement).value.trim().toLowerCase();
  const out = document.getElementById("sov-alliances") as HTMLElement;
  const shown = sov.order.filter((a) => !filter || a.name.toLowerCase().includes(filter) || a.ticker.toLowerCase().includes(filter));
  out.replaceChildren(...shown.map((a) => {
    const li = document.createElement("li");
    if (sov.focus === a.id) li.className = "focused";
    const swatch = span("", "swatch"); swatch.style.background = allianceColour(a.id);
    const name = span(`${a.name} <${a.ticker}>`, "name"); name.title = `${a.name} <${a.ticker}> · ${numbers.format(a.pilots)} pilots`;
    li.append(swatch, name, span(a.bots ? "sim" : "players", "kind"), span(numbers.format(a.systems), "count"));
    li.addEventListener("click", () => focusSovAlliance(sov.focus === a.id ? null : a.id));
    return li;
  }));
  if (!shown.length) out.replaceChildren(span(sov.order.length ? "No alliance matches." : "No alliance holds sovereignty yet.", "empty"));
}

function focusSovAlliance(id: number | null): void {
  if (id !== null) {
    const owned = sov.systems.filter((s) => s.holder === id);
    if (owned.length) {
      fitSov({
        minX: Math.min(...owned.map((s) => s.x)), maxX: Math.max(...owned.map((s) => s.x)),
        minY: Math.min(...owned.map((s) => s.y)), maxY: Math.max(...owned.map((s) => s.y)),
      });
    }
  }
  setSovFocus(id);
}

function centreSovOn(systemID: number): void {
  const s = sov.byId.get(systemID);
  if (!s) return;
  sov.view = { cx: s.x, cy: s.y, scale: Math.max(sov.view.scale, 40) };
  sov.hover = s;
  sov.marked = s;
  sov.markedAt = Date.now();
  drawSov();
}

function sovSystemAt(clientX: number, clientY: number): MapSystem | null {
  const rect = sovCanvas().getBoundingClientRect();
  const mx = clientX - rect.left, my = clientY - rect.top;
  let best: MapSystem | null = null;
  let bestDistance = 10;
  for (const s of sov.systems) {
    const [sx, sy] = toScreen(s.x, s.y, rect);
    const d = Math.hypot(sx - mx, sy - my);
    if (d < bestDistance) { bestDistance = d; best = s; }
  }
  return best;
}

function showSovTooltip(s: MapSystem | null, clientX: number, clientY: number): void {
  const tip = document.getElementById("sov-tooltip") as HTMLElement;
  if (!s) { tip.hidden = true; return; }
  const stage = (document.getElementById("sov-stage") as HTMLElement).getBoundingClientRect();
  const title = document.createElement("strong"); title.textContent = s.name;
  const where = document.createElement("div");
  where.append(security(s.sec), ` ${sov.regions.get(s.region)?.name ?? ""}`);
  tip.replaceChildren(title, where);
  const holder = document.createElement("div"); holder.className = "holder";
  if (s.holder !== null) {
    const alliance = sov.alliances.get(s.holder);
    const swatch = span("", "swatch"); swatch.style.background = allianceColour(s.holder);
    holder.append(swatch, `${alliance?.name ?? "Unknown alliance"}${alliance?.ticker ? ` <${alliance.ticker}>` : ""}`);
    tip.append(holder);
    if (s.since) tip.append(span(`Held since ${eveTime(s.since)} EVE`, "dim small"));
  } else {
    holder.append(span(s.sec < 0.05 ? "Unclaimed" : "Empire space", "dim"));
    tip.append(holder);
  }
  const siege = sov.sieges.find((x) => x.system === s.name);
  if (siege) tip.append(span(siege.fighting ? `Battle: ${siege.attacker} vs ${siege.defender}` : `Siege by ${siege.attacker} at ${eveTime(siege.starts)}`, "hostile small"));
  tip.hidden = false;
  const x = Math.min(clientX - stage.left + 14, stage.width - tip.offsetWidth - 8);
  const y = Math.min(clientY - stage.top + 14, stage.height - tip.offsetHeight - 8);
  tip.style.left = `${Math.max(8, x)}px`;
  tip.style.top = `${Math.max(8, y)}px`;
}

function wireSovMap(): void {
  const canvas = sovCanvas();
  let drag: { x: number; y: number; cx: number; cy: number; moved: boolean } | null = null;
  canvas.addEventListener("wheel", (event) => {
    if (!sov.loaded) return;
    event.preventDefault();
    const rect = canvas.getBoundingClientRect();
    const mx = event.clientX - rect.left - rect.width / 2, my = event.clientY - rect.top - rect.height / 2;
    const wx = sov.view.cx + mx / sov.view.scale, wy = sov.view.cy + my / sov.view.scale;
    const scale = Math.max(2, Math.min(600, sov.view.scale * Math.exp(-event.deltaY * 0.0016)));
    sov.view = { cx: wx - mx / scale, cy: wy - my / scale, scale };
    drawSov();
  }, { passive: false });
  canvas.addEventListener("pointerdown", (event) => {
    drag = { x: event.clientX, y: event.clientY, cx: sov.view.cx, cy: sov.view.cy, moved: false };
    canvas.setPointerCapture(event.pointerId);
    canvas.classList.add("dragging");
  });
  canvas.addEventListener("pointermove", (event) => {
    if (drag) {
      const dx = event.clientX - drag.x, dy = event.clientY - drag.y;
      if (Math.abs(dx) + Math.abs(dy) > 3) drag.moved = true;
      sov.view.cx = drag.cx - dx / sov.view.scale;
      sov.view.cy = drag.cy - dy / sov.view.scale;
      showSovTooltip(null, 0, 0);
      drawSov();
      return;
    }
    const s = sovSystemAt(event.clientX, event.clientY);
    if (s !== sov.hover) { sov.hover = s; drawSov(); }
    showSovTooltip(s, event.clientX, event.clientY);
  });
  canvas.addEventListener("pointerup", (event) => {
    const clicked = drag && !drag.moved;
    drag = null;
    canvas.classList.remove("dragging");
    if (clicked) {
      const s = sovSystemAt(event.clientX, event.clientY);
      if (s?.holder != null) focusSovAlliance(sov.focus === s.holder ? null : s.holder);
    }
  });
  canvas.addEventListener("pointerleave", () => { sov.hover = null; showSovTooltip(null, 0, 0); drawSov(); });
  canvas.addEventListener("dblclick", () => { fitSov(sov.bounds); setSovFocus(null); });
  for (const id of ["sov-gates", "sov-regions", "sov-names", "sov-empire"]) {
    const box = document.getElementById(id) as HTMLInputElement;
    try { const saved = localStorage.getItem(`rmr.${id}`); if (saved !== null) box.checked = saved === "1"; } catch { /* storage unavailable */ }
    box.addEventListener("change", () => {
      try { localStorage.setItem(`rmr.${id}`, box.checked ? "1" : "0"); } catch { /* storage unavailable */ }
      drawSov();
    });
  }
  const find = document.getElementById("sov-find") as HTMLInputElement | null;
  const findSystem = () => {
    if (!find) return;
    const wanted = find.value.trim().toLowerCase();
    if (!wanted) return;
    const s = sov.byName.get(find.value.trim()) ?? sov.systems.find((x) => x.name.toLowerCase() === wanted) ?? sov.systems.find((x) => x.name.toLowerCase().startsWith(wanted));
    if (s) { centreSovOn(s.id); find.classList.remove("not-found"); } else find.classList.add("not-found");
  };
  find?.addEventListener("change", findSystem);
  find?.addEventListener("keydown", (e) => { if (e.key === "Enter") findSystem(); });
  const zoomBy = (factor: number) => {
    if (!sov.loaded) return;
    sov.view = { ...sov.view, scale: Math.max(2, Math.min(600, sov.view.scale * factor)) };
    drawSov();
  };
  document.getElementById("sov-zoom-in")?.addEventListener("click", () => zoomBy(1.4));
  document.getElementById("sov-zoom-out")?.addEventListener("click", () => zoomBy(1 / 1.4));
  document.addEventListener("keydown", (e) => {
    if (document.body.dataset.page !== "sovereignty" || e.ctrlKey || e.metaKey || e.altKey) return;
    const target = e.target as HTMLElement | null;
    if (target && (target.tagName === "INPUT" || target.tagName === "TEXTAREA" || target.tagName === "SELECT")) return;
    if (e.key === "+" || e.key === "=") zoomBy(1.4);
    else if (e.key === "-" || e.key === "_") zoomBy(1 / 1.4);
    else if (e.key === "r" || e.key === "R" || e.key === "Home") { fitSov(sov.bounds); setSovFocus(null); }
    else if (e.key === "Escape") setSovFocus(null);
    else if (e.key === "/") { e.preventDefault(); find?.focus(); find?.select(); }
    else return;
  });
  (document.getElementById("sov-reset") as HTMLButtonElement).addEventListener("click", () => { fitSov(sov.bounds); setSovFocus(null); });
  (document.getElementById("sov-filter") as HTMLInputElement).addEventListener("input", showSovAlliances);
  new ResizeObserver(() => { if (document.body.dataset.page === "sovereignty") resizeSovCanvas(); }).observe(document.getElementById("sov-stage") as HTMLElement);
}

wireSovMap();

// ---- the neocom clock and the minimize buttons ------------------------------------

function tick(): void {
  const now = new Date();
  (document.getElementById("clock-time") as HTMLElement).textContent = `${pad(now.getUTCHours())}:${pad(now.getUTCMinutes())}`;
  (document.getElementById("clock-date") as HTMLElement).textContent = `${now.getUTCFullYear()}.${pad(now.getUTCMonth() + 1)}.${pad(now.getUTCDate())}`;
}

for (const button of document.querySelectorAll<HTMLButtonElement>(".window .min")) {
  button.addEventListener("click", () => button.closest(".window")?.classList.toggle("minimized"));
}

const pages: Record<string, { title: string; description: string; windows: string[] }> = {
  home: { title: "Command overview", description: "Live activity across New Eden.", windows: ["w-status", "w-health", "w-wars", "w-pilots", "w-news"] },
  status: { title: "Server status", description: "Connections, simulation, uptime and how hard the machine is working.", windows: ["w-status", "w-health"] },
  wars: { title: "War report", description: "Active wars and losses.", windows: ["w-wars"] },
  pilots: { title: "Pilots online", description: "Capsuleers currently connected.", windows: ["w-pilots"] },
  systems: { title: "Busiest systems", description: "Where pilots are flying now.", windows: ["w-systems"] },
  alliances: { title: "Alliances", description: "Coalitions and their territory.", windows: ["w-alliances"] },
  corporations: { title: "Corporations", description: "Corporation roster and members.", windows: ["w-corporations"] },
  sieges: { title: "Sieges", description: "Campaigns and system assaults.", windows: ["w-sieges"] },
  news: { title: "Galactic news", description: "Published articles and recent events from across the cluster.", windows: ["w-articles", "w-news"] },
  killboard: { title: "Killboard", description: "Killmails, top killers and recent destruction.", windows: ["w-killboard"] },
  sovereignty: { title: "Sovereignty map", description: "Who holds which systems, updated as sovereignty changes hands.", windows: ["w-sovmap"] },
  downtime: { title: "Server downtime", description: "Announce and schedule a graceful shutdown or restart.", windows: ["w-downtime"] },
  bots: { title: "Bots", description: "How many bots the server plays, and how aggressive they are.", windows: ["w-bots"] },
  market: { title: "Market", description: "Trade activity and hub stock.", windows: ["w-market"] },
  seeding: { title: "Market seeding", description: "Pick a station, such as a newly built outpost, and stock its market.", windows: ["w-seeding"] },
  relations: { title: "Standings", description: "Bot relationships and grudges.", windows: ["w-relations"] },
  follow: { title: "Follow a pilot", description: "Track a pilot's current activity.", windows: ["w-follow"] },
  control: { title: "Events, raids and gangs", description: "Pirate raids and roaming gangs out now, and political events. Start one with the buttons below.", windows: ["w-control"] },
  users: { title: "User accounts", description: "Account roles and in-game staff colours.", windows: ["w-users"] },
  gm: { title: "GM commands", description: "Slash commands for staff, typed into any chat channel in game.", windows: ["w-gm"] },
};

// The GM command list: filter as you type.
document.getElementById("gm-filter")?.addEventListener("input", (event) => {
  const wanted = (event.target as HTMLInputElement).value.trim().toLowerCase();
  for (const row of document.querySelectorAll<HTMLTableRowElement>("#gm-commands tbody tr")) {
    row.hidden = wanted !== "" && !(row.textContent ?? "").toLowerCase().includes(wanted);
  }
});

function route(): void {
  const hash = window.location.hash.slice(1) || "home";
  const killID = Number(hash.match(/^kill-(\d+)$/)?.[1] ?? 0);
  const pilotID = Number(hash.match(/^pilot-(\d+)$/)?.[1] ?? 0);
  const groupMatch = hash.match(/^(corporation|alliance)-(\d+)$/);
  const groupKind = (groupMatch?.[1] ?? "") as "corporation" | "alliance" | "";
  const groupID = Number(groupMatch?.[2] ?? 0);
  const page = killID > 0 ? { title: `Killmail #${killID}`, description: "Fitting, loss and involved pilots.", windows: ["w-kill-detail"] }
    : pilotID > 0 ? { title: "Pilot record", description: "Kills, losses and ships flown.", windows: ["w-pilot-profile"] }
    : groupID > 0 ? { title: groupKind === "alliance" ? "Alliance record" : "Corporation record",
      description: "Kills, losses, top pilots and ships.", windows: ["w-group-profile"] }
    : pages[hash] ?? pages.home;
  setText("page-title", page.title);
  setText("page-description", page.description);
  setText("page-kicker", killID > 0 || pilotID > 0 || groupID > 0 || hash === "killboard" ? "KILLBOARD / NEW EDEN"
    : hash === "sovereignty" ? "SOVEREIGNTY / NEW EDEN" : "CLUSTER ADMINISTRATION / NEW EDEN");
  document.title = `${page.title} · Red Moon Rising`;
  const activePage = killID > 0 || pilotID > 0 || groupID > 0 ? "killboard" : (hash in pages ? hash : "home");
  document.body.dataset.page = activePage;
  (document.getElementById("home-metrics") as HTMLElement).hidden = activePage !== "home";
  for (const window of document.querySelectorAll<HTMLElement>(".desktop .window")) {
    window.classList.toggle("page-hidden", !page.windows.includes(window.id));
  }
  (document.getElementById("w-kill-detail") as HTMLElement).hidden = killID === 0;
  (document.getElementById("w-pilot-profile") as HTMLElement).hidden = pilotID === 0;
  (document.getElementById("w-group-profile") as HTMLElement).hidden = groupID === 0;
  for (const link of document.querySelectorAll<HTMLAnchorElement>(".neocom a")) {
    if (link.hash === `#${activePage}`) link.setAttribute("aria-current", "page");
    else link.removeAttribute("aria-current");
  }
  if (killID > 0) void showKill(killID);
  if (pilotID > 0) void showPilot(pilotID);
  if (groupID > 0 && groupKind) void showGroup(groupKind, groupID);
  if (hash === "users") void refreshUsers();
  if (hash === "seeding") void refreshSeeding();
  if (hash === "status" || hash === "home") void refreshHealth();
  if (hash === "bots") void refreshBotSettings();
  if (hash === "sovereignty") void openSovMap();
  window.scrollTo(0, 0);
}
window.addEventListener("hashchange", route);

tick();
setInterval(tick, 1_000);
route();
void refresh();
setInterval(() => void refresh(), REFRESH_MS);
void refreshArticles();
setInterval(() => void refreshArticles(), REFRESH_MS);
void refreshDowntime();
void refreshBotSettings();
setInterval(() => void refreshBotSettings(), 15_000);
void refreshHealth();
setInterval(() => void refreshHealth(), 2_000);
setInterval(() => void refreshDowntime(), 5_000);
setInterval(renderDowntime, 1_000);
