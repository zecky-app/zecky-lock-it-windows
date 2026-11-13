// Zecky Lock it: the window. It draws and asks; it never holds a key.
// The 24 words pass through here once, to be written down (or typed in for a
// restore), and a passphrase on its way to the Rust side. Fields are cleared
// as soon as they are sent. The lock's words stay on the Rust side.

"use strict";

const invoke = window.__TAURI__.core.invoke;
const listen = window.__TAURI__.event ? window.__TAURI__.event.listen : null;

const KEY_NAMES = { device_hello: "This PC", security_key: "Security key", passphrase: "Passphrase", code: "Zecky code" };
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

// ---------- Appearance and value (Theme.swift, PriceFeed.swift) ----------

const themeMedia = window.matchMedia("(prefers-color-scheme: light)");
function themeChoice() { return localStorageGet("theme.mode") || "dark"; }
function applyTheme() {
  const choice = themeChoice();
  const light = choice === "light" || (choice === "auto" && themeMedia.matches);
  document.documentElement.dataset.theme = light ? "light" : "dark";
  const gold = light ? [169, 121, 43] : [212, 166, 87];
  const medallion = document.getElementById("medallion");
  if (medallion && typeof Lockart !== "undefined") Lockart.drawMedallion(medallion, gold);
}
themeMedia.addEventListener("change", applyTheme);

function currency() { return localStorageGet("value.currency") || "none"; }
let rates = {};
let priceTimer = null;
async function fetchPrice() {
  if (currency() === "none") return;
  try { rates = await invoke("price_get"); } catch { /* keep the last price */ }
  if (typeof overview !== "undefined" && overview) renderHome();
}
function restartPrice() {
  clearInterval(priceTimer);
  rates = {};
  if (currency() === "none") return;
  fetchPrice();
  priceTimer = setInterval(fetchPrice, 5 * 60 * 1000);
}
/// "≈ $34.12", "≈ €31.50", "≈ 0.00031200 BTC", or null when off or unknown.
function valueOf(zats) {
  const c = currency();
  const rate = rates[c];
  if (c === "none" || !rate) return null;
  const amount = (zats / 1e8) * rate;
  if (c === "btc") return `≈ ${amount.toLocaleString("en-US", { minimumFractionDigits: 8, maximumFractionDigits: 8 })} BTC`;
  const symbol = c === "usd" ? "$" : "€";
  if (amount > 0 && amount < 0.01) return `≈ <${symbol}0.01`;
  return `≈ ${symbol}${amount.toLocaleString("en-US", { minimumFractionDigits: 2, maximumFractionDigits: 2 })}`;
}

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
  check: $("view-check"), restore: $("view-restore"), setup: $("view-setup"), terms: $("view-terms"),
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
Lockart.padlock(document.querySelector('[data-padlock="terms"]'), { size: 64 });

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

$("words-back").addEventListener("click", () => { forgetWords(); leaveOnboarding(); });

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
  if (adding) finishAdding();
  else openSetup();
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
  $("restore-block").value = "";
  say($("restore-message"), "");
  showView("restore");
  $("restore-words").focus();
});
$("restore-back").addEventListener("click", () => { forgetWords(); leaveOnboarding(); });
/// The typed birthday block: null when empty; throws when it is not a block.
function restoreBlock() {
  const text = $("restore-block").value.replace(/[\s,.']/g, "");
  if (!text) return null;
  const block = Number(text);
  if (!/^\d+$/.test(text) || block < 419200 || block > 99999999) {
    throw new Error("The birthday block is a number from 419,200 up. Leave it empty if you are not sure.");
  }
  return block;
}

$("restore-submit").addEventListener("click", async () => {
  try {
    restoreBlock();
    await invoke("wallet_use_words", { words: $("restore-words").value });
    $("restore-words").value = "";
    madeIn = { year: Number($("restore-year").value), month: Number($("restore-month").value), block: restoreBlock() };
    if (adding) finishAdding();
    else openSetup();
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
applyTheme();
restartPrice();
Lockart.padlock(document.querySelector('[data-padlock="prompt"]'), { size: 62 });
Lockart.padlock(document.querySelector('[data-padlock="timelock"]'), { size: 62 });
Lockart.padlock(document.querySelector('[data-padlock="applock"]'), { size: 150, plate: "LOCKED" });

let overview = null;
let rules = null;
let activity = [];
let homeTimer = null;
let lockOpen = false;
let shownHeight = 0;
let beats = 0;
let beatToken = 0;
const heightSamples = [];

function startHome() {
  if (homeTimer) return;
  maybeShowTour();
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
  rules = await invoke("lock_rules").catch(() => rules);
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
  renderKeyRing();
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

  const value = hidden ? null : valueOf(b.total);
  $("value-line").hidden = !value;
  $("value-line").textContent = value || "";
  const days = heldDays();
  $("held-line").textContent = b.total > 0 ? (days === null ? "Locked on this PC" : `Locked ${Holding.heldFor(days)} ›`) : "";
  $("held-line").disabled = days === null;
  $("held-line").title = days === null ? "" : "Milestones and highlights";
  // Moves inside the wallet, as on the Mac: only once up to date.
  const upToDate = overview.sync.state === "up_to_date";
  const ironwood = $("move-ironwood");
  const lockin = $("move-lockin");
  ironwood.hidden = !(upToDate && b.orchard > 0);
  lockin.hidden = !(upToDate && b.transparent > 0);
  if (!ironwood.hidden) ironwood.innerHTML = `${hidden ? "Part of your Zcash is in the old Orchard pool" : `${amountText(b.orchard)} in the old Orchard pool`} · <strong>Move it to Ironwood ›</strong>`;
  if (!lockin.hidden) lockin.innerHTML = `${hidden ? "Zcash waiting outside the lock" : `+${amountText(b.transparent)} waiting outside the lock`} · <strong>Lock it in ›</strong>`;
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
  const sealed = sealedWallet();
  $("home-seal-send").hidden = !sealed;
  if (sealed && b.total > 0 && days === null) $("held-line").textContent = "Keys on your phone (Seal it)";
  padlockHost.title = sealed ? "Send with Seal it" : padlockHost.title;
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
  renderTimeLockLine();
  refreshCoin();
}

const dateTime = (seconds) => new Date(seconds * 1000).toLocaleString(undefined, { weekday: "long", day: "numeric", month: "short", hour: "2-digit", minute: "2-digit" });

function renderTimeLockLine() {
  const line = $("timelock-line");
  const notice = $("glass-notice");
  if (!rules) { line.hidden = true; notice.hidden = true; return; }
  notice.hidden = !(rules.broken_notice && rules.last_broken);
  if (!notice.hidden) $("glass-notice-text").textContent = `Glass broken on ${dateTime(rules.last_broken)}. The time lock is off now.`;
  const p = rules.phase;
  line.hidden = !(p.phase === "waiting" || p.phase === "ready");
  line.classList.toggle("ready", p.phase === "ready");
  if (p.phase === "waiting") line.textContent = `◷ The lock can be opened from ${dateTime(p.until)}`;
  if (p.phase === "ready") line.textContent = `◷ The lock can be opened until ${dateTime(p.until)}`;
}

$("glass-seen").addEventListener("click", async () => { await invoke("seen_broken").catch(() => {}); if (rules) rules.broken_notice = false; renderTimeLockLine(); });

// ---------- Time lock: ask, or wait (TimeLockPrompt) ----------

function hoursLabel(h) { return h === 0 ? "Off" : h === 24 ? "24 hours" : h % 24 === 0 ? `${h / 24} days` : `${h} hours`; }

function showTimeLock() {
  const p = rules.phase;
  $("tl-label").textContent = `TIME LOCK · ${hoursLabel(rules.hours).toUpperCase()}`;
  $("tl-asking").hidden = false;
  $("tl-glass").hidden = true;
  $("tl-sentence").value = "";
  say($("tl-message"), "");
  if (p.phase === "waiting") {
    $("tl-title").textContent = "The lock is waiting";
    $("tl-text").textContent = `It can be opened from ${dateTime(p.until)}, for 24 hours. Nothing can leave before then, whoever asks.`;
    $("tl-ask").textContent = "Cancel the request";
  } else {
    const ready = Date.now() / 1000 + rules.hours * 3600;
    $("tl-title").textContent = "Ask to open the lock";
    $("tl-text").textContent = `Your time lock asks for ${hoursLabel(rules.hours)} first. Start the wait now, and the lock can be opened from ${dateTime(ready)}, for 24 hours. Nobody legitimate will ever rush you.`;
    $("tl-ask").textContent = "Start the wait";
  }
  $("timelock-prompt").hidden = false;
  $("tl-keep").focus();
}

function hideTimeLock() { $("timelock-prompt").hidden = true; $("tl-sentence").value = ""; }

$("tl-keep").addEventListener("click", hideTimeLock);
$("tl-glass-keep").addEventListener("click", hideTimeLock);
$("tl-ask").addEventListener("click", async () => {
  const waiting = rules.phase.phase === "waiting";
  await invoke(waiting ? "time_lock_cancel_request" : "time_lock_request").catch(() => {});
  rules = await invoke("lock_rules").catch(() => rules);
  hideTimeLock();
  renderHome();
});
$("tl-break").addEventListener("click", () => { $("tl-asking").hidden = true; $("tl-glass").hidden = false; $("tl-sentence").focus(); });
$("tl-sentence").addEventListener("input", () => {
  const ok = $("tl-sentence").value.trim().replace(/[.!]+$/, "").toLowerCase() === "i am breaking the glass myself";
  $("tl-glass-go").disabled = !ok;
});
$("tl-glass-go").addEventListener("click", async () => {
  const sentence = $("tl-sentence").value;
  hideTimeLock();
  try {
    const typed = await askKeys("Break the glass", "Your keys, then the lock opens straight away and the time lock turns off.", { danger: true, action: "Break the glass" });
    await invoke("break_glass", { sentence, typed: typedOnly(typed) });
    lockOpen = true;
    rules = await invoke("lock_rules").catch(() => rules);
    renderHome();
  } catch (problem) {
    if (problem && problem.message !== "cancelled") $("open-note").textContent = describe(problem) || "Not confirmed, so the lock stayed closed.";
  }
});

// ---------- Lock it is locked: asked when it opens, and after 5 minutes away ----------

let appLocked = false;
let lastActive = Date.now();
let lastTick = Date.now();
const AWAY = 5 * 60 * 1000;

function showAppLocked() {
  appLocked = true;
  if (lockOpen) closeLock();
  closeSheet();
  keepLocked();
  hideTimeLock();
  $("confirm").hidden = true;
  const hello = rules && rules.start_with_hello;
  $("app-locked-passphrase-field").hidden = hello || !needsPassphrase();
  $("app-locked-code-field").hidden = hello || !needsCode();
  $("app-locked-lead").textContent = hello ? "Windows Hello opens it: face, fingerprint or PIN." : "Your keys open it.";
  say($("app-locked-message"), "");
  $("eye").hidden = true;
  $("app-locked").hidden = false;
  $("app-unlock").focus();
}

$("app-unlock").addEventListener("click", async () => {
  const typed = { passphrase: $("app-locked-passphrase").value || null, code: $("app-locked-code").value || null };
  $("app-locked-passphrase").value = "";
  $("app-locked-code").value = "";
  $("app-unlock").disabled = true;
  say($("app-locked-message"), "Follow the prompt from Windows.", true);
  try {
    await invoke("app_unlock", { typed });
    appLocked = false;
    lastActive = Date.now();
    $("app-locked").hidden = true;
    $("eye").hidden = false;
    refreshHome();
    maybeShowTour();
  } catch (problem) {
    say($("app-locked-message"), describe(problem) || "Not confirmed, so Lock it stayed locked.");
  } finally {
    $("app-unlock").disabled = false;
  }
});

for (const type of ["mousemove", "mousedown", "keydown", "wheel"]) document.addEventListener(type, () => { lastActive = Date.now(); }, { passive: true });

setInterval(() => {
  const now = Date.now();
  const slept = now - lastTick > 2 * 60 * 1000;
  lastTick = now;
  if (appLocked || sending || !overview || !overview.exists || !(rules && rules.ask_on_start)) return;
  if (slept || now - lastActive > AWAY) showAppLocked();
}, 15000);

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
function needsCode() { return overview && overview.keys.some((key) => key.kind === "code"); }

async function openWithoutPause() {
  try {
    const typed = needsPassphrase() || needsCode() ? await askKeys("Open the lock", "Your keys open the lock.", { action: "Open the lock" }) : {};
    await invoke("wallet_open_lock", { typed });
    lockOpen = true;
    renderHome();
  } catch (problem) {
    if (problem && problem.message !== "cancelled") $("open-note").textContent = describe(problem) || "Not confirmed, so the lock stayed closed.";
  }
}

function askToOpen() {
  if (lockOpen || !overview) return;
  if (sealedWallet()) return openSeal("send");
  if (rules && (rules.phase.phase === "not_asked" || rules.phase.phase === "waiting")) return showTimeLock();
  OPEN_WAIT = overview.settings ? overview.settings.wait : 10;
  if (OPEN_WAIT === 0) return openWithoutPause();
  const days = heldDays();
  const held = days === null ? "" : days < 1 ? "This Zcash was locked today. " : `This Zcash has been locked ${Holding.heldFor(days)}. `;
  $("prompt-intro").textContent = held + "Nobody legitimate will ever rush you, ask for your 24 words, or tell you to move your Zcash to keep it safe.";
  $("prompt-agree").checked = false;
  $("prompt-passphrase").value = "";
  $("prompt-code").value = "";
  $("prompt-passphrase-field").hidden = !needsPassphrase();
  $("prompt-code-field").hidden = !needsCode();
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
  $("prompt-code").value = "";
  $("open-prompt").hidden = true;
}

$("prompt-agree").addEventListener("change", tickPrompt);
$("prompt-keep").addEventListener("click", keepLocked);
$("prompt-open").addEventListener("click", async () => {
  if (busy || countdown > 0 || !$("prompt-agree").checked) return;
  const typed = { passphrase: needsPassphrase() ? $("prompt-passphrase").value : null, code: needsCode() ? $("prompt-code").value : null };
  $("prompt-passphrase").value = "";
  $("prompt-code").value = "";
  busy = true;
  tickPrompt();
  say($("prompt-message"), "Follow the prompts from Windows.", true);
  try {
    await invoke("wallet_open_lock", { typed });
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

// ---------- Wallets: the key ring (as Lock it Mac) ----------

/// The wallet being added (its name) while the words are shown or typed.
let adding = null;
const viewTitles = {
  words: views.words.querySelector(".title").textContent,
  restore: views.restore.querySelector(".title").textContent,
};

function walletName(w) {
  return w.main && w.name === "Lock it" ? "This PC" : w.name;
}

function currentWallet() {
  const list = (overview && overview.wallets) || [];
  const id = overview && overview.settings && overview.settings.current;
  return list.find((w) => (id ? w.id === id : w.main)) || list[0] || null;
}

const keyOf = (w) => (w.main ? "main" : w.id);
let showHidden = false;

function ringLayout() {
  const s = (overview && overview.settings) || {};
  return { order: s.order || [], hidden: s.hidden || [] };
}

/// The wallets in the owner's order: This PC first unless moved.
function orderedWallets() {
  const list = (overview && overview.wallets) || [];
  const { order } = ringLayout();
  const rank = (w) => {
    const i = order.indexOf(keyOf(w));
    return i === -1 ? (w.main ? -1 : 1000 + list.indexOf(w)) : i;
  };
  return [...list].sort((a, b) => rank(a) - rank(b));
}

function shownWallets() {
  const { hidden } = ringLayout();
  return orderedWallets().filter((w) => showHidden || w.main || !hidden.includes(w.id));
}

const LOCK_ICON = '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M7 10V8a5 5 0 0 1 10 0v2h1a1 1 0 0 1 1 1v9a1 1 0 0 1-1 1H6a1 1 0 0 1-1-1v-9a1 1 0 0 1 1-1zm2 0h6V8a3 3 0 0 0-6 0z"/></svg>';
const ENVELOPE_ICON = '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M3 5h18a1 1 0 0 1 1 1v12a1 1 0 0 1-1 1H3a1 1 0 0 1-1-1V6a1 1 0 0 1 1-1zm9 7.2L4.5 7v.2l7.5 5.2 7.5-5.2V7z"/></svg>';

function renderKeyRing() {
  const ring = $("key-ring");
  ring.textContent = "";
  const current = currentWallet();
  const { hidden } = ringLayout();
  for (const w of shownWallets()) {
    const chip = document.createElement("button");
    chip.type = "button";
    chip.className = "ring-chip";
    chip.classList.toggle("on", Boolean(current && current.id === w.id));
    chip.classList.toggle("out", hidden.includes(w.id));
    chip.disabled = lockOpen;
    chip.title = lockOpen ? "Close the lock to switch wallets" : w.watch_only ? "Seal it: signed on your phone" : "";
    chip.innerHTML = w.watch_only ? ENVELOPE_ICON : LOCK_ICON;
    const label = document.createElement("span");
    label.textContent = walletName(w);
    chip.append(label);
    chip.addEventListener("click", () => chooseWallet(w));
    chip.addEventListener("contextmenu", (event) => {
      event.preventDefault();
      if (!lockOpen) openRingMenu(event, w);
    });
    ring.append(chip);
  }
  if (orderedWallets().some((w) => !w.main && hidden.includes(w.id))) {
    const star = document.createElement("button");
    star.type = "button";
    star.className = "ring-round";
    star.textContent = showHidden ? "★" : "☆";
    star.title = showHidden ? "Put them out of sight again" : "Show the wallets kept out of sight";
    star.disabled = lockOpen;
    star.addEventListener("click", () => { showHidden = !showHidden; leaveIfOutOfSight(); renderKeyRing(); });
    ring.append(star);
  }
  const plus = document.createElement("button");
  plus.type = "button";
  plus.className = "ring-round plus";
  plus.textContent = "+";
  plus.title = "Add a wallet to the key ring";
  plus.disabled = lockOpen;
  plus.addEventListener("click", (event) => openPlusMenu(event));
  ring.append(plus);
}

function leaveIfOutOfSight() {
  const w = currentWallet();
  if (!showHidden && w && !w.main && ringLayout().hidden.includes(w.id)) chooseWallet(orderedWallets().find((x) => x.main));
}

async function chooseWallet(w) {
  if (!w || lockOpen) return;
  try {
    await invoke("keyring_select", { id: w.main ? null : w.id });
    activity = [];
    await refreshHome();
  } catch (problem) {
    alert(describe(problem));
  }
}

// The small menus: right-click on a chip, and "+".
const ringMenu = $("ring-menu");

function showMenu(event, items) {
  ringMenu.textContent = "";
  for (const item of items) {
    if (item === "-") { ringMenu.append(document.createElement("hr")); continue; }
    const button = document.createElement("button");
    button.type = "button";
    button.textContent = item.label;
    if (item.danger) button.className = "danger-text";
    button.disabled = Boolean(item.disabled);
    button.addEventListener("click", () => { closeRingMenu(); item.run(); });
    ringMenu.append(button);
  }
  ringMenu.hidden = false;
  const rect = ringMenu.getBoundingClientRect();
  ringMenu.style.left = `${Math.min(event.clientX, window.innerWidth - rect.width - 8)}px`;
  ringMenu.style.top = `${Math.min(event.clientY + 4, window.innerHeight - rect.height - 8)}px`;
  setTimeout(() => document.addEventListener("click", closeRingMenu, { once: true }), 0);
}

function closeRingMenu() { ringMenu.hidden = true; }
document.addEventListener("keydown", (event) => { if (event.key === "Escape") closeRingMenu(); });

function openPlusMenu(event) {
  showMenu(event, [
    { label: "Pair Seal it (offline phone)", run: () => openSeal("pair") },
    "-",
    { label: "New wallet on this PC", run: () => openAdd("new") },
    { label: "Restore a wallet with 24 words", run: () => openAdd("restore") },
  ]);
}

function openRingMenu(event, w) {
  const shown = shownWallets();
  const index = shown.indexOf(w);
  const items = [{ label: "Rename…", run: () => openRename(w) }];
  if (!w.main && !w.watch_only) items.push({ label: "Show its words…", run: () => showItsWords(w) });
  items.push("-",
    { label: "Move left", disabled: index <= 0, run: () => moveWallet(w, -1) },
    { label: "Move right", disabled: index >= shown.length - 1, run: () => moveWallet(w, 1) });
  if (!w.main) {
    const out = ringLayout().hidden.includes(w.id);
    items.push({ label: out ? "Bring back into sight" : "Keep out of sight", run: () => hideWallet(w, !out) });
  }
  items.push("-", w.watch_only
    ? { label: "Remove from Lock it…", danger: true, run: () => removeWallet(w) }
    : { label: "Erase this wallet…", danger: true, run: () => (w.main ? eraseMain(w) : removeWallet(w)) });
  showMenu(event, items);
}

async function saveLayout(order, hidden) {
  try {
    await invoke("keyring_layout", { order, hidden });
    overview.settings.order = order;
    overview.settings.hidden = hidden;
    renderKeyRing();
  } catch (problem) {
    alert(describe(problem));
  }
}

function moveWallet(w, step) {
  const all = orderedWallets().map(keyOf);
  const shown = shownWallets().map(keyOf);
  const at = shown.indexOf(keyOf(w));
  const neighbour = shown[at + step];
  if (neighbour === undefined) return;
  const from = all.indexOf(keyOf(w));
  const to = all.indexOf(neighbour);
  all.splice(from, 1);
  all.splice(to, 0, keyOf(w));
  saveLayout(all, ringLayout().hidden);
}

function hideWallet(w, out) {
  const hidden = ringLayout().hidden.filter((id) => id !== w.id);
  if (out) hidden.push(w.id);
  saveLayout(orderedWallets().map(keyOf), hidden).then(leaveIfOutOfSight);
}

// Adding and renaming share a small sheet.
let walletSheet = null;

function openAdd(mode) {
  walletSheet = { mode };
  $("wallet-sheet-title").textContent = mode === "new" ? "New wallet on this PC" : "Restore a wallet";
  $("wallet-sheet-sub").textContent = mode === "new"
    ? "It sits behind the same lock and the same keys, with its own 24 words. Write those down too."
    : "Type its 24 words next. It goes behind the same lock and the same keys.";
  $("wallet-name").value = "";
  $("wallet-go").textContent = "Continue";
  say($("wallet-message"), "");
  openSheet("wallets");
  $("wallet-name").focus();
}

function openRename(w) {
  walletSheet = { mode: "rename", wallet: w };
  $("wallet-sheet-title").textContent = "Rename wallet";
  $("wallet-sheet-sub").textContent = "The name is only shown on this PC.";
  $("wallet-name").value = walletName(w);
  $("wallet-go").textContent = "Rename";
  say($("wallet-message"), "");
  openSheet("wallets");
  $("wallet-name").select();
}

$("wallet-name").addEventListener("keydown", (event) => { if (event.key === "Enter") $("wallet-go").click(); });

$("wallet-go").addEventListener("click", async () => {
  const name = $("wallet-name").value.trim();
  if (!name) return say($("wallet-message"), "Give the wallet a name first.");
  const sheet = walletSheet || { mode: "new" };
  try {
    if (sheet.mode === "rename") {
      await invoke("keyring_rename", { id: sheet.wallet.main ? null : sheet.wallet.id, name });
      closeSheet();
      return refreshHome();
    }
    adding = name;
    if (sheet.mode === "new") {
      words = await invoke("wallet_new_words");
      madeIn = null;
      views.words.querySelector(".title").textContent = `Words for ${name}`;
      listWords();
      closeSheet();
      showView("words");
    } else {
      views.restore.querySelector(".title").textContent = `Restore ${name}`;
      $("restore-words").value = "";
      $("restore-block").value = "";
      say($("restore-message"), "");
      closeSheet();
      showView("restore");
      $("restore-words").focus();
    }
  } catch (problem) {
    say($("wallet-message"), describe(problem));
  }
});

let ringWordsTimer = null;
async function showItsWords(w) {
  try {
    const typed = await askKeys("Confirm it's you", `${walletName(w)}'s 24 words are that wallet. Make sure nobody can see your screen.`, { action: "Show the words" });
    const list = await invoke("wallet_show_words", { typed: typedOnly(typed), id: w.id });
    const birthday = list.pop();
    const ol = $("ring-words");
    ol.textContent = "";
    for (const word of list) {
      const li = document.createElement("li");
      li.textContent = word;
      ol.append(li);
    }
    $("words-sheet-title").textContent = `${walletName(w)}: its words`;
    $("ring-birthday").textContent = Number(birthday) ? `Birthday block ${numbers.format(Number(birthday))}: with the words, it tells any Zcash wallet where to start looking.` : "";
    openSheet("words");
    clearTimeout(ringWordsTimer);
    ringWordsTimer = setTimeout(() => { $("ring-words").textContent = ""; if (!$("sheet-words").hidden) closeSheet(); }, 120000);
  } catch (problem) {
    const text = describe(problem);
    if (text && text !== "cancelled") alert(text);
  }
}

async function removeWallet(w) {
  try {
    if (w.watch_only) {
      const ok = await askKeys(`Remove ${walletName(w)} from Lock it?`, "Its keys stay on your phone: you can pair it again at any time.", { danger: true, action: "Remove", keys: false });
      await invoke("keyring_remove", { id: w.id, typed: typedOnly(ok) });
    } else {
      const typed = await askKeys(`Erase ${walletName(w)}?`, "Its words leave this PC. Make sure you have them on paper: without them, its Zcash cannot be taken out anywhere.", { danger: true, action: "Erase" });
      await invoke("keyring_remove", { id: w.id, typed: typedOnly(typed) });
    }
    activity = [];
    await refreshHome();
  } catch (problem) {
    const text = describe(problem);
    if (text && text !== "cancelled") alert(text);
  }
}

async function eraseMain(w) {
  const next = orderedWallets().find((x) => !x.main && !x.watch_only);
  if (!next) {
    alert("This is the only wallet with its words on this PC. To erase it, use Erase everything in Settings › Security.");
    return;
  }
  try {
    const title = walletName(w) === "This PC" ? "this PC's wallet" : walletName(w);
    const typed = await askKeys(`Erase ${title}?`, `Its words leave this PC: make sure you have them on paper. The lock passes to ${walletName(next)}, with the same keys and rules.`, { danger: true, action: "Erase" });
    await invoke("keyring_erase_main", { typed: typedOnly(typed) });
    activity = [];
    await refreshHome();
  } catch (problem) {
    const text = describe(problem);
    if (text && text !== "cancelled") alert(text);
  }
}

function leaveOnboarding() {
  if (adding) {
    adding = null;
    views.words.querySelector(".title").textContent = viewTitles.words;
    views.restore.querySelector(".title").textContent = viewTitles.restore;
    showView("home");
  } else {
    showView("welcome");
  }
}

async function finishAdding() {
  const name = adding;
  const message = views.check.hidden ? $("restore-message") : $("check-message");
  try {
    const typed = await askKeys(`Add ${name}`, "Your keys are asked once: the new words are sealed with your main words, behind the same lock.", { action: "Add the wallet" });
    say(message, "Adding the wallet. This takes a moment.");
    await invoke("keyring_add", { name, madeIn, typed: typedOnly(typed) });
    say(message, "");
    forgetWords();
    leaveOnboarding();
    activity = [];
    await refreshHome();
  } catch (problem) {
    const text = describe(problem);
    say(message, text || "Not confirmed, so nothing was added.");
  }
}

// ---------- Beta: small amounts, problem reports ----------

$("send-amount").addEventListener("input", () => {
  const value = Number($("send-amount").value.replace(",", "."));
  $("send-beta").hidden = !(value > 0.1);
});

$("report-copy").addEventListener("click", async () => {
  const o = overview || {};
  const b = o.balance || {};
  const s = o.sync || {};
  let version = "";
  try { version = (await invoke("settings_get")).version; } catch (_) { /* not needed */ }
  const lines = [
    "Zecky Lock it for Windows: problem report",
    `Version: ${version}`,
    `Windows: ${navigator.userAgent}`,
    `Sync: ${s.state || "?"} via ${s.server || "?"}${s.problem ? ` (${s.problem})` : ""}`,
    `Scanned to block ${b.scanned_height || "?"} of ${b.chain_tip || "?"}`,
    `Wallets: ${(o.wallets || []).length} (Seal it: ${(o.wallets || []).filter((w) => w.watch_only).length})`,
    `Keys: ${(o.keys || []).map((k) => k.kind).join(", ")}`,
    `Tor for sending: ${o.settings && o.settings.tor ? "on" : "off"}`,
    "Recent problems:",
    ...((o.problems || []).map((p) => `  ${new Date(p.at * 1000).toISOString()}  ${p.message}`)),
    "",
    "What happened (in your words):",
    "",
  ];
  try {
    await navigator.clipboard.writeText(lines.join("\n"));
    say($("report-message"), "Copied. Paste it in a new issue on the GitHub page and add what you did.", true);
  } catch (_) {
    say($("report-message"), "Copying did not work. Take a screenshot of this page instead.");
  }
});

// ---------- Seal it ----------

function sealedWallet() {
  const w = currentWallet();
  return Boolean(w && w.watch_only);
}

const seal = { mode: null, frames: [], frameTimer: null, stream: null, scanTimer: null, kind: null };

(function fillSealDates() {
  const month = $("seal-month");
  MONTHS.forEach((name, index) => month.append(new Option(name, index + 1)));
  const year = $("seal-year");
  const now = new Date();
  for (let y = now.getFullYear(); y >= 2018; y -= 1) year.append(new Option(String(y), y));
  month.value = String(now.getMonth() + 1);
})();

function sealStage(name) {
  for (const stage of document.querySelectorAll("#sheet-seal .seal-stage")) stage.hidden = stage.dataset.seal !== name;
  if (name !== "show") stopFrames();
  if (name !== "scan") stopCamera();
  say($("seal-message"), "");
}

function openSeal(mode) {
  seal.mode = mode;
  closeSheet();
  openSheet("seal");
  if (mode === "pair") {
    $("seal-title").textContent = "Pair a phone";
    $("seal-sub").textContent = "The keys stay on your phone. This PC only sees the balance and builds payments.";
    startScan("accounts", "In Seal it on your phone, open the account QR code and hold it in front of the camera.");
  } else {
    const w = currentWallet();
    $("seal-title").textContent = `Send from ${w ? walletName(w) : "Seal it"}`;
    $("seal-sub").textContent = "Your phone signs it. Nothing that can spend is on this PC.";
    for (const id of ["seal-to", "seal-amount", "seal-memo"]) $(id).value = "";
    $("seal-summary").hidden = true;
    $("seal-show").hidden = true;
    $("seal-work").hidden = false;
    sealStage("form");
    $("seal-to").focus();
  }
}

function closeSeal() {
  stopFrames();
  stopCamera();
  invoke("sealit_cancel").catch(() => {});
  closeSheet();
}
$("seal-close").addEventListener("click", closeSeal);
$("home-seal-send").addEventListener("click", () => openSeal("send"));
$("seal-done").addEventListener("click", closeSeal);

for (const id of ["seal-to", "seal-amount", "seal-memo"]) {
  $(id).addEventListener("input", () => { $("seal-summary").hidden = true; $("seal-show").hidden = true; $("seal-work").hidden = false; });
}

$("seal-work").addEventListener("click", async () => {
  say($("seal-message"), "");
  try {
    const p = await invoke("wallet_prepare", { to: $("seal-to").value.trim(), amount: $("seal-amount").value, memo: $("seal-memo").value || null });
    const box = $("seal-summary");
    box.textContent = "";
    for (const [label, value] of [["To", p.to], ["Amount", amountText(p.amount)], ["Fee", amountText(p.fee)], ["Total", amountText(p.total)]]) {
      const line = document.createElement("p");
      line.className = label === "To" ? "mono small" : "";
      line.textContent = `${label}: ${value}`;
      box.append(line);
    }
    box.hidden = false;
    $("seal-work").hidden = true;
    $("seal-show").hidden = false;
  } catch (problem) {
    say($("seal-message"), describe(problem));
  }
});

$("seal-show").addEventListener("click", async () => {
  say($("seal-message"), "Building the payment…", true);
  $("seal-show").disabled = true;
  try {
    const unsigned = await invoke("sealit_start");
    seal.frames = unsigned.frames.map((svg) => "data:image/svg+xml;charset=utf-8," + encodeURIComponent(svg));
    sealStage("show");
    let index = 0;
    const show = () => {
      $("seal-qr").src = seal.frames[index % seal.frames.length];
      $("seal-frame").textContent = seal.frames.length > 1 ? "A moving code: keep the phone on it until it is read." : "";
      index += 1;
    };
    show();
    if (seal.frames.length > 1) seal.frameTimer = setInterval(show, 250);
  } catch (problem) {
    say($("seal-message"), describe(problem));
  } finally {
    $("seal-show").disabled = false;
  }
});

function stopFrames() {
  clearInterval(seal.frameTimer);
  seal.frameTimer = null;
}

$("seal-signed").addEventListener("click", () => startScan("pczt", "Now hold the phone's signed code in front of the camera."));

async function startScan(kind, hint) {
  seal.kind = kind;
  sealStage("scan");
  document.querySelector("#sheet-seal .camera").hidden = false;
  $("seal-progress").textContent = hint;
  $("seal-paste").value = "";
  $("seal-paste").placeholder = kind === "accounts" ? "ur:zcash-accounts/…" : "ur:zcash-pczt/…";
  try {
    await invoke("sealit_scan_start", { kind });
  } catch (problem) {
    return say($("seal-message"), describe(problem));
  }
  if (!navigator.mediaDevices || !navigator.mediaDevices.getUserMedia) return noCamera("This PC has no camera that Lock it can use.");
  try {
    seal.stream = await navigator.mediaDevices.getUserMedia({ video: { facingMode: "user", width: { ideal: 1280 }, height: { ideal: 720 } }, audio: false });
  } catch (error) {
    const name = error && error.name;
    return noCamera(name === "NotAllowedError"
      ? "Lock it was not allowed to use the camera. Allow it in Windows Settings › Privacy › Camera, then try again."
      : name === "NotFoundError" || name === "OverconstrainedError"
        ? "No webcam was found. Seal it reads the phone's codes with a webcam; everything else in Lock it works without one."
        : "The camera could not start. Close other apps that use it, then try again.");
  }
  const video = $("seal-video");
  video.srcObject = seal.stream;
  await video.play().catch(() => {});
  const canvas = document.createElement("canvas");
  const context = canvas.getContext("2d", { willReadFrequently: true });
  let last = "";
  seal.scanTimer = setInterval(async () => {
    if (!video.videoWidth) return;
    const width = 640;
    const height = Math.round((video.videoHeight / video.videoWidth) * width);
    canvas.width = width;
    canvas.height = height;
    context.drawImage(video, 0, 0, width, height);
    const found = jsQR(context.getImageData(0, 0, width, height).data, width, height, { inversionAttempts: "attemptBoth" });
    if (!found || !found.data || found.data === last) return;
    last = found.data;
    await pushScanned(found.data);
  }, 120);
}

function noCamera(text) {
  document.querySelector("#sheet-seal .camera").hidden = true;
  $("seal-progress").textContent = text;
  $("seal-paste-box").open = true;
}

function stopCamera() {
  clearInterval(seal.scanTimer);
  seal.scanTimer = null;
  if (seal.stream) for (const track of seal.stream.getTracks()) track.stop();
  seal.stream = null;
  const video = $("seal-video");
  if (video) video.srcObject = null;
}

let pushing = false;
async function pushScanned(text) {
  if (pushing) return;
  pushing = true;
  try {
    const progress = await invoke("sealit_scan_push", { text });
    if (!progress.done) {
      $("seal-progress").textContent = progress.of > 1 ? `Reading the moving code: ${Math.min(99, Math.round((progress.seen / progress.of) * 100))}%` : "Reading…";
      return;
    }
    stopCamera();
    if (seal.kind === "accounts") sealStage("age");
    else await finishSeal();
  } catch (problem) {
    say($("seal-message"), describe(problem));
  } finally {
    pushing = false;
  }
}

$("seal-paste-go").addEventListener("click", async () => {
  const lines = $("seal-paste").value.split(/\s+/).filter(Boolean);
  for (const line of lines) await pushScanned(line);
});

async function pair(madeIn) {
  say($("seal-message"), "Pairing. This takes a moment.", true);
  try {
    await invoke("sealit_pair", { madeIn });
    activity = [];
    await refreshHome();
    sealStage("done");
    $("seal-done-text").textContent = "Paired. Lock it now follows this phone wallet's balance; payments from it are signed on the phone.";
  } catch (problem) {
    say($("seal-message"), describe(problem));
  }
}
$("seal-new").addEventListener("click", () => pair(null));
$("seal-old").addEventListener("click", () => pair({ year: Number($("seal-year").value), month: Number($("seal-month").value) }));

async function finishSeal() {
  sealStage("done");
  $("seal-done-text").textContent = "Finishing the proofs and sending…";
  $("seal-done").hidden = true;
  try {
    const over = overview && overview.settings ? overview.settings.tor : true;
    const sent = await invoke("sealit_finish", { overTor: over });
    $("seal-done-text").textContent = `Sent. ${sent.accepted_by.length ? "The network has it." : ""}`.trim();
    activity = [];
    await refreshHome();
  } catch (problem) {
    $("seal-done-text").textContent = describe(problem) || "It did not go through.";
  } finally {
    $("seal-done").hidden = false;
  }
}

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
  else if (!$("timelock-prompt").hidden) hideTimeLock();
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
  $("tab-payments").classList.toggle("on", activityTab === "payments");
  $("tab-lock").classList.toggle("on", activityTab === "lock");
  if (activityTab === "lock") return renderLockLog();
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

// ---------- Moves inside the wallet (IronwoodSheet) ----------

let moveKind = null;
let moving = false;

async function openMove(kind) {
  moveKind = kind;
  const b = overview.balance;
  const iw = kind === "ironwood";
  $("move-title").textContent = iw ? "Move to Ironwood" : "Lock it in";
  $("move-sub").textContent = iw ? "Once, inside your own wallet." : "From the transparent address into the lock.";
  $("move-text").textContent = iw
    ? "Since the Ironwood upgrade (28 July 2026) the Orchard pool only lets Zcash out; new Zcash goes to Ironwood. Part of yours is still in Orchard. Move it once to Ironwood, in this same wallet, and it stays as private as before. Your other Zcash is not touched."
    : "Zcash on a transparent address is visible to everyone, like Bitcoin. Lock it in moves it into your shielded pool, in this same wallet, where it is private like the rest.";
  $("move-held-label").textContent = iw ? "In Orchard now" : "Outside the lock";
  $("move-arrives-label").textContent = iw ? "Arrives in Ironwood" : "Locked in";
  $("move-go").textContent = iw ? "Move to Ironwood" : "Lock it in";
  $("move-receipt").hidden = true;
  $("move-progress").hidden = true;
  $("move-buttons").hidden = false;
  $("move-go").disabled = true;
  $("move-close").hidden = false;
  say($("move-message"), "Working it out…", true);
  openSheet("move");
  try {
    const plan = await invoke("wallet_plan_move", { kind: iw ? "ironwood" : "lock_in" });
    $("move-held").textContent = exact(iw ? b.orchard : b.transparent);
    $("move-fee").textContent = exact(plan.fee);
    $("move-arrives").textContent = exact(plan.amount);
    $("move-receipt").hidden = false;
    $("move-go").disabled = false;
    say($("move-message"), "Your keys are asked to sign. Nothing leaves your wallet.", true);
  } catch (problem) {
    say($("move-message"), describe(problem));
  }
}

function closeMove() { if (!moving) closeSheet(); }

$("move-ironwood").addEventListener("click", () => openMove("ironwood"));
$("move-lockin").addEventListener("click", () => openMove("lock_in"));
$("move-close").addEventListener("click", closeMove);
$("move-cancel").addEventListener("click", closeMove);
$("move-go").addEventListener("click", async () => {
  if (moving) return;
  const iw = moveKind === "ironwood";
  let typed = {};
  try {
    if (needsPassphrase() || needsCode()) typed = await askKeys("Confirm it's you", iw ? "Your keys sign the move to Ironwood." : "Your keys sign locking it in.", { action: "Continue" });
  } catch {
    return;
  }
  moving = true;
  $("move-buttons").hidden = true;
  $("move-close").hidden = true;
  $("move-progress").hidden = false;
  $("move-step").textContent = "Follow the prompts from Windows";
  say($("move-message"), "");
  try {
    await invoke("wallet_do_move", { typed: typedOnly(typed) });
    $("move-progress").hidden = true;
    say($("move-message"), iw ? "Moved. It shows in Ironwood once it is in a block, and stays locked." : "Locking in. Once it is in a block it is locked in ✓, shielded like the rest.", true);
    $("move-close").hidden = false;
    $("move-close").textContent = "Done";
    refreshHome();
  } catch (problem) {
    $("move-progress").hidden = true;
    $("move-close").hidden = false;
    if (problem && problem.unsent) {
      say($("move-message"), `${describe(problem)}. Open Send to try again; it is the same move, so nothing is paid twice.`);
    } else {
      $("move-buttons").hidden = false;
      say($("move-message"), describe(problem) || (iw ? "Nothing was moved." : "Nothing was locked in."));
    }
  } finally {
    moving = false;
  }
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
  if (stage === "check" && overview) next.textContent = prefs.hidden ? "Send" : `Send ${exact(plan.amount)}`;
  $("send-saved").hidden = stage !== "compose";
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
  renderSendSaved();
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
    if (moving) $("move-step").textContent = event.payload;
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
    const text = describe(problem);
    const tor = /tor\b|onion|circuit|3 minutes/i.test(text)
      ? " Some networks (offices, schools, public Wi-Fi) block Tor: try again without Tor, or on another network."
      : "";
    say($("send-message"), `${text}. It is the same payment each time, so do not pay again.${tor}`);
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
      const saved = rules && rules.contacts.find((c) => c.address === plan.to);
      $("plan-to").textContent = (saved ? `${saved.name} · ` : "") + grouped(plan.to);
      $("plan-to").classList.add("grouped");
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
function askKeys(title, intro, { danger = false, action = "Continue", newCode = false, keys = true } = {}) {
  return new Promise((resolve, reject) => {
    $("confirm-title").textContent = title;
    $("confirm-intro").textContent = intro;
    for (const id of ["confirm-passphrase", "confirm-code", "confirm-new-code", "confirm-new-code-again"]) $(id).value = "";
    $("confirm-passphrase-field").hidden = !(keys && needsPassphrase());
    $("confirm-code-field").hidden = !(keys && needsCode());
    $("confirm-new-code-field").hidden = !newCode;
    $("confirm-new-code-again-field").hidden = !newCode;
    say($("confirm-message"), "");
    const go = $("confirm-go");
    go.textContent = action;
    go.classList.toggle("danger", danger);
    $("confirm").hidden = false;
    const first = [...$("confirm").querySelectorAll(".field:not([hidden]) input")][0];
    (first || $("confirm-cancel")).focus();
    const clear = () => { for (const id of ["confirm-passphrase", "confirm-code", "confirm-new-code", "confirm-new-code-again"]) $(id).value = ""; };
    const finish = (ok) => {
      if (ok && newCode) {
        const code = $("confirm-new-code").value;
        if (!/^\d{6}$/.test(code)) return say($("confirm-message"), "A Zecky code is six digits.");
        if (code !== $("confirm-new-code-again").value) return say($("confirm-message"), "The two codes are not the same.");
      }
      $("confirm").hidden = true;
      go.onclick = null;
      $("confirm-cancel").onclick = null;
      const typed = {
        passphrase: keys && needsPassphrase() ? $("confirm-passphrase").value : null,
        code: keys && needsCode() ? $("confirm-code").value : null,
        newCode: newCode ? $("confirm-new-code").value : null,
      };
      clear();
      if (ok) resolve(typed);
      else reject(new Error("cancelled"));
    };
    go.onclick = () => finish(true);
    $("confirm-cancel").onclick = () => finish(false);
  });
}

function typedOnly(t) { return { passphrase: t.passphrase, code: t.code }; }

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
  timelock: ["Time lock", "A wait of days before the lock opens."],
  contacts: ["Address book", "Addresses you send to often, picked by name."],
  phrase: ["Recovery phrase", "Your 24 words, sealed on this PC."],
  viewkey: ["Viewing key", "Read-only access to this wallet."],
  about: ["About", "Zecky Lock it for Windows."],
  how: ["How Zecky works", "Short chapters. Open the one you want."],
};
const PARENT = { keys: "security", timelock: "security", phrase: "security", viewkey: "security" };

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
  $("row-display").textContent = `${{ dark: "Dark", light: "Light", auto: "Same as this PC" }[themeChoice()]} · Value ${currency() === "none" ? "None" : currency().toUpperCase()}`;
  const keys = overview ? overview.keys.map((k) => KEY_NAMES[k.kind]) : [];
  $("row-security").textContent = keys.join(" + ");
  if (rules) {
    const n = rules.contacts.length;
    $("row-contacts").textContent = n === 0 ? "None yet" : `${n} saved${rules.only_saved ? " · only these" : ""}`;
    renderContacts();
  }
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
  for (const chip of document.querySelectorAll("[data-theme-choice]")) chip.classList.toggle("on", chip.dataset.themeChoice === themeChoice());
  for (const chip of document.querySelectorAll("[data-currency]")) chip.classList.toggle("on", chip.dataset.currency === currency());
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
    title.textContent = { device_hello: "This PC (Windows Hello)", security_key: "Security key", passphrase: "Passphrase", code: "Zecky code" }[key.kind];
    const state = document.createElement("span");
    state.className = "row-detail on";
    state.textContent = { device_hello: "Face, fingerprint or PIN", security_key: "Touch the key", passphrase: "Asked when opening", code: "Six digits" }[key.kind];
    row.append(n, title, state);
    list.append(row);
  });
  for (const chip of document.querySelectorAll("[data-wait]")) chip.classList.toggle("on", Number(chip.dataset.wait) === s.wait);
  if (rules) {
    $("row-timelock").textContent = hoursLabel(rules.hours);
    for (const chip of document.querySelectorAll("[data-hours]")) chip.classList.toggle("on", Number(chip.dataset.hours) === rules.hours);
    $("timelock-pending").hidden = !rules.pending_from;
    $("timelock-keep").hidden = !rules.pending_from;
    if (rules.pending_from) {
      $("timelock-pending").textContent = `Becomes ${hoursLabel(rules.pending_hours).toLowerCase()} on ${dateTime(rules.pending_from)}.`;
      $("timelock-keep").textContent = `Keep ${hoursLabel(rules.hours).toLowerCase()}`;
    }
    $("timelock-broken").hidden = !rules.last_broken;
    if (rules.last_broken) $("timelock-broken").textContent = `⚒ Glass last broken on ${dateTime(rules.last_broken)}`;
    $("ask-on-start").checked = rules.ask_on_start;
    $("ask-on-start-note").textContent = rules.start_with_hello
      ? "Windows Hello when Lock it starts, and after it locked itself (sleep, 5 minutes away). Why: your balance and history stay private on a shared or unattended PC."
      : "Your keys when Lock it starts, and after it locked itself (sleep, 5 minutes away). Why: your balance and history stay private on a shared or unattended PC.";
    renderSecurityKeys();
    $("code-set").textContent = rules.code ? "Change code" : "Set a Zecky code";
    $("code-remove").hidden = !rules.code;
  }
  // About
  $("about-version").textContent = settingsView ? settingsView.version : "…";
  $("about-name").textContent = `Lock it for Windows ${settingsView ? settingsView.version : ""}`;
  $("about-block").textContent = b.chain_tip ? numbers.format(b.chain_tip) : "…";
  $("about-birthday").textContent = b.birthday ? numbers.format(b.birthday) : "…";
  $("about-server").textContent = (overview && overview.sync.server) || "…";
  for (const pool of ["ironwood", "orchard", "sapling", "transparent"]) $(`pool-${pool}`).textContent = prefs.hidden ? HIDDEN : exact(b[pool] || 0);
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

for (const chip of document.querySelectorAll("[data-theme-choice]")) {
  chip.addEventListener("click", () => { localStorageSet("theme.mode", chip.dataset.themeChoice); applyTheme(); renderSettings(); });
}
for (const chip of document.querySelectorAll("[data-currency]")) {
  chip.addEventListener("click", () => { localStorageSet("value.currency", chip.dataset.currency); restartPrice(); renderSettings(); if (overview) renderHome(); });
}

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
      const typed = weaker ? await askKeys("Confirm it's you", "Making the pause shorter weakens a key, so your keys are asked first.") : {};
      await invoke("settings_set_wait", { seconds, typed: typedOnly(typed) });
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
    const typed = await askKeys("Confirm it's you", "Your 24 words are the wallet. Make sure nobody can see your screen.", { action: "Show the words" });
    const words = await invoke("wallet_show_words", { typed: typedOnly(typed) });
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
    const typed = await askKeys("Confirm it's you", "Whoever has the viewing key sees every payment, past and future.", { action: "Show the key" });
    $("viewkey-text").textContent = await invoke("wallet_viewing_key", { typed: typedOnly(typed) });
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
      { danger: true, action: "Erase everything", keys: false });
  } catch {
    return;
  }
  try {
    const typed = await askKeys("Confirm it's you", "Erasing asks your keys, whatever you chose.", { danger: true, action: "Erase" });
    await invoke("wallet_erase", { typed: typedOnly(typed) });
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
  rules = await invoke("lock_rules").catch(() => rules);
  openSheet("settings");
  goSettings("overview");
  refreshGoMaxRow();
});

async function refreshGoMaxRow() {
  try {
    const v = await invoke("go_max_view");
    $("row-gomax").textContent = v.state === "off" ? "Off" : v.state !== "on" ? (v.state === "starting" ? "Starting" : "Stopping")
      : v.status && nodeProgress(v.status).caughtUp ? "Yours, up to date"
      : v.status && nodeProgress(v.status).network > 0 ? `Catching up · ${Math.floor(nodeProgress(v.status).fraction * 100)}%` : "On";
  } catch { $("row-gomax").textContent = ""; }
}

async function refreshRules() {
  rules = await invoke("lock_rules").catch(() => rules);
  overview = await invoke("wallet_overview").catch(() => overview);
  renderSettings();
}

for (const chip of document.querySelectorAll("[data-hours]")) {
  chip.addEventListener("click", async () => {
    const hours = Number(chip.dataset.hours);
    if (!rules || hours === rules.hours) return;
    say($("timelock-note"), "");
    const longer = hours > rules.hours;
    const title = hours === 0 ? "Turn the time lock off?" : longer ? `Set a ${hoursLabel(hours).toLowerCase()} time lock?` : `Shorten the time lock to ${hoursLabel(hours).toLowerCase()}?`;
    const from = Date.now() / 1000 + rules.hours * 3600;
    const text = longer
      ? `From now on, opening the lock has to be asked ${hoursLabel(hours).toLowerCase()} ahead. Turning it off or shorter later also takes ${hoursLabel(hours).toLowerCase()}. In an emergency you can break the glass. The new rule is sealed into the lock, so your keys are asked.`
      : `This takes effect on ${dateTime(from)}, after the current ${hoursLabel(rules.hours).toLowerCase()}. Until then the time lock stays as it is. Your keys are asked.`;
    try {
      const typed = await askKeys(title, text, { action: hours === 0 ? "Turn off" : longer ? "Set time lock" : "Shorten" });
      await invoke("time_lock_choose", { hours, typed: typedOnly(typed) });
      await refreshRules();
    } catch (problem) {
      if (problem && problem.message !== "cancelled") say($("timelock-note"), describe(problem) || "Not confirmed, so nothing changed.");
    }
  });
}

$("timelock-keep").addEventListener("click", async () => {
  try {
    const typed = await askKeys("Keep the time lock?", `The ${hoursLabel(rules.hours).toLowerCase()} time lock stays, and the shorter one is dropped. Your keys are asked.`, { action: "Keep it" });
    await invoke("time_lock_keep", { typed: typedOnly(typed) });
    await refreshRules();
  } catch (problem) {
    if (problem && problem.message !== "cancelled") say($("timelock-note"), describe(problem));
  }
});

$("ask-on-start").addEventListener("change", async (event) => {
  const on = event.target.checked;
  say($("ask-on-start-message"), "");
  try {
    const typed = on ? {} : (rules && rules.start_with_hello ? {} : await askKeys("Confirm it's you", "Not asking when Lock it opens weakens a key, so you are asked first."));
    await invoke("set_ask_on_start", { on, typed: typedOnly(typed) });
    await refreshRules();
  } catch (problem) {
    event.target.checked = !on;
    if (problem && problem.message !== "cancelled") say($("ask-on-start-message"), describe(problem) || "Not confirmed, so nothing changed.");
  }
});

$("code-set").addEventListener("click", async () => {
  say($("code-note"), "");
  try {
    const typed = await askKeys(rules && rules.code ? "Change the code" : "Set a Zecky code",
      "Six digits, asked with your other keys every time the lock opens. Keep it somewhere safe: without it the lock does not open, though your 24 words still restore the wallet.",
      { action: rules && rules.code ? "Change code" : "Set code", newCode: true });
    await invoke("code_set", { newCode: typed.newCode, typed: typedOnly(typed) });
    say($("code-note"), "Done. The code is asked from now on.", true);
    await refreshRules();
  } catch (problem) {
    if (problem && problem.message !== "cancelled") say($("code-note"), describe(problem) || "Not confirmed, so nothing changed.");
  }
});

$("code-remove").addEventListener("click", async () => {
  say($("code-note"), "");
  try {
    const typed = await askKeys("Remove the code", "The lock opens with your other keys only. Your current code is asked one last time.", { action: "Remove code" });
    await invoke("code_remove", { typed: typedOnly(typed) });
    say($("code-note"), "Removed.", true);
    await refreshRules();
  } catch (problem) {
    if (problem && problem.message !== "cancelled") say($("code-note"), describe(problem) || "Not confirmed, so nothing changed.");
  }
});

// Security keys (Settings › Security › Keys to the lock)
function renderSecurityKeys() {
  const keys = rules.security_keys || [];
  const list = $("seckey-list");
  list.textContent = "";
  for (const key of keys) {
    const row = document.createElement("div");
    row.className = "seckey-row";
    const grow = document.createElement("span");
    grow.className = "grow";
    const name = document.createElement("span");
    name.textContent = key.role === "backup" ? "Backup key" : "Security key";
    const added = document.createElement("small");
    added.textContent = key.added ? `added ${new Date(key.added * 1000).toLocaleDateString(undefined, { dateStyle: "medium" })}` : "added in an earlier test build";
    grow.append(name, added);
    const remove = document.createElement("button");
    remove.type = "button";
    remove.textContent = "Remove";
    remove.addEventListener("click", () => securityKeyAction("seckey_remove", { id: key.id }, "Remove this key?",
      "The lock opens with every key, so one of your security keys is touched first, then the key is taken off the lock.", "Security key removed."));
    row.append(grow, remove);
    list.append(row);
  }
  const available = rules.security_keys_available;
  const removal = rules.removal;
  $("seckey-add").hidden = keys.length > 0 || !available;
  $("seckey-backup").hidden = keys.length === 0 || !available || !!removal;
  $("seckey-test").hidden = keys.length === 0 || !available;
  $("seckey-lost").hidden = keys.length === 0 || !!removal;
  $("seckey-removal").hidden = !removal;
  if (removal) {
    $("seckey-removal-text").textContent = removal.due
      ? "The 48 hours have passed. The keys are taken off the next time the lock opens, with your other keys."
      : `Not before ${dateTime(removal.clock_done_at)}${removal.blocks_to_go > 0 ? ` and ${numbers.format(removal.blocks_to_go)} more Zcash blocks` : ""}. Until then your keys still work. Found them? Cancel with one of them. It finishes only on one of the built-in servers.`;
  }
  if (!available && keys.length === 0) say($("seckey-note"), "This Windows cannot use security keys (it needs Windows 10 version 1903 or later).", true);
}

async function securityKeyAction(command, args, title, text, done, { askFirst = true } = {}) {
  say($("seckey-note"), "");
  try {
    const typed = askFirst ? await askKeys(title, text, { action: "Continue" }) : {};
    say($("seckey-note"), "Follow the prompts from Windows.", true);
    await invoke(command, { ...args, typed: typedOnly(typed) });
    say($("seckey-note"), done, true);
    await refreshRules();
  } catch (problem) {
    if (problem && problem.message !== "cancelled") say($("seckey-note"), describe(problem) || "Nothing changed.");
    else say($("seckey-note"), "");
  }
}

$("seckey-add").addEventListener("click", () => securityKeyAction("seckey_add", { backup: false }, "Add a security key",
  "Your keys open the lock first, then touch the new key when Windows asks. From then on it is touched every time the lock opens.", "Key added. It is asked every time the lock opens."));
$("seckey-backup").addEventListener("click", () => securityKeyAction("seckey_add", { backup: true }, "Add a backup key",
  "Your keys open the lock first (one of your security keys is touched), then touch the backup key when Windows asks. Keep it somewhere else.", "Backup key added."));
$("seckey-test").addEventListener("click", async () => {
  say($("seckey-note"), "Touch your key when Windows asks.", true);
  try {
    say($("seckey-note"), await invoke("seckey_test"), true);
  } catch (problem) {
    say($("seckey-note"), describe(problem) || "No key was used.");
  }
});
$("seckey-lost-button").addEventListener("click", async () => {
  try {
    await askKeys("Remove all security keys in 48 hours?",
      "After 48 hours, and 48 hours of Zcash blocks, every security key is removed and the lock opens with your other keys again. Any of your keys can cancel it before then. It finishes only on one of the built-in servers.",
      { action: "Start the 48 hours", danger: true, keys: false });
  } catch {
    return;
  }
  securityKeyAction("seckey_removal_start", {}, "Confirm it's you", "Your other keys (not a security key) confirm it's you.",
    "Started. All security keys are removed after 48 hours unless you cancel.");
});
$("seckey-removal-cancel").addEventListener("click", async () => {
  say($("seckey-note"), "Touch one of your keys when Windows asks.", true);
  try {
    await invoke("seckey_removal_cancel");
    say($("seckey-note"), "The removal is cancelled. Your keys stay.", true);
    await refreshRules();
  } catch (problem) {
    say($("seckey-note"), describe(problem) || "No key was used.");
  }
});

// Address book (Contacts.swift)
/// Long addresses in groups of four, so a swapped character stands out.
function grouped(address) { return address.replace(/(.{4})/g, "$1 ").trim(); }
function shortAddress(a) { return a.length > 20 ? `${a.slice(0, 10)}…${a.slice(-8)}` : a; }

let renaming = null;

function renderContacts() {
  const list = $("contacts-list");
  list.textContent = "";
  for (const contact of rules.contacts) {
    const row = document.createElement("div");
    row.className = "contact-row";
    const grow = document.createElement("span");
    grow.className = "grow";
    const name = document.createElement("span");
    name.textContent = contact.name;
    const addr = document.createElement("small");
    addr.textContent = shortAddress(contact.address);
    addr.title = contact.address;
    grow.append(name, addr);
    const rename = document.createElement("button");
    rename.type = "button";
    rename.textContent = "Rename";
    rename.addEventListener("click", () => openContactForm(contact));
    const remove = document.createElement("button");
    remove.type = "button";
    remove.className = "remove";
    remove.textContent = "Remove";
    remove.addEventListener("click", () => contactAction("contacts_remove", { address: contact.address },
      `Remove ${contact.name}?`, "The address is removed from the address book. Your keys are asked.", "Removed."));
    row.append(grow, rename, remove);
    list.append(row);
  }
  $("only-saved").checked = rules.only_saved;
  $("only-saved").disabled = rules.contacts.length === 0 && !rules.only_saved;
}

function openContactForm(contact = null) {
  renaming = contact;
  $("contact-form").hidden = false;
  $("contact-add").hidden = true;
  $("contact-name").value = contact ? contact.name : "";
  $("contact-address").value = contact ? contact.address : "";
  $("contact-address").disabled = !!contact;
  say($("contact-form-note"), "");
  $("contact-name").focus();
}

function closeContactForm() {
  renaming = null;
  $("contact-form").hidden = true;
  $("contact-add").hidden = false;
  $("contact-name").value = "";
  $("contact-address").value = "";
}

async function contactAction(command, args, title, text, done) {
  say($("contacts-note"), "");
  try {
    const typed = await askKeys(title, text, { action: "Continue" });
    say($("contacts-note"), "Follow the prompts from Windows.", true);
    await invoke(command, { ...args, typed: typedOnly(typed) });
    say($("contacts-note"), done, true);
    closeContactForm();
    await refreshRules();
    return true;
  } catch (problem) {
    if (problem && problem.message !== "cancelled") say($("contacts-note"), describe(problem) || "Nothing changed.");
    else say($("contacts-note"), "");
    return false;
  }
}

$("contact-add").addEventListener("click", () => openContactForm());
$("contact-cancel").addEventListener("click", closeContactForm);
$("contact-save").addEventListener("click", async () => {
  const name = $("contact-name").value.trim();
  const address = $("contact-address").value.trim();
  if (!name) return say($("contact-form-note"), "Give the address a name.");
  if (renaming) {
    await contactAction("contacts_rename", { address: renaming.address, name }, "Rename this address?", "Your keys are asked.", "Renamed.");
  } else {
    if (!address) return say($("contact-form-note"), "Paste the address.");
    await contactAction("contacts_add", { name, address }, `Save ${name}?`,
      `Check the address once more, in groups of four: ${grouped(address)}. Your keys are asked.`, "Saved.");
  }
});
$("only-saved").addEventListener("change", async (event) => {
  const on = event.target.checked;
  const ok = await contactAction("contacts_only_saved", { on },
    on ? "Only send to saved addresses?" : "Allow any address again?",
    on ? "From now on Lock it pays only addresses in your address book. Your keys are asked." : "Lock it can then pay any address again. Your keys are asked.",
    on ? "Only saved addresses can be used now." : "Any address can be used again.");
  if (!ok) event.target.checked = !on;
});

// Send: pick a saved address, or paste a payment request.
function renderSendSaved() {
  const box = $("send-saved");
  box.textContent = "";
  for (const contact of (rules && rules.contacts) || []) {
    const chip = document.createElement("button");
    chip.type = "button";
    chip.className = "chip";
    chip.textContent = contact.name;
    chip.title = contact.address;
    chip.addEventListener("click", () => { $("send-to").value = contact.address; noteSaved(); });
    box.append(chip);
  }
  noteSaved();
}

function noteSaved() {
  const to = $("send-to").value.trim();
  const saved = rules && rules.contacts.find((c) => c.address === to);
  for (const chip of $("send-saved").children) chip.classList.toggle("on", chip.title === to);
  $("send-saved-note").textContent = saved ? `✓ Saved as ${saved.name}`
    : rules && rules.only_saved ? "Only saved addresses can be used. Pick one above, or change the rule in Settings." : "";
}

$("send-to").addEventListener("input", async () => {
  const text = $("send-to").value.trim();
  if (/^zcash:/i.test(text)) {
    try {
      const request = await invoke("wallet_parse_request", { text });
      $("send-to").value = request.address;
      if (request.amount) $("send-amount").value = (request.amount / 1e8).toFixed(8).replace(/0+$/, "").replace(/\.$/, "");
      if (request.memo) $("send-memo").value = request.memo;
      say($("send-message"), "Filled in from the payment request. Check it before sending.", true);
    } catch (problem) {
      say($("send-message"), describe(problem));
    }
  }
  noteSaved();
});

// Activity › The lock
let activityTab = "payments";
$("tab-payments").addEventListener("click", () => { activityTab = "payments"; renderActivity(); });
$("tab-lock").addEventListener("click", () => { activityTab = "lock"; renderActivity(); });

async function renderLockLog() {
  const list = $("activity-list");
  const entries = await invoke("lock_log").catch(() => []);
  list.textContent = "";
  if (!entries.length) {
    const empty = document.createElement("li");
    empty.className = "empty";
    empty.textContent = "No changes to the lock yet.";
    list.append(empty);
    return;
  }
  for (const e of entries) {
    const line = document.createElement("li");
    const what = document.createElement("span");
    what.className = "what";
    what.textContent = e.text;
    const when = document.createElement("span");
    when.className = "when";
    when.textContent = new Date(e.at * 1000).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" });
    line.append(what, when);
    list.append(line);
  }
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
    "The keys are chosen when the wallet is made: This PC (Windows Hello), a passphrase, and a security key. Every key you chose is needed. A six-digit Zecky code can be added on top.",
    "Really open the lock? adds a pause with a reminder. Showing your 24 words, the viewing key and erasing always ask your keys. Making a key stricter goes straight through; weakening one asks you first.",
  ], [
    "Windows Hello keeps its key in this PC's security chip (TPM) and asks for your face, fingerprint or PIN.",
    "A security key, such as a YubiKey, signs a fresh challenge each time and Lock it checks the signature itself; nothing goes over the internet. It always comes together with Windows Hello or a passphrase.",
    "With a time lock, opening has to be asked for 24 hours, 3 or 7 days ahead. In an emergency, Break the glass opens straight away and stays on record. The time lock is sealed inside the lock itself, so it cannot be removed by editing files. It does not stop someone who has your 24 words.",
    "Every change is listed in Activity › The lock.",
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

function setSwitch(on) {
  nodeToggle.classList.toggle("on", on);
  $("gm-label").textContent = on ? "GO MAX IS ON" : "SWITCH ON";
  $("gm-glyph").innerHTML = GLYPHS[on ? "power" : "node"];
}

function renderGoMax(view) {
  lastView = view;
  nodeMessage.textContent = "";
  nodeToggle.disabled = false;
  setSwitch(view.state === "on" || view.state === "starting");
  if (view.state === "off") {
    setRing("off", 0);
    nodePercent.textContent = "";
    nodeState.textContent = "Off";
    nodeLead.textContent = LEAD_OFF;
    nodeFacts.hidden = true;
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
    nodeToggle.disabled = true;
    return;
  }
  const status = view.status;
  nodeFacts.hidden = !status;
  if (!status) {
    setRing("on", 0);
    nodePercent.textContent = "";
    nodeState.textContent = "On";
    nodeLead.textContent = "The node is not answering right now.";
    return;
  }
  const { network: netHeight, fraction: f, caughtUp } = nodeProgress(status);
  const fraction = netHeight > 0 ? f : status.progress;
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
    nodeLead.textContent = "Checking the blocks since the beginning. You can keep using this PC; Lock it reads from zec.rocks until the node has caught up.";
  }
  const network = netHeight > 0 ? ` of ${numbers.format(netHeight)}` : "";
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

const GLYPHS = {
  node: '<circle class="solid" cx="12" cy="5" r="2.6"/><circle class="solid" cx="5" cy="18" r="2.6"/><circle class="solid" cx="19" cy="18" r="2.6"/><path d="M12 5 5 18h14z"/>',
  seal: '<path d="M12 2.5l2.4 1.8 3-.2.9 2.8 2.5 1.7-1 2.9 1 2.8-2.5 1.7-.9 2.9-3-.2L12 21.5l-2.4-1.8-3 .2-.9-2.9-2.5-1.7 1-2.8-1-2.9 2.5-1.7.9-2.8 3 .2z"/><path d="m8.5 12 2.3 2.3 4.7-4.7"/>',
  warn: '<path d="M12 6v8"/><circle class="solid" cx="12" cy="18" r="1.4"/>',
  power: '<path d="M12 3v8"/><path d="M6.6 6.6a7.5 7.5 0 1 0 10.8 0"/>',
};

/// The network's height: the wallet's server knows it best; the node's own
/// estimate runs ahead while it is far behind.
function networkHeight(status) {
  const known = overview && overview.balance && overview.balance.chain_tip;
  return known > 0 ? known : status && status.estimated_height > 0 ? status.estimated_height : 0;
}

function nodeProgress(status) {
  const network = networkHeight(status);
  if (!status || network <= 0) return { network, fraction: 0, caughtUp: false };
  const caughtUp = status.height + 10 >= network;
  return { network, fraction: Math.min(1, status.height / network), caughtUp };
}

async function refreshCoin() {
  let view;
  try { view = await invoke("go_max_view"); } catch { return; }
  const button = $("go-max-coin");
  const coin = $("coin");
  const sub = $("coin-sub");
  const badge = $("coin-badge");
  let glyph = "node";
  let text = "Run your own node";
  let tone = "";
  let checking = false;
  let fraction = 0;
  badge.hidden = true;
  badge.className = "coin-badge";
  if (view.state === "on" && view.status) {
    const { fraction: f, caughtUp } = nodeProgress(view.status);
    fraction = f;
    if (caughtUp && view.wallet_ready) {
      glyph = "seal"; text = "Your own node"; tone = "green"; badge.hidden = false;
    } else if (view.status.peers === 0) {
      text = "Not connected · check connection"; tone = "gold"; checking = true; badge.hidden = false; badge.classList.add("gold");
    } else {
      text = caughtUp ? "Almost there" : `Checking · ${Math.floor(f * 100)}%`; checking = true;
    }
  } else if (view.state === "on") {
    text = "Starting"; checking = true;
  } else if (view.state === "starting") {
    text = "Starting"; checking = true;
  } else if (view.state === "stopping") {
    text = "Stopping";
  } else if (view.problem) {
    glyph = "warn"; text = "Stopped · open to see why"; tone = "red"; badge.hidden = false; badge.classList.add("red");
  }
  $("coin-glyph").innerHTML = GLYPHS[glyph];
  sub.textContent = text;
  sub.className = `coin-sub ${tone}`;
  coin.classList.toggle("checking", checking);
  $("coin-ring").setAttribute("stroke-dasharray", `${Math.max(3, fraction * 100)} 100`);
  button.classList.toggle("inviting", view.state === "off" && !view.problem);
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

// ---------- Before you start, and the tour (as on Lock it Mac) ----------

let firstRun = { terms_accepted: false, tour_seen: true };

$("terms-read").addEventListener("click", () => invoke("open_page", { page: "terms" }).catch(() => {}));
$("terms-privacy").addEventListener("click", () => invoke("open_page", { page: "privacy" }).catch(() => {}));
$("terms-accept").addEventListener("click", async () => {
  try {
    await invoke("terms_accept");
    firstRun.terms_accepted = true;
    await start();
  } catch (problem) {
    alert(describe(problem));
  }
});

const TOUR = [
  ["Receive", "Click Receive for your address. Zcash sent to it goes straight into the lock."],
  ["Click the lock to send", "The lock opens with a deliberate click. Send takes out only what you choose; the rest stays locked. It closes again by itself."],
  ["Go Max, if you want", "The gold coin at the bottom right starts your own Zcash node inside Lock it, so Lock it reads the chain from this PC instead of a server. Optional: it needs about 35 GB and most of a day the first time. Lock it works the same without it."],
  ["Choose your keys", "Decide what is asked before the lock opens: Windows Hello, a Zecky code, a short pause, a security key. Settings › Security › Keys to the lock."],
];
let tourStep = 0;

function showTourStep() {
  const [title, text] = TOUR[tourStep];
  const last = tourStep === TOUR.length - 1;
  $("tour-step").textContent = `${tourStep + 1} OF ${TOUR.length}`;
  $("tour-title").textContent = title;
  $("tour-text").textContent = text;
  $("tour-next").textContent = last ? "Done" : "Next";
  $("tour-keys").hidden = !last;
}

function openTour() {
  tourStep = 0;
  showTourStep();
  $("tour").hidden = false;
  $("tour-next").focus();
}

function finishTour() {
  $("tour").hidden = true;
  firstRun.tour_seen = true;
  invoke("tour_seen", { seen: true }).catch(() => {});
}

function maybeShowTour() {
  // After the start (and Lock it's own lock, when it is asked) settles.
  setTimeout(() => {
    if (!firstRun.tour_seen && !appLocked && !$("view-home").hidden && $("tour").hidden) openTour();
  }, 700);
}

$("tour-skip").addEventListener("click", finishTour);
$("tour-next").addEventListener("click", () => {
  if (tourStep === TOUR.length - 1) return finishTour();
  tourStep += 1;
  showTourStep();
});
$("tour-keys").addEventListener("click", async () => {
  finishTour();
  $("open-settings").click();
  setTimeout(() => goSettings("keys"), 50);
});
$("tour-again").addEventListener("click", () => { closeSheet(); openTour(); });
document.addEventListener("keydown", (event) => {
  if ($("tour").hidden) return;
  if (event.key === "Escape") finishTour();
  if (event.key === "Enter") $("tour-next").click();
});

// ---------- Start ----------

async function start() {
  const first = await invoke("wallet_overview");
  firstRun = await invoke("first_run").catch(() => firstRun);
  // Everyone accepts once, existing wallets too (an upgrade from a test build).
  if (!firstRun.terms_accepted) return showView("terms");
  showView(first.exists ? "home" : "welcome");
  if (first.exists) {
    overview = first;
    rules = await invoke("lock_rules").catch(() => null);
    if (rules && rules.ask_on_start) showAppLocked();
  }
}

start().catch((problem) => {
  showView("loading");
  views.loading.querySelector("p").textContent = describe(problem) || "The app could not start.";
});
