// Zecky Lock it: the window. It draws and asks; it never holds a key.
// The 24 words pass through here once, to be written down (or typed in for a
// restore), and a passphrase on its way to the Rust side. Fields are cleared
// as soon as they are sent. The lock's words stay on the Rust side.

"use strict";

const invoke = window.__TAURI__.core.invoke;
const listen = window.__TAURI__.event ? window.__TAURI__.event.listen : null;

const KEY_NAMES = { device_hello: "This PC", security_key: "Security key", passphrase: "Passphrase" };
const MIN_PASSPHRASE = 12;
/// "Really open the lock?" waits this long, as Lock it Mac's default.
const OPEN_WAIT = 10;
const NU7_HEIGHT = 3_543_405;

const $ = (id) => document.getElementById(id);

function say(element, text, calm = false) {
  element.textContent = text;
  element.classList.toggle("calm", calm);
}

function describe(problem) {
  if (problem && typeof problem === "object" && "message" in problem) return problem.cancelled ? "" : problem.message;
  return String(problem);
}

// ---------- Amounts, the Mac's way (Amount.swift) ----------

const prefs = {
  get unit() { return localStorageGet("amount.unit") === "zats" ? "zats" : "ZEC"; },
  set unit(v) { localStorageSet("amount.unit", v); },
  get hidden() { return localStorageGet("amount.hidden") === "1"; },
  set hidden(v) { localStorageSet("amount.hidden", v ? "1" : "0"); },
};

function localStorageGet(key) { try { return localStorage.getItem(key); } catch { return null; } }
function localStorageSet(key, value) { try { localStorage.setItem(key, value); } catch { /* not kept */ } }

const HIDDEN = "••••••";

function group(digits) { return digits.replace(/\B(?=(\d{3})+(?!\d))/g, ","); }

/// "1,234.567", or "<0.001" for a tiny non-zero amount.
function zecNumber(zats) {
  const sign = zats < 0 ? "-" : "";
  const abs = Math.abs(zats);
  const thousandths = Math.floor(abs / 100_000);
  if (thousandths === 0 && abs > 0) return "<0.001";
  return sign + group(String(Math.floor(thousandths / 1000))) + "." + String(thousandths % 1000).padStart(3, "0");
}

function number(zats, unit = prefs.unit) {
  return unit === "ZEC" ? zecNumber(zats) : (zats < 0 ? "-" : "") + group(String(Math.abs(zats)));
}

function amountText(zats, unit = prefs.unit) { return `${number(zats, unit)} ${unit}`; }

/// Every digit, for a payment's details: at least three decimals.
function exact(zats) {
  const abs = Math.abs(zats);
  let fraction = String(abs % 1e8).padStart(8, "0");
  while (fraction.length > 3 && fraction.endsWith("0")) fraction = fraction.slice(0, -1);
  return (zats < 0 ? "-" : "") + group(String(Math.floor(abs / 1e8))) + "." + fraction + " ZEC";
}

const numbers = new Intl.NumberFormat("en-US");

// ---------- Getting started ----------

const onboarding = document.querySelector(".onboarding");
const views = {
  loading: $("view-loading"), welcome: $("view-welcome"), words: $("view-words"),
  check: $("view-check"), restore: $("view-restore"), setup: $("view-setup"),
};

let busy = false;

function showView(name) {
  onboarding.hidden = name === "home";
  $("view-home").hidden = name !== "home";
  $("eye").hidden = name !== "home";
  for (const [key, element] of Object.entries(views)) element.hidden = key !== name;
  if (name === "home") startHome();
  else stopHome();
  onboarding.scrollTo(0, 0);
}

Lockart.padlock(document.querySelector('[data-padlock="small"]'), { size: 150 });

let words = [];
let madeIn = null;

function listWords() {
  const list = $("words-list");
  list.textContent = "";
  for (const word of words) {
    const item = document.createElement("li");
    item.textContent = word;
    list.append(item);
  }
}

$("welcome-create").addEventListener("click", async () => {
  try {
    words = await invoke("wallet_new_words");
    madeIn = null;
    listWords();
    showView("words");
  } catch (problem) {
    alert(describe(problem));
  }
});

function forgetWords() {
  words = [];
  $("words-list").textContent = "";
  $("check-form").textContent = "";
  $("restore-words").value = "";
  invoke("wallet_forget_words");
}

$("words-back").addEventListener("click", () => { forgetWords(); showView("welcome"); });

$("words-done").addEventListener("click", () => {
  const positions = new Set();
  while (positions.size < 3) positions.add(Math.floor(Math.random() * 24));
  const form = $("check-form");
  form.textContent = "";
  for (const position of [...positions].sort((a, b) => a - b)) {
    const label = document.createElement("label");
    label.className = "field";
    const span = document.createElement("span");
    span.textContent = `Word ${position + 1}`;
    const input = document.createElement("input");
    input.className = "input";
    input.spellcheck = false;
    input.autocomplete = "off";
    input.dataset.position = position;
    label.append(span, input);
    form.append(label);
  }
  $("words-list").textContent = "";
  say($("check-message"), "");
  showView("check");
  form.querySelector("input").focus();
});

$("check-back").addEventListener("click", () => { listWords(); showView("words"); });

$("check-submit").addEventListener("click", () => {
  const inputs = [...$("check-form").querySelectorAll("input")];
  if (!inputs.every((input) => input.value.trim().toLowerCase() === words[Number(input.dataset.position)])) {
    say($("check-message"), "Not quite. Look at your paper again.");
    return;
  }
  for (const input of inputs) input.value = "";
  words = [];
  openSetup();
});

const MONTHS = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
(function fillDates() {
  const month = $("restore-month");
  MONTHS.forEach((name, index) => month.append(new Option(name, index + 1)));
  const year = $("restore-year");
  const now = new Date();
  for (let y = now.getFullYear(); y >= 2018; y -= 1) year.append(new Option(String(y), y));
  month.value = String(now.getMonth() + 1);
})();

$("welcome-restore").addEventListener("click", () => {
  $("restore-words").value = "";
  say($("restore-message"), "");
  showView("restore");
  $("restore-words").focus();
});
$("restore-back").addEventListener("click", () => { forgetWords(); showView("welcome"); });
$("restore-submit").addEventListener("click", async () => {
  try {
    await invoke("wallet_use_words", { words: $("restore-words").value });
    $("restore-words").value = "";
    madeIn = { year: Number($("restore-year").value), month: Number($("restore-month").value) };
    openSetup();
  } catch (problem) {
    say($("restore-message"), describe(problem));
  }
});

const setupForm = $("setup-form");
const setupSubmit = $("setup-submit");
const setupMessage = $("setup-message");
const newPassphrase = $("new-passphrase");
const newPassphraseAgain = $("new-passphrase-again");

function chosenKinds() {
  return [...setupForm.querySelectorAll('input[name="kind"]:checked')].map((box) => box.value);
}

function refreshSetup() {
  const kinds = chosenKinds();
  $("setup-passphrase").hidden = !kinds.includes("passphrase");
  // A security key checks that it is you but gives no secret (as on Lock it
  // Mac): a lock also needs Windows Hello or a passphrase.
  const onlyCheck = kinds.length > 0 && kinds.every((kind) => kind === "security_key");
  setupSubmit.disabled = busy || kinds.length === 0 || onlyCheck;
  setupSubmit.title = onlyCheck ? "Add Windows Hello or a passphrase too" : "";
}

async function openSetup() {
  setupForm.reset();
  say(setupMessage, "");
  const available = await invoke("availability");
  const reasons = { device_hello: "Needs Windows Hello set up on this PC", security_key: "Needs Windows 10 version 1903 or later" };
  for (const row of setupForm.querySelectorAll(".key")) {
    const kind = row.dataset.kind;
    row.querySelector("input").disabled = !available[kind];
    if (!available[kind]) row.querySelector("[data-note]").textContent = reasons[kind] || "Not available here";
  }
  refreshSetup();
  showView("setup");
}

setupForm.addEventListener("change", refreshSetup);
$("setup-back").addEventListener("click", () => { forgetWords(); showView("welcome"); });

setupForm.addEventListener("submit", async (event) => {
  event.preventDefault();
  if (busy) return;
  const kinds = chosenKinds();
  let passphrase = null;
  if (kinds.includes("passphrase")) {
    if ([...newPassphrase.value].length < MIN_PASSPHRASE) return say(setupMessage, `The passphrase needs at least ${MIN_PASSPHRASE} characters.`);
    if (newPassphrase.value !== newPassphraseAgain.value) return say(setupMessage, "The two passphrases are not the same.");
    passphrase = newPassphrase.value;
  }
  const keys = kinds.map((kind) => ({ kind, label: KEY_NAMES[kind], passphrase: kind === "passphrase" ? passphrase : null }));
  busy = true;
  refreshSetup();
  say(setupMessage, "Asking the network where to start, then each key in turn. Follow the prompts from Windows.", true);
  try {
    await invoke("wallet_create", { keys, madeIn });
    newPassphrase.value = "";
    newPassphraseAgain.value = "";
    madeIn = null;
    say(setupMessage, "");
    showView("home");
  } catch (problem) {
    say(setupMessage, describe(problem) || "Nothing was set up.");
  } finally {
    passphrase = null;
    busy = false;
    refreshSetup();
  }
});

// ---------- Home: the lock ----------

const padlockHost = $("padlock");
Lockart.padlock(padlockHost, { size: 212, plate: "LOCKED" });
Lockart.drawMedallion($("medallion"));
Lockart.padlock(document.querySelector('[data-padlock="prompt"]'), { size: 62 });

let overview = null;
let activity = [];
let homeTimer = null;
let lockOpen = false;
let shownHeight = 0;
let beats = 0;
let beatToken = 0;
const heightSamples = [];

function startHome() {
  if (homeTimer) return;
  refreshHome();
  homeTimer = setInterval(refreshHome, 4000);
}

function stopHome() {
  clearInterval(homeTimer);
  homeTimer = null;
}

async function refreshHome() {
  try {
    overview = await invoke("wallet_overview");
  } catch (problem) {
    $("status-text").textContent = describe(problem);
    return;
  }
  if (!overview.exists) return start();
  activity = await invoke("wallet_activity").catch(() => activity);
  lockOpen = overview.lock_open;
  renderHome();
  if (!$("sheet-sync").hidden) renderSync();
  if (!$("sheet-activity").hidden) renderActivity();
}

/// Days the Zcash has been in the lock; null while unknown (as Lock it Mac).
function heldDays() {
  const b = overview && overview.balance;
  if (!b || b.total <= 0) return null;
  const confirmed = activity.filter((a) => a.height && !a.expired).map((a) => ({ height: a.height, time: a.time || 0, change: a.change }));
  const pendingIn = activity.filter((a) => !a.height && !a.expired && a.change > 0).reduce((s, a) => s + a.change, 0);
  if (!Holding.historyComplete(confirmed, pendingIn, b.total)) return null;
  const since = Holding.since(confirmed);
  return since === null ? null : Holding.days(since);
}

function renderHome() {
  const b = overview.balance || { total: 0, spendable: 0, pending: 0, chain_tip: 0 };
  const hidden = prefs.hidden;
  $("eye").classList.toggle("on", hidden);
  $("eye").title = hidden ? "Show amounts" : "Hide amounts";

  const text = hidden ? HIDDEN : number(b.total);
  const numberEl = $("balance-number");
  numberEl.textContent = text;
  numberEl.style.fontSize = `${text.length <= 8 ? 60 : text.length <= 11 ? 48 : text.length <= 14 ? 38 : 30}px`;
  $("balance-unit").textContent = prefs.unit;
  $("balance").title = hidden ? "" : `Click to show in ${prefs.unit === "ZEC" ? "zats" : "ZEC"}`;

  const days = heldDays();
  $("held-line").textContent = b.total > 0 ? (days === null ? "Locked on this PC" : `Locked ${Holding.heldFor(days)}`) : "";
  const ring = $("milestone");
  Lockart.milestone(ring, Holding.progress(days ?? 0), days !== null && Holding.reachedToday(days));
  ring.classList.toggle("dim", days === null);
  const engraved = days === null ? 0 : Holding.engraved(days);
  if (engraved !== renderHome.engraved) {
    Lockart.drawEngraved($("engraved"), engraved, days !== null && Holding.engravedToday(days));
    renderHome.engraved = engraved;
  }

  // The lock and its plate.
  Lockart.setOpen(padlockHost, lockOpen);
  Lockart.setPlate(padlockHost, lockOpen ? "OPEN" : "LOCKED");
  padlockHost.title = lockOpen ? "" : "Open the lock";
  $("actions-locked").hidden = lockOpen;
  $("actions-open").hidden = !lockOpen;

  // A calm payment line while one waits for its block.
  const waiting = activity.find((a) => !a.height && !a.expired);
  const line = $("payment-line");
  let lineText = null;
  if (overview.unsent) lineText = hidden ? "Not sent yet" : "A signed payment is not sent yet";
  else if (waiting) {
    const what = waiting.change > 0 ? "Receiving" : "Sending";
    lineText = hidden ? what : `${what} ${waiting.change > 0 ? "+" : "−"}${amountText(Math.abs(waiting.change))}`;
  }
  line.textContent = lineText ? `${lineText} ›` : " ";
  line.classList.toggle("shown", !!lineText);
  line.classList.toggle("attention", !!overview.unsent);
  line.disabled = !lineText;

  // The chain's heartbeat.
  heartbeat(b.chain_tip, overview.sync.state === "up_to_date");
  if (b.chain_tip > 0) {
    heightSamples.push({ at: Date.now(), height: b.scanned_height });
    while (heightSamples.length > 12) heightSamples.shift();
  }

  // The status line.
  const sync = overview.sync;
  const dot = $("status-dot");
  const status = $("status-text");
  dot.className = "dot";
  status.classList.remove("attention");
  const behind = b.chain_tip > 0 ? b.chain_tip - b.scanned_height : null;
  const nearly = behind !== null && behind <= 3;
  if (sync.state === "problem" && !nearly) {
    dot.classList.add("red");
    status.classList.add("attention");
    status.textContent = `Can't reach ${sync.server || "the server"}`;
    status.title = sync.problem || "";
  } else if (sync.state === "up_to_date" || nearly) {
    dot.classList.add("green");
    status.textContent = `Up to date${sync.server ? " · " + sync.server : ""}`;
    status.title = "Sync details";
  } else {
    const percent = syncPercent();
    status.textContent = percent === null ? "Connecting…" : `Checking blocks · ${percent.toFixed(1)}%${sync.server ? " · " + sync.server : ""}`;
    status.title = "Sync details";
  }
  refreshCoin();
}

/// How much of [birthday, tip) is checked, as the Mac's sync ring.
function checkedShare() {
  const b = overview && overview.balance;
  if (!b || !b.chain_tip || !b.birthday) return null;
  const span = b.chain_tip - b.birthday;
  if (span <= 0) return 1;
  const parts = b.to_scan.map(([a, e]) => [Math.max(a, b.birthday), Math.min(e, b.chain_tip)]).filter(([a, e]) => e > a).sort((x, y) => x[0] - y[0]);
  let open = 0;
  let reach = b.birthday;
  for (const [a, e] of parts) {
    const from = Math.max(a, reach);
    if (e > from) open += e - from;
    reach = Math.max(reach, e);
  }
  return Math.max(0, Math.min(1, (span - open) / span));
}

function syncPercent() {
  const share = checkedShare();
  if (share === null) return null;
  if (overview.sync.state === "up_to_date") return 100;
  return Math.min(99.9, Math.floor(share * 1000) / 10);
}

function heartbeat(height, active) {
  if (!active || height <= 0 || height === shownHeight) {
    if (height > 0 && !shownHeight) shownHeight = height;
    return;
  }
  shownHeight = height;
  const token = ++beatToken;
  beats += 1;
  const arm = $("beat-arm");
  const label = $("beat-label");
  label.textContent = `BLOCK ${numbers.format(height)}`;
  label.classList.add("on");
  arm.style.transform = `rotate(${beats * 360 - 27}deg)`;
  arm.classList.add("lit");
  setTimeout(() => { if (token === beatToken) arm.classList.remove("lit"); }, 2300);
  setTimeout(() => { if (token === beatToken) label.classList.remove("on"); }, 4500);
}

$("eye").addEventListener("click", () => { prefs.hidden = !prefs.hidden; renderHome(); });
$("balance").addEventListener("click", () => {
  if (prefs.hidden) return;
  prefs.unit = prefs.unit === "ZEC" ? "zats" : "ZEC";
  renderHome();
});

// ---------- Really open the lock? (OpenLockPrompt) ----------

let countdown = 0;
let countdownTimer = null;

function needsPassphrase() { return overview && overview.keys.some((key) => key.kind === "passphrase"); }

function askToOpen() {
  if (lockOpen || !overview) return;
  const days = heldDays();
  const held = days === null ? "" : days < 1 ? "This Zcash was locked today. " : `This Zcash has been locked ${Holding.heldFor(days)}. `;
  $("prompt-intro").textContent = held + "Nobody legitimate will ever rush you, ask for your 24 words, or tell you to move your Zcash to keep it safe.";
  $("prompt-agree").checked = false;
  $("prompt-passphrase").value = "";
  $("prompt-passphrase-field").hidden = !needsPassphrase();
  say($("prompt-message"), "");
  countdown = OPEN_WAIT;
  $("open-prompt").hidden = false;
  tickPrompt();
  clearInterval(countdownTimer);
  countdownTimer = setInterval(() => { if (countdown > 0) countdown -= 1; tickPrompt(); }, 1000);
  $("prompt-keep").focus();
}

function tickPrompt() {
  $("prompt-fill").setAttribute("stroke-dasharray", `${((OPEN_WAIT - countdown) / OPEN_WAIT) * 100} 100`);
  $("prompt-count").textContent = countdown > 0 ? String(countdown) : "";
  const ready = countdown === 0 && $("prompt-agree").checked && !busy;
  const open = $("prompt-open");
  open.textContent = busy ? "Opening…" : countdown > 0 ? `Open in ${countdown} s` : "Open the lock";
  open.disabled = !ready;
}

function keepLocked() {
  clearInterval(countdownTimer);
  $("prompt-passphrase").value = "";
  $("open-prompt").hidden = true;
}

$("prompt-agree").addEventListener("change", tickPrompt);
$("prompt-keep").addEventListener("click", keepLocked);
$("prompt-open").addEventListener("click", async () => {
  if (busy || countdown > 0 || !$("prompt-agree").checked) return;
  const passphrase = needsPassphrase() ? $("prompt-passphrase").value : null;
  $("prompt-passphrase").value = "";
  busy = true;
  tickPrompt();
  say($("prompt-message"), "Follow the prompts from Windows.", true);
  try {
    await invoke("wallet_open_lock", { passphrase });
    lockOpen = true;
    keepLocked();
    $("open-note").textContent = "";
    renderHome();
  } catch (problem) {
    say($("prompt-message"), describe(problem) || "Not confirmed, so the lock stayed closed.", !describe(problem));
  } finally {
    busy = false;
    tickPrompt();
  }
});

padlockHost.addEventListener("click", askToOpen);

async function closeLock() {
  await invoke("wallet_close_lock");
  lockOpen = false;
  if (overview) renderHome();
}

$("home-close").addEventListener("click", closeLock);

// Hidden or minimised: the open lock closes, as on the Mac.
document.addEventListener("visibilitychange", () => {
  if (document.hidden && lockOpen && !sending) closeLock();
});

// ---------- Sheets ----------

const layer = $("sheet-layer");

function openSheet(name) {
  for (const sheet of layer.querySelectorAll(".sheet")) sheet.hidden = sheet.id !== `sheet-${name}`;
  layer.hidden = false;
  if (name === "gomax") startWatching();
  else stopWatching();
}

function closeSheet() {
  layer.hidden = true;
  for (const sheet of layer.querySelectorAll(".sheet")) sheet.hidden = true;
  stopWatching();
}

for (const button of layer.querySelectorAll("[data-close]")) button.addEventListener("click", closeSheet);
document.addEventListener("keydown", (event) => {
  if (event.key !== "Escape") return;
  if (!$("open-prompt").hidden) keepLocked();
  else if (!layer.hidden && !sending) ($("sheet-send").hidden ? closeSheet() : closeSend());
});

// Receive
$("home-receive").addEventListener("click", async () => {
  openSheet("receive");
  say($("receive-message"), "", true);
  try {
    const receive = await invoke("wallet_receive");
    $("receive-address").textContent = receive.address;
    $("receive-qr").src = "data:image/svg+xml;charset=utf-8," + encodeURIComponent(receive.qr);
  } catch (problem) {
    say($("receive-message"), describe(problem));
  }
});

$("receive-copy").addEventListener("click", async () => {
  try {
    await navigator.clipboard.writeText($("receive-address").textContent);
    say($("receive-message"), "Copied.", true);
  } catch {
    say($("receive-message"), "Select the address and copy it by hand.");
  }
});

// Activity
function renderActivity() {
  const list = $("activity-list");
  list.textContent = "";
  if (activity.length === 0) {
    const empty = document.createElement("li");
    empty.className = "empty";
    empty.textContent = "Nothing yet. Payments in and out show here.";
    list.append(empty);
    return;
  }
  for (const item of activity) {
    const line = document.createElement("li");
    const what = document.createElement("span");
    what.className = "what";
    what.textContent = item.change > 0 ? "Locked" : "Released";
    const amount = document.createElement("span");
    amount.className = `amount ${item.change > 0 ? "in" : "out"}`;
    amount.textContent = prefs.hidden ? HIDDEN : `${item.change > 0 ? "+" : "−"}${exact(Math.abs(item.change))}`;
    const when = document.createElement("span");
    when.className = "when";
    when.textContent = item.expired ? "Did not go through; nothing left the lock"
      : item.height ? `${new Date((item.time || 0) * 1000).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" })} · block ${numbers.format(item.height)}`
      : "Waiting for a block";
    line.title = item.txid;
    line.append(what, amount, when);
    list.append(line);
  }
}

$("open-activity").addEventListener("click", () => { renderActivity(); openSheet("activity"); });
$("payment-line").addEventListener("click", () => {
  if (overview && overview.unsent) return openSend();
  renderActivity();
  openSheet("activity");
});

// Sync details (SyncView.swift)
function renderSync() {
  if (!overview) return;
  const b = overview.balance || {};
  const sync = overview.sync;
  const network = b.chain_tip || 0;
  const upToDate = sync.state === "up_to_date";
  const share = checkedShare();
  const percent = syncPercent();
  $("sync-sub").textContent = upToDate ? "Up to date" : sync.state === "problem" ? "Having trouble, see below" : network ? "Checking blocks" : "Connecting…";
  $("sync-percent").textContent = percent === null ? "…" : `${percent === 100 ? "100" : percent.toFixed(1)}%`;
  $("sync-ring-word").textContent = upToDate ? "up to date" : "checked";
  $("sync-ring-fill").setAttribute("stroke-dasharray", `${percent ?? 0} 100`);
  const open = share === null ? null : Math.round((1 - share) * (network - b.birthday));
  $("sync-ring-note").textContent = percent === null ? "Waiting for the network block"
    : upToDate ? "Every block since the wallet birthday"
    : open > 0 ? `Of all blocks since the wallet birthday. Catching up ${numbers.format(open)} new ${open === 1 ? "block" : "blocks"}.`
    : "Of all blocks since the wallet birthday";
  $("sync-network").textContent = network ? numbers.format(network) : "…";
  $("sync-wallet").textContent = b.scanned_height ? numbers.format(b.scanned_height) : "…";
  let behindText = "…";
  if (network && b.scanned_height) {
    const behind = upToDate ? 0 : Math.max(0, network - b.scanned_height);
    if (behind === 0) behindText = "Nothing";
    else {
      const first = heightSamples[0];
      const last = heightSamples[heightSamples.length - 1];
      const rate = heightSamples.length >= 3 && last.at > first.at ? (last.height - first.height) / ((last.at - first.at) / 1000) : 0;
      const minutes = rate > 0 ? Math.max(1, Math.round(behind / rate / 60)) : null;
      behindText = `${numbers.format(behind)} blocks${minutes ? `, about ${minutes} min` : ""}`;
    }
  }
  $("sync-behind").textContent = behindText;
  $("sync-blocktime").textContent = network === 0 ? "…" : network >= NU7_HEIGHT ? "25 seconds (since NU7)" : "75 seconds";
  $("sync-birthday").textContent = b.birthday ? `Block ${numbers.format(b.birthday)}` : "…";
  $("sync-bar").style.width = `${Math.max(share > 0 ? 1.5 : 0, (share ?? 0) * 100)}%`;
  $("sync-bar-from").textContent = b.birthday ? numbers.format(b.birthday) : "…";
  $("sync-bar-to").textContent = network ? numbers.format(network) : "…";
  const own = (sync.server || "").startsWith("your own node");
  $("sync-server").textContent = sync.server || "…";
  $("sync-connection").textContent = own ? "Direct, to your own node" : "Encrypted (TLS)";
  const problems = $("sync-problems");
  problems.textContent = "";
  if (!overview.problems.length) {
    const none = document.createElement("p");
    none.className = "quiet";
    none.classList.add("none-yet");
    none.textContent = "None since Lock it opened.";
    problems.append(none);
  } else {
    for (const p of overview.problems) {
      const row = document.createElement("div");
      row.className = "problem";
      const time = document.createElement("time");
      time.textContent = new Date(p.at * 1000).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });
      const text = document.createElement("span");
      text.textContent = p.message;
      row.append(time, text);
      problems.append(row);
    }
  }
}

$("status-text").addEventListener("click", () => {
  if (overview && overview.sync.state === "problem") invoke("wallet_sync_now");
  renderSync();
  openSheet("sync");
});

// ---------- Send (SendView.swift) ----------

let sendStage = "compose"; // compose, check, sending, done, unsent
let sending = false;

function sendLayout() {
  const stage = sendStage;
  for (const input of $("send-form").querySelectorAll("input")) input.disabled = stage !== "compose";
  $("send-form").hidden = stage === "sending" || stage === "done" || stage === "unsent";
  $("send-receipt").hidden = stage !== "check";
  $("send-progress").hidden = stage !== "sending";
  const titles = { compose: "Send Zcash", check: "Check before sending", sending: "Sending", done: "Sent", unsent: "Not delivered yet" };
  const subs = {
    compose: "Taken out of the lock. Everything else stays locked.",
    check: "Nothing has left the lock yet.",
    sending: $("send-tor").checked ? "Signing on this PC and sending it over Tor. This can take a minute." : "Signing on this PC and handing it to the network.",
    done: "The lock closes again.",
    unsent: "Made, but not delivered yet.",
  };
  $("send-title").textContent = titles[stage];
  $("send-sub").textContent = subs[stage];
  $("send-close").hidden = stage === "sending";
  $("send-close").textContent = stage === "done" ? "Done" : "Cancel";
  $("send-buttons").hidden = stage === "sending";
  $("send-back").hidden = stage !== "check";
  const next = $("send-next");
  next.textContent = { compose: "Check before sending", check: "Send", done: "Done", unsent: "Try again over Tor" }[stage] || "";
  if (stage === "check" && overview) next.textContent = prefs.hidden ? "Send" : `Send ${amountText(plan.amount, "ZEC")}`;
  next.hidden = stage === "sending";
  $("send-plain")?.remove();
  if (stage === "unsent") offerWithoutTor();
}

let plan = null;

function openSend() {
  if (!lockOpen && !(overview && overview.unsent)) return askToOpen();
  $("send-form").reset();
  $("send-tor").checked = true;
  plan = null;
  sendStage = overview && overview.unsent ? "unsent" : "compose";
  say($("send-message"), sendStage === "unsent" ? "A signed payment did not reach a server. It is the same payment each time, so nothing is paid twice." : "");
  sendLayout();
  openSheet("send");
  if (sendStage === "compose") $("send-to").focus();
}

function closeSend() {
  if (sending) return;
  closeSheet();
  if (sendStage === "done") closeLock();
}

$("home-send").addEventListener("click", openSend);
$("send-close").addEventListener("click", closeSend);
$("send-back").addEventListener("click", () => { sendStage = "compose"; say($("send-message"), ""); sendLayout(); });

function offerWithoutTor() {
  const plain = document.createElement("button");
  plain.type = "button";
  plain.className = "quiet-button";
  plain.id = "send-plain";
  plain.textContent = "Send without Tor (the server sees your internet address)";
  plain.addEventListener("click", () => sendAgain(false));
  $("send-buttons").after(plain);
}

if (listen) {
  listen("send-step", (event) => {
    $("send-step").textContent = event.payload;
    $("send-step-note").textContent = event.payload === "Connecting over Tor"
      ? "The first time, Tor needs a minute or two to find its way. Keep Lock it open."
      : "This can take a minute. Keep Lock it open.";
  });
}

function sent(result) {
  sendStage = "done";
  sendLayout();
  const via = result.accepted_by.length ? ` Accepted by ${result.accepted_by.join(" and ")}.` : "";
  say($("send-message"), `On its way.${via} It shows as confirmed once it is in a block. The rest stays locked.`, true);
  refreshHome();
}

function notSent(problem) {
  if (problem && problem.unsent) {
    sendStage = "unsent";
    sendLayout();
    say($("send-message"), `${describe(problem)}. It is the same payment each time, so do not pay again.`);
  } else {
    say($("send-message"), describe(problem) || "Nothing was sent.");
  }
}

async function sendAgain(overTor) {
  if (sending) return;
  sending = true;
  sendStage = "sending";
  $("send-step").textContent = overTor ? "Connecting over Tor" : "Handing it to the network";
  sendLayout();
  try {
    sent(await invoke("wallet_send_again", { overTor }));
  } catch (problem) {
    notSent(problem);
  } finally {
    sending = false;
  }
}

$("send-next").addEventListener("click", async () => {
  if (sending) return;
  if (sendStage === "done") return closeSend();
  if (sendStage === "unsent") return sendAgain(true);
  if (sendStage === "compose") {
    say($("send-message"), "Working it out…", true);
    try {
      plan = await invoke("wallet_prepare", { to: $("send-to").value, amount: $("send-amount").value, memo: $("send-memo").value || null });
      $("plan-amount").textContent = exact(plan.amount);
      $("plan-fee").textContent = exact(plan.fee);
      $("plan-total").textContent = exact(plan.total);
      $("plan-to").textContent = plan.to;
      sendStage = "check";
      say($("send-message"), "");
      sendLayout();
    } catch (problem) {
      say($("send-message"), describe(problem));
    }
    return;
  }
  if (sendStage === "check") {
    sending = true;
    const overTor = $("send-tor").checked;
    sendStage = "sending";
    $("send-step").textContent = "Signing on this PC";
    say($("send-message"), "");
    sendLayout();
    try {
      sent(await invoke("wallet_send", { overTor }));
    } catch (problem) {
      if (!(problem && problem.unsent)) {
        sendStage = "check";
        sendLayout();
      }
      notSent(problem);
    } finally {
      sending = false;
    }
  }
});

// ---------- Go Max ----------

const nodeRing = $("node-ring");
const ringProgress = $("ring-progress");
const nodePercent = $("node-percent");
const nodeState = $("node-state");
const nodeLead = $("node-lead");
const nodeFacts = $("node-facts");
const nodeMessage = $("node-message");
const nodeToggle = $("node-toggle");
const LEAD_OFF = "Your own Zcash node, on this PC. About 20 GB, and a few hours to catch up the first time.";

let watchTimer = null;
let lastView = null;

function gigabytes(bytes) { return bytes > 0 ? `${(bytes / 1e9).toFixed(1)} GB` : "Nothing yet"; }

function setRing(state, fraction) {
  nodeRing.dataset.state = state;
  ringProgress.setAttribute("stroke-dasharray", `${Math.max(0, Math.min(100, fraction * 100))} 100`);
}

function renderGoMax(view) {
  lastView = view;
  nodeMessage.textContent = "";
  nodeToggle.disabled = false;
  if (view.state === "off") {
    setRing("off", 0);
    nodePercent.textContent = "";
    nodeState.textContent = "Off";
    nodeLead.textContent = LEAD_OFF;
    nodeFacts.hidden = true;
    nodeToggle.textContent = "Turn on Go Max";
    nodeToggle.disabled = !view.installed;
    if (!view.installed) say(nodeMessage, "This build has no node programs inside.", true);
    else if (view.problem) say(nodeMessage, view.problem);
    return;
  }
  if (view.state === "starting" || view.state === "stopping") {
    setRing(view.state, 1);
    nodePercent.textContent = "";
    nodeState.textContent = view.state === "starting" ? "Starting" : "Stopping";
    nodeLead.textContent = view.state === "starting" ? `${view.step}.` : "Letting the node finish its work.";
    nodeFacts.hidden = true;
    nodeToggle.textContent = view.state === "starting" ? "Starting…" : "Stopping…";
    nodeToggle.disabled = true;
    return;
  }
  const status = view.status;
  nodeToggle.textContent = "Turn off Go Max";
  nodeFacts.hidden = !status;
  if (!status) {
    setRing("on", 0);
    nodePercent.textContent = "";
    nodeState.textContent = "On";
    nodeLead.textContent = "The node is not answering right now.";
    return;
  }
  const caughtUp = status.estimated_height > 0 && status.height + 2 >= status.estimated_height;
  const fraction = status.estimated_height > 0 ? status.height / status.estimated_height : status.progress;
  setRing("on", caughtUp ? 1 : fraction);
  nodePercent.textContent = caughtUp ? "" : `${Math.floor(fraction * 100)}%`;
  if (status.peers === 0 && status.height === 0) {
    nodeState.textContent = "Waiting";
    nodeLead.textContent = "Looking for other Zcash nodes. Check that this PC is online.";
  } else if (caughtUp) {
    nodeState.textContent = "Yours";
    nodeLead.textContent = "Caught up. Lock it reads from your own node.";
  } else {
    nodeState.textContent = "Catching up";
    nodeLead.textContent = "Checking every block since the beginning. You can keep using this PC.";
  }
  const network = status.estimated_height > 0 ? ` of ${numbers.format(status.estimated_height)}` : "";
  $("fact-height").textContent = `${numbers.format(status.height)}${network}`;
  $("fact-peers").textContent = numbers.format(status.peers);
  $("fact-wallet").textContent = view.wallet_ready ? "Ready" : view.wallet_note ? "Waiting for the node" : "Waiting for blocks";
  $("fact-wallet").title = view.wallet_note || "";
  $("fact-disk").textContent = gigabytes(status.size_on_disk);
  const ONION_WORDS = { starting: "Connecting to Tor", reachable: "Ready", degraded: "Ready, partly", unreachable: "Not reachable right now" };
  if (view.onion) {
    $("fact-onion").textContent = ONION_WORDS[view.onion.state] || view.onion.state;
    $("fact-onion-address").textContent = view.onion.address;
  } else {
    $("fact-onion").textContent = view.onion_problem ? "Not available" : "Off";
    $("fact-onion-address").textContent = view.onion_problem || "";
  }
}

async function refreshGoMax() {
  try {
    renderGoMax(await invoke("go_max_view"));
  } catch (problem) {
    nodeMessage.textContent = describe(problem);
  }
}

function startWatching() {
  if (watchTimer) return;
  refreshGoMax();
  watchTimer = setInterval(refreshGoMax, 3000);
}

function stopWatching() {
  clearInterval(watchTimer);
  watchTimer = null;
}

async function refreshCoin() {
  try {
    const view = await invoke("go_max_view");
    const dot = $("coin-dot");
    dot.className = "coin-dot";
    if (view.state === "on") dot.classList.add(view.status && view.status.estimated_height > 0 && view.status.height + 2 >= view.status.estimated_height ? "on" : "busy");
    else if (view.state !== "off") dot.classList.add("busy");
  } catch { /* the coin just stays grey */ }
}

if (listen) listen("go-max", () => { if (!$("sheet-gomax").hidden) refreshGoMax(); });

nodeToggle.addEventListener("click", async () => {
  nodeToggle.disabled = true;
  const command = lastView && lastView.state === "on" ? "go_max_stop" : "go_max_start";
  try {
    await invoke(command);
  } catch (problem) {
    say(nodeMessage, describe(problem));
  }
  refreshGoMax();
});

$("go-max-coin").addEventListener("click", () => openSheet("gomax"));
$("open-gomax").addEventListener("click", () => openSheet("gomax"));

// ---------- Start ----------

async function start() {
  const first = await invoke("wallet_overview");
  showView(first.exists ? "home" : "welcome");
}

start().catch((problem) => {
  showView("loading");
  views.loading.querySelector("p").textContent = describe(problem) || "The app could not start.";
});
