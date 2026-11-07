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
  name: string;
  ticker: string | null;
  alliance: string | null;
  ship: string | null;
  system?: string;
  systemSecurity?: number;
  docked?: boolean;
}

interface SystemRow {
  name: string;
  security: number;
  region: string;
  inSpace: number;
  docked: number;
}

interface AllianceRow {
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

interface CorporationRow {
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
  victimCorporation: string | null;
  ship: string | null;
  shipTypeID: number | null;
  shipClass: string | null;
  hullValue: number | null;
  system: string | null;
  finalBlow: string | null;
  pod: boolean;
}

interface Killboard {
  total: number;
  day: number;
  week: number;
  pods: number;
  pilots: { id: number; name: string; corporation: string; kills: number }[];
  sides: { name: string; ticker: string; kills: number; losses: number; bots: boolean }[];
  ships: { ship: string; lost: number }[];
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
  raids: { system: string; raiders: number; kills: number; lost: number }[];
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

function span(text: string, className: string): HTMLElement {
  const e = document.createElement("span");
  e.textContent = text;
  e.className = className;
  return e;
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
    if (id === "kills") {
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
  kill?: { id: number; time: number; victim: string; victimCorporation: string; victimAlliance: string; ship: string; shipTypeID: number; shipClass: string; hullValue: number; system: string; security: number; region: string; damage: number; pod: boolean };
  participants?: { pilot: string; corporation: string; alliance: string; ship: string; shipTypeID: number; weapon: string; damage: number; finalBlow: boolean }[];
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

function card(title: string, ...content: Node[]): HTMLElement {
  const section = document.createElement("section");
  section.className = "kill-card";
  const heading = document.createElement("h3");
  heading.textContent = title;
  section.append(heading, ...content);
  return section;
}

function detailLine(label: string, value: string): HTMLElement {
  const row = document.createElement("div");
  row.className = "detail-line";
  row.append(span(label, "dim"), span(value, ""));
  return row;
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
    const layout = document.createElement("div"); layout.className = "kill-layout";
    const main = document.createElement("div"); main.className = "kill-main";
    const side = document.createElement("aside"); side.className = "kill-side";
    const portrait = document.createElement("div"); portrait.className = "ship-portrait";
    portrait.append(typeImage(k.shipTypeID, "render", 512));
    const shipLabel = document.createElement("div"); shipLabel.className = "ship-label";
    shipLabel.append(span(k.shipClass || "Destroyed ship", "eyebrow"), document.createElement("strong"));
    shipLabel.querySelector("strong")!.textContent = k.ship;
    portrait.append(shipLabel);
    const fitting = document.createElement("div"); fitting.className = "fitting-ring";
    for (const [i, item] of (detail.items ?? []).filter((item) => item.flag >= 11 && item.flag <= 35).slice(0, 20).entries()) {
      const icon = document.createElement("div"); icon.className = "fitting-icon";
      const angle = (i / Math.min(20, (detail.items ?? []).length)) * Math.PI * 2 - Math.PI / 2;
      icon.style.left = `${50 + 43 * Math.cos(angle)}%`;
      icon.style.top = `${50 + 43 * Math.sin(angle)}%`;
      icon.title = item.name;
      icon.append(typeImage(item.typeID, "icon", 64));
      fitting.append(icon);
    }
    const fittingStage = document.createElement("div"); fittingStage.className = "fitting-stage";
    fittingStage.append(portrait, fitting);
    main.append(fittingStage);
    if (!detail.items?.length) main.append(span("Fitting and cargo were not recorded for this older killmail.", "kill-history-note"));
    const itemTable = document.createElement("table"); itemTable.className = "kill-items";
    const itemHead = itemTable.createTHead().insertRow();
    for (const h of ["Item", "Qty", "Base value", "Status"]) { const th = document.createElement("th"); th.textContent = h; itemHead.append(th); }
    const itemBody = itemTable.createTBody();
    for (const item of detail.items ?? []) {
      const tr = itemBody.insertRow();
      const name = tr.insertCell(); name.className = "item-name";
      name.append(typeImage(item.typeID, "icon", 64), span(item.name, ""));
      tr.insertCell().textContent = numbers.format(item.quantity);
      tr.insertCell().textContent = item.unitValue > 0 ? isk(item.unitValue * item.quantity) : "—";
      const status = tr.insertCell(); status.textContent = item.dropped ? "Dropped" : "Destroyed";
      status.className = item.dropped ? "dropped" : "hostile";
    }
    main.append(card(`Items destroyed & dropped (${detail.items?.length ?? 0})`, itemTable));
    const victim = document.createElement("div"); victim.className = "victim-name";
    victim.append(typeImage(k.shipTypeID, "icon", 64), span(k.victim, ""));
    side.append(card("Victim", victim, detailLine("Corporation", k.victimCorporation || "Unknown"), detailLine("Alliance", k.victimAlliance || "—")));
    side.append(card("Kill details", detailLine("System", `${k.system} (${k.security.toFixed(1)})`), detailLine("Region", k.region), detailLine("Time", `${eveTime(k.time)} EVE`), detailLine("Total damage", numbers.format(k.damage))));
    side.append(card("Estimated hull value", detailLine("Base price", k.hullValue > 0 ? isk(k.hullValue) : "Unavailable"), span("Historical kill values require prices that were not recorded at the time.", "dim small")));
    const finalBlow = detail.participants?.find((p) => p.finalBlow);
    side.append(card("Final blow", detailLine("Pilot", finalBlow?.pilot || "Unknown"), detailLine("Ship", finalBlow?.ship || "—"), detailLine("Weapon", finalBlow?.weapon || "—")));
    const partyList = document.createElement("div"); partyList.className = "party-list";
    for (const p of detail.participants ?? []) {
      const party = document.createElement("div"); party.className = "party";
      if (p.shipTypeID) party.append(typeImage(p.shipTypeID, "icon", 64));
      const names = document.createElement("div"); names.append(span(p.pilot, "party-name"), span([p.corporation, p.alliance].filter(Boolean).join(" · "), "dim"));
      party.append(names, span(`${numbers.format(p.damage)}${p.finalBlow ? " · FINAL BLOW" : ""}`, p.finalBlow ? "hostile" : "dim"));
      partyList.append(party);
    }
    side.append(card(`Involved parties (${detail.participants?.length ?? 0})`, partyList));
    layout.append(main, side);
    output.replaceChildren(layout);
  } catch (e) { output.replaceChildren(span(`Could not load report: ${String(e)}`, "hostile")); }
}

const count = (n: number) => [numbers.format(n)];

const compact = new Intl.NumberFormat("en-US", { notation: "compact", maximumFractionDigits: 1 });
const isk = (n: number) => `${compact.format(n)} ISK`;
const price = (n: number) => (n >= 1000 ? compact.format(n) : n.toFixed(2));

// ---- windows ----------------------------------------------------------------------

function showStatus(s: Summary): void {
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

function uptime(seconds: number): string {
  const d = Math.floor(seconds / 86400);
  const h = Math.floor((seconds % 86400) / 3600);
  const m = Math.floor((seconds % 3600) / 60);
  return d > 0 ? `${d}d ${h}h ${m}m` : h > 0 ? `${h}h ${m}m` : `${m}m`;
}

async function refresh(): Promise<void> {
  try {
    const [summary, wars, players, systems, alliances, corporations, kills, news, sieges] = await Promise.all([
      get<Summary>("/api/summary"),
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
      { title: "Name", key: (p) => p.name, cell: (p) => [p.name] },
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
      { title: "Alliance", key: (a) => a.name, cell: (a) => [a.name, allianceTicker(a.ticker)] },
      { title: "Holds", key: (a) => a.region ?? "", cell: (a) => [a.region ?? span(a.kind === "players" ? "" : a.kind, "dim")] },
      { title: "Corps", num: true, key: (a) => a.corporations, cell: (a) => count(a.corporations) },
      { title: "Pilots", num: true, key: (a) => a.pilots, cell: (a) => count(a.pilots) },
      { title: "Systems", num: true, key: (a) => a.systems, cell: (a) => (a.systems > 0 ? count(a.systems) : [span("-", "dim")]) },
    ], alliances, "No alliances have been founded.", 3, true);

    list<CorporationRow>("corporations", [
      { title: "Corporation", key: (c) => c.name, cell: (c) => [c.name, corpTicker(c.ticker), c.bots ? "" : span(" (players)", "dim")] },
      { title: "Alliance", key: (c) => c.alliance ?? "", cell: (c) => [allianceTicker(c.alliance)] },
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
  const kills = query ? killRows.filter((k) => [k.victim, k.ship, k.system, k.finalBlow].some((s) => s?.toLowerCase().includes(query))) : killRows;
  list<Kill>("kills", [
    { title: "Ship type", key: (k) => k.ship ?? "", cell: (k) => {
      const cell = document.createElement("div"); cell.className = "kill-ship-cell";
      if (k.shipTypeID) cell.append(typeImage(k.shipTypeID, "render", 64));
      const labels = document.createElement("div"); labels.append(span(k.ship ?? "Unknown ship", "ship-name"), span(k.shipClass ?? "", "dim")); cell.append(labels); return [cell];
    } },
    { title: "Victim", key: (k) => k.victim ?? "", cell: (k) => [span(k.victim ?? "Unknown", "pilot-name"), span(k.victimCorporation ?? "", "subline")] },
    { title: "Final blow", key: (k) => k.finalBlow ?? "", cell: (k) => [span(k.finalBlow ?? "Unknown", "pilot-name")] },
    { title: "Location", key: (k) => k.system ?? "", cell: (k) => [k.system ?? "Unknown"] },
    { title: "Hull base value", num: true, key: (k) => k.hullValue ?? 0, cell: (k) => [k.hullValue ? isk(k.hullValue) : "—"] },
    { title: "Time", key: (k) => k.time ?? 0, cell: (k) => [eveTime(k.time)] },
  ], kills, "No kills found on this page.", 5, true);
  setText("kills-page", `Page ${killPage + 1} of ${Math.max(1, Math.ceil(killTotal / 30))}`);
  (document.getElementById("kills-prev") as HTMLButtonElement).disabled = killPage === 0;
  (document.getElementById("kills-next") as HTMLButtonElement).disabled = (killPage + 1) * 30 >= killTotal;
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
  const stats = document.getElementById("killboard-stats") as HTMLElement;
  stats.replaceChildren(...([
    ["Total killmails", board.total], ["Kills (24h)", board.day], ["Kills (7d)", board.week], ["Pod kills", board.pods],
  ] as [string, number][]).map(([label, value]) => {
    const item = document.createElement("div"); item.className = "stat-tile";
    item.append(span(label, "eyebrow"), span(numbers.format(value), "stat-number")); return item;
  }));
  list("killboard-pilots", [
    { title: "Pilot", key: (p) => p.name, cell: (p) => [p.name, corpTicker(p.corporation)] },
    { title: "Final blows", num: true, key: (p) => p.kills, cell: (p) => count(p.kills) },
  ], board.pilots, "No kills in the last day.", 1, true);
  list("killboard", [
    { title: "Alliance / corporation", key: (k) => k.name, cell: (k) => [k.name, allianceTicker(k.ticker), k.bots ? "" : span(" (players)", "dim")] },
    { title: "Kills", num: true, key: (k) => k.kills, cell: (k) => count(k.kills) },
    { title: "Losses", num: true, key: (k) => k.losses, cell: (k) => count(k.losses) },
  ], board.sides, "Nothing has died today.", 1, true);
  list("killboard-ships", [
    { title: "Hull", key: (s) => s.ship, cell: (s) => [s.ship] },
    { title: "Lost", num: true, key: (s) => s.lost, cell: (s) => count(s.lost) },
  ], board.ships, "", 1, true);

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
      { title: "Raid on", key: (r) => r.system, cell: (r) => [span(r.system, "hostile")] },
      { title: "Raiders", num: true, key: (r) => r.raiders, cell: (r) => count(r.raiders) },
      { title: "Kills", num: true, key: (r) => r.kills, cell: (r) => count(r.kills) },
      { title: "Lost", num: true, key: (r) => r.lost, cell: (r) => count(r.lost) },
    ], events.raids, "No raids under way.");
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
  home: { title: "Command overview", description: "Live activity across New Eden.", windows: ["w-status", "w-wars", "w-pilots", "w-news"] },
  status: { title: "Server status", description: "Connections, simulation and uptime.", windows: ["w-status"] },
  wars: { title: "War report", description: "Active wars and losses.", windows: ["w-wars"] },
  pilots: { title: "Pilots online", description: "Capsuleers currently connected.", windows: ["w-pilots"] },
  systems: { title: "Busiest systems", description: "Where pilots are flying now.", windows: ["w-systems"] },
  alliances: { title: "Alliances", description: "Coalitions and their territory.", windows: ["w-alliances"] },
  corporations: { title: "Corporations", description: "Corporation roster and members.", windows: ["w-corporations"] },
  sieges: { title: "Sieges", description: "Campaigns and system assaults.", windows: ["w-sieges"] },
  news: { title: "Galactic news", description: "Recent events from across the cluster.", windows: ["w-news"] },
  killboard: { title: "Killboard", description: "Killmails, top killers and recent destruction.", windows: ["w-killboard", "w-kills"] },
  market: { title: "Market", description: "Trade activity and hub stock.", windows: ["w-market"] },
  relations: { title: "Standings", description: "Bot relationships and grudges.", windows: ["w-relations"] },
  follow: { title: "Follow a pilot", description: "Track a pilot's current activity.", windows: ["w-follow"] },
  control: { title: "Events and controls", description: "Simulation events and commands.", windows: ["w-control"] },
};

function route(): void {
  const hash = window.location.hash.slice(1) || "home";
  const killID = Number(hash.match(/^kill-(\d+)$/)?.[1] ?? 0);
  const page = killID > 0 ? { title: `Killmail #${killID}`, description: "Fitting, loss and involved pilots.", windows: ["w-kill-detail"] } : pages[hash] ?? pages.home;
  setText("page-title", page.title);
  setText("page-description", page.description);
  setText("page-kicker", killID > 0 || hash === "killboard" ? "KILLBOARD / NEW EDEN" : "COMMAND CENTER / NEW EDEN");
  document.title = `${page.title} · Red Moon Rising`;
  for (const window of document.querySelectorAll<HTMLElement>(".desktop .window")) {
    window.classList.toggle("page-hidden", !page.windows.includes(window.id));
  }
  (document.getElementById("w-kill-detail") as HTMLElement).hidden = killID === 0;
  for (const link of document.querySelectorAll<HTMLAnchorElement>(".neocom a")) {
    link.classList.toggle("active", link.hash === (killID > 0 ? "#killboard" : `#${hash}`));
  }
  if (killID > 0) void showKill(killID);
  window.scrollTo(0, 0);
}
window.addEventListener("hashchange", route);

tick();
setInterval(tick, 1_000);
route();
void refresh();
setInterval(() => void refresh(), REFRESH_MS);
