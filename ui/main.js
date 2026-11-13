// Zecky Lock it: the window. It draws and asks; it never holds a key.
// The 24 words pass through here once, to be written down (or typed in for a
// restore), and a passphrase on its way to the Rust side. Fields are cleared
// as soon as they are sent. The lock's words stay on the Rust side.

"use strict";

const invoke = window.__TAURI__.core.invoke;
const listen = window.__TAURI__.event ? window.__TAURI__.event.listen : null;

const KEY_NAMES = { device_hello: "This PC", security_key: "Security key", passphrase: "Passphrase" };
const MIN_PASSPHRASE = 12;
/// "Really open the lock?" waits this long by default, as Lock it Mac.
let OPEN_WAIT = 10;
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
  if (!$("sheet-holding").hidden) renderHolding();
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
  $("held-line").textContent = b.total > 0 ? (days === null ? "Locked on this PC" : `Locked ${Holding.heldFor(days)} ›`) : "";
  $("held-line").disabled = days === null;
  $("held-line").title = days === null ? "" : "Milestones and highlights";
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

async function openWithoutPause() {
  try {
    const passphrase = needsPassphrase() ? await askKeys("Open the lock", "Your keys open the lock.", { action: "Open the lock" }) : null;
    await invoke("wallet_open_lock", { passphrase });
    lockOpen = true;
    renderHome();
  } catch (problem) {
    if (problem && problem.message !== "cancelled") $("open-note").textContent = describe(problem) || "Not confirmed, so the lock stayed closed.";
  }
}

function askToOpen() {
  if (lockOpen || !overview) return;
  OPEN_WAIT = overview.settings ? overview.settings.wait : 10;
  if (OPEN_WAIT === 0) return openWithoutPause();
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
  if (!$("confirm").hidden) $("confirm-cancel").click();
  else if (!$("open-prompt").hidden) keepLocked();
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
  $("send-tor").checked = !overview || !overview.settings || overview.settings.tor;
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

// ---------- Confirm it's you (before the words, the viewing key, erasing) ----------

/// Shows the dialog; resolves with the passphrase (or null when the lock has
/// no passphrase key) on Continue, rejects on Cancel. The keys themselves
/// (Windows Hello, security key) are asked by Windows right after.
function askKeys(title, intro, { danger = false, action = "Continue" } = {}) {
  return new Promise((resolve, reject) => {
    $("confirm-title").textContent = title;
    $("confirm-intro").textContent = intro;
    $("confirm-passphrase").value = "";
    $("confirm-passphrase-field").hidden = !needsPassphrase();
    say($("confirm-message"), "");
    const go = $("confirm-go");
    go.textContent = action;
    go.classList.toggle("danger", danger);
    $("confirm").hidden = false;
    (needsPassphrase() ? $("confirm-passphrase") : $("confirm-cancel")).focus();
    const finish = (ok) => {
      $("confirm").hidden = true;
      go.onclick = null;
      $("confirm-cancel").onclick = null;
      const passphrase = needsPassphrase() ? $("confirm-passphrase").value : null;
      $("confirm-passphrase").value = "";
      if (ok) resolve(passphrase);
      else reject(new Error("cancelled"));
    };
    go.onclick = () => finish(true);
    $("confirm-cancel").onclick = () => finish(false);
  });
}

// ---------- Holding: "Locked for ..." (HoldingSheet) ----------

function confirmedEntries() {
  return activity.filter((a) => a.height && !a.expired).map((a) => ({ height: a.height, time: a.time || 0, change: a.change }));
}

function shortDate(seconds) {
  return new Date(seconds * 1000).toLocaleDateString("en-US", { month: "short", day: "numeric", year: "numeric" });
}

let holdingAll = false;

function renderHolding() {
  const h = Holding.summary(confirmedEntries());
  const amount = (z) => (prefs.hidden ? "••••" : amountText(z));
  $("holding-title").textContent = h ? `Locked ${Holding.heldFor(h.days)}` : "Nothing locked";
  $("holding-sub").textContent = h ? `Since ${shortDate(h.since)}` : "";
  $("holding-note").textContent = h
    ? "Worked out on this PC from your own payments. Sending or receiving more never resets it; only an empty lock does."
    : "The count starts when Zcash comes into the lock.";
  for (const id of ["holding-milestones", "holding-highlights"]) $(id).hidden = !h;
  document.querySelectorAll("#sheet-holding .section-title, #sheet-holding .holding-ring").forEach((el) => { el.hidden = !h; });
  if (!h) return;

  Lockart.milestone($("holding-ring"), Holding.progress(h.days), Holding.reachedToday(h.days));
  const words = Holding.heldFor(h.days).replace("for ", "").split(" ");
  $("holding-number").textContent = h.days < 1 ? "" : words[0];
  $("holding-unit").textContent = h.days < 1 ? "since today" : words.slice(1).join(" ");
  const moment = Holding.reachedToday(h.days) ? h.days : null;
  $("holding-moment").hidden = moment === null;
  if (moment !== null) $("holding-moment").textContent = `${Holding.name(moment)}. ${Holding.line(moment)}`;

  const full = Holding.path(h.since, h.days);
  const path = holdingAll ? full : Holding.shortPath(full);
  const box = $("holding-milestones");
  box.textContent = "";
  path.forEach((step, i) => {
    const next = step.progress !== null;
    const row = document.createElement("div");
    row.className = `step${!step.reached && !next ? " dim" : ""}${moment === step.days ? " moment-step" : ""}`;
    const rail = document.createElement("div");
    rail.className = "step-rail";
    const dot = document.createElement("span");
    dot.className = `step-dot${step.reached ? " reached" : next ? " next" : ""}`;
    rail.append(dot);
    if (i < path.length - 1) {
      const line = document.createElement("span");
      line.className = `step-line${step.reached && path[i + 1].reached ? " gold" : ""}`;
      rail.append(line);
    }
    const text = document.createElement("div");
    text.className = "step-text";
    const name = document.createElement("span");
    name.className = "step-name";
    name.textContent = step.name;
    const when = document.createElement("span");
    when.className = "step-when";
    when.textContent = step.reached
      ? `${step.length} · ${step.at.toLocaleDateString("en-US", { month: "short", day: "numeric", year: "numeric" })}`
      : next ? `${step.length} · in ${step.inDays} ${step.inDays === 1 ? "day" : "days"}` : step.length;
    text.append(name, when);
    if (next) {
      const bar = document.createElement("div");
      bar.className = "step-bar";
      const fill = document.createElement("div");
      fill.style.width = `${Math.max(3, step.progress * 100)}%`;
      bar.append(fill);
      text.append(bar);
    }
    row.append(rail, text);
    box.append(row);
  });
  const more = document.createElement("span");
  more.className = "show-all";
  more.textContent = holdingAll ? "Show less" : "Show all milestones ›";
  box.append(more);

  const lines = [["Since", shortDate(h.since)], ["Received since then", amount(h.current.received)]];
  const pace = Holding.pace(h.current.received, h.days);
  if (pace) lines.push(["Stacking pace", `+${amount(pace.zats)} a ${pace.per}`]);
  lines.push(["Payments in", String(h.current.paymentsIn)]);
  if (h.current.largest) lines.push(["Largest payment in", `${amount(h.current.largest.zats)} · ${shortDate(h.current.largest.at)}`]);
  if (h.longestBefore) lines.push(["Longest before", Holding.heldFor(h.longestBefore.days).replace("for ", "")]);
  const card = $("holding-highlights");
  card.textContent = "";
  for (const [label, value] of lines) card.append(cardLine(label, value));
}

function cardLine(label, value) {
  const line = document.createElement("div");
  line.className = "line";
  const l = document.createElement("span");
  l.textContent = label;
  const v = document.createElement("strong");
  v.textContent = value;
  line.append(l, v);
  return line;
}

$("holding-milestones").addEventListener("click", () => { holdingAll = !holdingAll; renderHolding(); });
$("held-line").addEventListener("click", () => {
  if (heldDays() === null) return;
  holdingAll = false;
  renderHolding();
  openSheet("holding");
});

// ---------- Settings (SettingsView) ----------

const PRESETS = [["Default", "zec.rocks"], ["Europe", "eu.zec.rocks"], ["North America", "na.zec.rocks"], ["South America", "sa.zec.rocks"], ["Asia Pacific", "ap.zec.rocks"]];
const PAGE_TEXT = {
  overview: ["Settings", null],
  network: ["Network", "Where Lock it gets the Zcash chain from."],
  display: ["Display", "How Lock it looks on this PC."],
  security: ["Security", "The keys, your words, erasing."],
  keys: ["Keys to the lock", "What has to be passed before the lock opens."],
  phrase: ["Recovery phrase", "Your 24 words, sealed on this PC."],
  viewkey: ["Viewing key", "Read-only access to this wallet."],
  about: ["About", "Zecky Lock it for Windows."],
  how: ["How Zecky works", "Short chapters. Open the one you want."],
};
const PARENT = { keys: "security", phrase: "security", viewkey: "security" };

let settingsPage = "overview";
let settingsView = null;
let serverChoice = 0;
let phraseTimer = null;

async function loadSettings() {
  try { settingsView = await invoke("settings_get"); } catch { settingsView = null; }
}

function currentSettings() {
  return (settingsView && settingsView.settings) || (overview && overview.settings) || { server: null, tor: true, wait: 10 };
}

function goSettings(page) {
  if (page === "gomax") return openSheet("gomax");
  settingsPage = page;
  hidePhrase();
  hideViewKey();
  for (const el of document.querySelectorAll("#sheet-settings .page")) el.hidden = el.dataset.page !== page;
  const [title, sub] = PAGE_TEXT[page];
  $("settings-title").textContent = title;
  $("settings-sub").textContent = sub || "";
  $("settings-sub").hidden = !sub;
  $("settings-close").textContent = page === "overview" ? "Done" : "Back";
  renderSettings();
  document.querySelector("#sheet-settings .sheet-scroll").scrollTop = 0;
}

function renderSettings() {
  const s = currentSettings();
  const b = (overview && overview.balance) || {};
  // Overview
  $("net-block").textContent = b.chain_tip ? `Block ${numbers.format(b.chain_tip)}` : "Catching up…";
  $("net-pace").textContent = b.chain_tip ? (b.chain_tip >= NU7_HEIGHT ? "A block every 25 seconds, since NU7" : "A block every 75 seconds") : "";
  $("row-network").textContent = s.server ? `${s.server[0]}${s.server[1] === 443 ? "" : ":" + s.server[1]}` : "zec.rocks";
  $("row-display").textContent = `${prefs.unit}${prefs.hidden ? " · hidden" : ""}`;
  const keys = overview ? overview.keys.map((k) => KEY_NAMES[k.kind]) : [];
  $("row-security").textContent = keys.join(" + ");
  $("row-keys").textContent = `${keys.length} ${keys.length === 1 ? "key" : "keys"}${s.wait ? ` · wait ${s.wait} s` : ""}`;
  $("row-about").textContent = settingsView ? `Version ${settingsView.version}` : "";
  // Network
  serverChoice = !s.server ? 0 : s.server[1] === 443 ? PRESETS.findIndex(([, h]) => h === s.server[0]) : -1;
  if (s.server && serverChoice === -1 && !$("server-own").value) $("server-own").value = `${s.server[0]}:${s.server[1]}`;
  renderServerChips();
  $("settings-tor").checked = s.tor;
  const tor = $("tor-note");
  tor.textContent = s.tor
    ? "Your payments leave over Tor, each on its own connection, to a public server that cannot see this PC's internet address. If Tor cannot connect, the payment waits; it is never sent another way by itself. Reading the chain is not affected."
    : "Your payments go to the server above, which sees this PC's internet address. With your own node, other Zcash nodes see the payment coming from your internet address.";
  tor.className = `small ${s.tor ? "quiet" : "gold"}`;
  // Display
  for (const chip of document.querySelectorAll("[data-unit]")) chip.classList.toggle("on", chip.dataset.unit === prefs.unit);
  $("settings-hide").checked = prefs.hidden;
  // Keys
  const list = $("keys-list");
  list.textContent = "";
  const all = overview ? overview.keys : [];
  all.forEach((key, i) => {
    const row = document.createElement("div");
    row.className = "row key-row";
    const n = document.createElement("span");
    n.className = "key-num";
    n.textContent = String(i + 1);
    const title = document.createElement("span");
    title.className = "row-title";
    title.textContent = { device_hello: "This PC (Windows Hello)", security_key: "Security key", passphrase: "Passphrase" }[key.kind];
    const state = document.createElement("span");
    state.className = "row-detail on";
    state.textContent = { device_hello: "Face, fingerprint or PIN", security_key: "Touch the key", passphrase: "Asked when opening" }[key.kind];
    row.append(n, title, state);
    list.append(row);
  });
  for (const chip of document.querySelectorAll("[data-wait]")) chip.classList.toggle("on", Number(chip.dataset.wait) === s.wait);
  // About
  $("about-version").textContent = settingsView ? settingsView.version : "…";
  $("about-name").textContent = `Lock it for Windows ${settingsView ? settingsView.version : ""}`;
  $("about-block").textContent = b.chain_tip ? numbers.format(b.chain_tip) : "…";
  $("about-birthday").textContent = b.birthday ? numbers.format(b.birthday) : "…";
  $("about-server").textContent = (overview && overview.sync.server) || "…";
}

function renderServerChips() {
  const chips = $("server-chips");
  chips.textContent = "";
  [...PRESETS.map(([label], i) => [i, label]), [-1, "Your own"]].forEach(([i, label]) => {
    const chip = document.createElement("button");
    chip.type = "button";
    chip.className = `chip${i === serverChoice ? " on" : ""}`;
    chip.textContent = label;
    chip.addEventListener("click", () => { serverChoice = i; say($("server-note"), ""); renderServerChips(); });
    chips.append(chip);
  });
  $("server-shown").textContent = serverChoice >= 0 ? `${PRESETS[serverChoice][1]}:443` : "Your own node (Rock it) or server: host:port";
  $("server-own").hidden = serverChoice !== -1;
  const own = serverChoice === -1 ? $("server-own").value.trim().split(":")[0] : "";
  $("server-why").textContent = serverChoice === -1 && /^(10\.|192\.168\.|172\.(1[6-9]|2\d|3[01])\.|127\.|localhost)/.test(own)
    ? "Your own node on your home network, without TLS: the traffic stays inside your home. Lock it cannot check that this address is really your node. On another Wi-Fi, a stranger's device with the same numbers could answer: it cannot take anything, but it could hide or delay payments. Away from home, use your node's Tailscale address."
    : serverChoice === -1 && /^100\.(6[4-9]|[7-9]\d|1[01]\d|12[0-7])\./.test(own)
      ? "Your own node through Tailscale: the connection is encrypted by Tailscale and only reaches your tailnet. Lock it cannot prove it is your node, so only use the address your node shows you."
      : "Encrypted (TLS). The server sees this PC's internet address, never your words or balance. With Go Max up to date, Lock it reads from your own node instead.";
}

$("server-own").addEventListener("input", renderServerChips);
$("server-why-toggle").addEventListener("click", () => {
  const why = $("server-why");
  why.hidden = !why.hidden;
  $("server-why-toggle").textContent = why.hidden ? "What the server sees" : "Less";
});

$("server-use").addEventListener("click", async () => {
  let host = null;
  let port = null;
  if (serverChoice > 0) { host = PRESETS[serverChoice][1]; port = 443; }
  if (serverChoice === -1) {
    const text = $("server-own").value.trim();
    const m = text.match(/^\[?([^\]]+?)\]?(?::(\d{1,5}))?$/);
    if (!m) return say($("server-note"), "Type host:port, for example 192.168.1.20:9067.");
    host = m[1];
    port = m[2] ? Number(m[2]) : 443;
  }
  say($("server-note"), "Checking that it answers…", true);
  $("server-use").disabled = true;
  try {
    const note = await invoke("settings_set_server", { host, port });
    say($("server-note"), note, true);
    await loadSettings();
    renderSettings();
  } catch (problem) {
    say($("server-note"), describe(problem));
  } finally {
    $("server-use").disabled = false;
  }
});

$("settings-tor").addEventListener("change", async (event) => {
  try {
    await invoke("settings_set_tor", { on: event.target.checked });
    await loadSettings();
  } catch (problem) {
    event.target.checked = !event.target.checked;
  }
  renderSettings();
});

for (const chip of document.querySelectorAll("[data-unit]")) {
  chip.addEventListener("click", () => { prefs.unit = chip.dataset.unit; renderSettings(); if (overview) renderHome(); });
}
$("settings-hide").addEventListener("change", (event) => { prefs.hidden = event.target.checked; renderSettings(); if (overview) renderHome(); });

for (const chip of document.querySelectorAll("[data-wait]")) {
  chip.addEventListener("click", async () => {
    const seconds = Number(chip.dataset.wait);
    const now = currentSettings().wait;
    if (seconds === now) return;
    const weaker = seconds === 0 || (now !== 0 && seconds < now);
    say($("wait-note"), "");
    try {
      const passphrase = weaker ? await askKeys("Confirm it's you", "Making the pause shorter weakens a key, so your keys are asked first.") : null;
      await invoke("settings_set_wait", { seconds, passphrase });
      await loadSettings();
      if (overview) overview.settings = currentSettings();
      renderSettings();
    } catch (problem) {
      if (problem && problem.message !== "cancelled") say($("wait-note"), describe(problem) || "Not confirmed, so nothing changed.");
    }
  });
}

// Recovery phrase: shown after the keys, hidden after 2 minutes or when
// Lock it is not in front.
function hidePhrase() {
  clearTimeout(phraseTimer);
  $("phrase-words").textContent = "";
  $("phrase-shown").hidden = true;
  $("phrase-toggle").textContent = "Show recovery phrase";
}

$("phrase-toggle").addEventListener("click", async () => {
  if (!$("phrase-shown").hidden) return hidePhrase();
  say($("phrase-note"), "");
  try {
    const passphrase = await askKeys("Confirm it's you", "Your 24 words are the wallet. Make sure nobody can see your screen.", { action: "Show the words" });
    const words = await invoke("wallet_show_words", { passphrase });
    const birthday = words.pop();
    const list = $("phrase-words");
    list.textContent = "";
    for (const word of words) {
      const item = document.createElement("li");
      item.textContent = word;
      list.append(item);
    }
    $("phrase-birthday").textContent = Number(birthday) ? `Birthday block ${numbers.format(Number(birthday))}: with the words, it tells any Zcash wallet where to start looking.` : "";
    $("phrase-shown").hidden = false;
    $("phrase-toggle").textContent = "Hide";
    clearTimeout(phraseTimer);
    phraseTimer = setTimeout(hidePhrase, 120000);
  } catch (problem) {
    if (problem && problem.message !== "cancelled") say($("phrase-note"), describe(problem) || "Not confirmed, so nothing is shown.");
  }
});

function hideViewKey() {
  $("viewkey-text").textContent = "";
  $("viewkey-text").hidden = true;
  $("viewkey-toggle").textContent = "Show viewing key";
}

$("viewkey-toggle").addEventListener("click", async () => {
  if (!$("viewkey-text").hidden) return hideViewKey();
  say($("viewkey-note"), "");
  try {
    const passphrase = await askKeys("Confirm it's you", "Whoever has the viewing key sees every payment, past and future.", { action: "Show the key" });
    $("viewkey-text").textContent = await invoke("wallet_viewing_key", { passphrase });
    $("viewkey-text").hidden = false;
    $("viewkey-toggle").textContent = "Hide";
  } catch (problem) {
    if (problem && problem.message !== "cancelled") say($("viewkey-note"), describe(problem) || "This wallet's viewing key could not be read.");
  }
});

window.addEventListener("blur", () => { hidePhrase(); hideViewKey(); });
document.addEventListener("visibilitychange", () => { if (document.hidden) { hidePhrase(); hideViewKey(); } });

$("erase").addEventListener("click", async () => {
  try {
    await askKeys("Erase everything on this PC?",
      "The wallet's 24 words, the history and settings are removed from this PC. Without the 24 words on paper, the Zcash in this wallet cannot be recovered. Go Max's node data stays.",
      { danger: true, action: "Erase everything" });
  } catch {
    return;
  }
  try {
    const passphrase = await askKeys("Confirm it's you", "Erasing asks your keys, whatever you chose.", { danger: true, action: "Erase" });
    await invoke("wallet_erase", { passphrase });
    closeSheet();
    overview = null;
    activity = [];
    lockOpen = false;
    await start();
  } catch (problem) {
    if (problem && problem.message !== "cancelled") alert(describe(problem) || "Not confirmed, so nothing was erased.");
  }
});

$("settings-close").addEventListener("click", () => {
  if (settingsPage === "overview") return closeSheet();
  goSettings(PARENT[settingsPage] || "overview");
});
for (const el of document.querySelectorAll("#sheet-settings [data-go]")) el.addEventListener("click", () => goSettings(el.dataset.go));

$("open-settings").addEventListener("click", async () => {
  await loadSettings();
  openSheet("settings");
  goSettings("overview");
  refreshGoMaxRow();
});

async function refreshGoMaxRow() {
  try {
    const v = await invoke("go_max_view");
    $("row-gomax").textContent = v.state === "off" ? "Off" : v.state !== "on" ? (v.state === "starting" ? "Starting" : "Stopping")
      : v.status && v.status.estimated_height > 0 && v.status.height + 2 >= v.status.estimated_height ? "Yours, up to date"
      : v.status && v.status.estimated_height > 0 ? `Catching up · ${Math.floor((v.status.height / v.status.estimated_height) * 100)}%` : "On";
  } catch { $("row-gomax").textContent = ""; }
}

// How Zecky works: the Mac's chapters, as they are on Windows.
const CHAPTERS = [
  ["The lock", "A place to keep Zcash, not to spend it.", [
    "Receiving is always one click. Opening the lock to send takes a deliberate moment: the pause, your keys, and then Send.",
    "The lock closes again after a payment, or when you minimise Lock it.",
    "The line under your balance says how long your Zcash has been in the lock: “Locked for 3 weeks”. It is never a date, and nothing unlocks on a date.",
    "The ring around the lock fills towards your next milestone: 1 week, then every month of the first year, then every year. On the day you reach one, the ring glows softly.",
    "Click the line for highlights and milestones, each with its own name, from Locked at 1 week to Legend at 4 years.",
  ], [
    "The count starts at your first payment in after the lock was last empty. Sending, of any size, and receiving more do not reset it; only an empty lock does.",
    "It is worked out on this PC from your own payment history, with no timer stored. Without the history from before your first payment in (after a restore), the line says “Locked on this PC”.",
    "1 week, 1 year and every year after also engrave a fine line into the medallion. With every new Zcash block a light runs once around it.",
    "Milestones bring no notifications, badges or pop-ups.",
  ]],
  ["Receive and send", "Waiting times, fees and what can't be undone.", [
    "Receive gives you a shielded address: amount, sender and receiver are encrypted on the blockchain. It is safe to share; only your 24 words must stay secret.",
    "Nothing leaves without a receipt first: who receives what, the network fee and what leaves the lock. A payment cannot be reversed once sent.",
  ], [
    "Zcash you receive can be used after 10 confirmations, your own change after 3. A block comes about every 25 seconds since the NU7 upgrade.",
    "A payment that could not be delivered is kept, and Try again sends that same payment. Do not pay again meanwhile. If it is never delivered, it expires after about 40 blocks and nothing leaves the lock.",
    "Sending to a transparent address is public: its amount and destination. A private note only goes to a shielded address.",
  ]],
  ["Your 24 words", "They are your Zcash. Keep them on paper.", [
    "Your 24 words and birthday block restore the wallet in Lock it or any Zcash wallet. Whoever has them has your Zcash.",
    "On this PC they are sealed by the keys you chose (Windows Hello, a passphrase, a security key) and shown only when you ask, for at most 2 minutes.",
    "A viewing key shows everything in a wallet but can't spend. Showing it asks your keys first, and sharing it can't be undone.",
  ], [
    "They are made on this PC from the system's secure random generator (256 bits). After you write them down, Lock it asks for three of them back.",
    "They are opened only to sign a payment or to show them to you, and let go straight after the lock closes. They are never written to a log or sent anywhere.",
    "To remove everything, use Settings › Security › Erase everything.",
  ]],
  ["Keys to the lock", "Checks before Zcash leaves. You choose.", [
    "The keys are chosen when the wallet is made: This PC (Windows Hello), a passphrase, and a security key. Every key you chose is needed.",
    "Really open the lock? adds a pause with a reminder. Showing your 24 words, the viewing key and erasing always ask your keys.",
  ], [
    "Windows Hello keeps its key in this PC's security chip (TPM) and asks for your face, fingerprint or PIN.",
    "A security key, such as a YubiKey, signs a fresh challenge each time and Lock it checks the signature itself; nothing goes over the internet. It always comes together with Windows Hello or a passphrase.",
  ]],
  ["Private by default", "Amounts, senders and addresses stay hidden.", [
    "Lock it only hands out shielded addresses, so amounts, senders and receivers stay private on the blockchain.",
  ], []],
  ["Go Max", "Your own Zcash node, inside Lock it.", [
    "Go Max runs a Zcash node in Lock it on this PC. It checks the blocks itself and keeps only recent blocks in full. When it is up to date, Lock it reads the chain from it instead of a server.",
    "It needs about 35 GB free to start. The first check takes most of a day and downloads the whole chain once, deleting older blocks as it goes.",
  ], [
    "Until the node is up to date, Lock it keeps using your server. When a wallet still has older blocks to check, Lock it reads those from your server.",
    "The node connects to other Zcash nodes on the internet, like every node: they see this PC's internet address, never your wallet, addresses or balance. The node accepts no connections from outside.",
    "It runs while Lock it is open. Its files live in %LOCALAPPDATA%\\app.zecky.lockit\\go-max.",
  ]],
  ["What others can see", "The server.", [
    "Lock it talks to one Zcash server (zec.rocks by default) over an encrypted connection. The server sees this PC's internet address, never your words or your balance.",
  ], [
    "Payments you send leave over Tor, each on its own connection, to a public Zcash server: it sees a payment arriving from Tor, not this PC's internet address. If Tor cannot connect, the payment waits and you are told. You can switch this off in Settings › Network.",
    "With Go Max up to date, Lock it uses no server at all for reading: it reads the chain from its own node on this PC.",
  ]],
  ["On this PC", "Where your data lives.", [
    "Your history (amounts and dates, never the words) is kept in %LOCALAPPDATA%\\app.zecky.lockit. Keep BitLocker or device encryption on so the whole disk is encrypted as well.",
  ], [
    "Keys, proofs, signatures and payments are made by librustzcash, the Zcash developers' own core (NU7 releases), in the same versions as the Zecky engine.",
    "The Sapling parameter files (about 50 MB) come with the app, checked against the Zcash developers' published fingerprints. Lock it never downloads them.",
  ]],
  ["What Zecky never does", "No account, no tracking.", [
    "No account, no analytics, no tracking, no ads. Lock it never sends your words, balance or history anywhere.",
  ], []],
  ["Where it stands", "Early software.", [
    "Lock it for Windows is new and in testing. Start with small amounts, and keep your 24 words on paper.",
  ], []],
];

(function buildChapters() {
  const box = $("chapters");
  for (const [title, summary, essentials, more] of CHAPTERS) {
    const chapter = document.createElement("div");
    chapter.className = "chapter";
    const head = document.createElement("button");
    head.type = "button";
    head.className = "chapter-head";
    const t = document.createElement("strong");
    t.textContent = title;
    const sm = document.createElement("span");
    sm.textContent = summary;
    head.append(t, sm);
    const body = document.createElement("div");
    body.className = "chapter-body";
    body.hidden = true;
    for (const text of essentials) {
      const p = document.createElement("p");
      p.textContent = text;
      body.append(p);
    }
    if (more.length) {
      const toggle = document.createElement("button");
      toggle.type = "button";
      toggle.className = "link-gold small-link";
      toggle.textContent = "More detail";
      const box2 = document.createElement("div");
      box2.className = "more";
      box2.hidden = true;
      for (const text of more) {
        const p = document.createElement("p");
        p.textContent = text;
        box2.append(p);
      }
      toggle.addEventListener("click", () => { box2.hidden = !box2.hidden; toggle.textContent = box2.hidden ? "More detail" : "Less"; });
      body.append(toggle, box2);
    }
    head.addEventListener("click", () => { body.hidden = !body.hidden; });
    chapter.append(head, body);
    box.append(chapter);
  }
})();

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

// ---------- Start ----------

async function start() {
  const first = await invoke("wallet_overview");
  showView(first.exists ? "home" : "welcome");
}

start().catch((problem) => {
  showView("loading");
  views.loading.querySelector("p").textContent = describe(problem) || "The app could not start.";
});
