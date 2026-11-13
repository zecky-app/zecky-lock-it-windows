// Zecky Lock it: the window. It draws and asks; it never holds a key.
// The 24 words pass through here once, to be written down (or typed in for a
// restore), and a passphrase on its way to the Rust side. Fields are cleared
// as soon as they are sent.

"use strict";

const invoke = window.__TAURI__.core.invoke;

const KEY_NAMES = {
  device_hello: "This PC",
  security_key: "Security key",
  passphrase: "Passphrase",
};

const MIN_PASSPHRASE = 12;

const $ = (id) => document.getElementById(id);

const views = {
  loading: $("view-loading"),
  welcome: $("view-welcome"),
  words: $("view-words"),
  check: $("view-check"),
  restore: $("view-restore"),
  setup: $("view-setup"),
  home: $("view-home"),
  receive: $("view-receive"),
  send: $("view-send"),
  goMax: $("view-go-max"),
};

let busy = false;
let current = "loading";

function show(name) {
  current = name;
  for (const [key, element] of Object.entries(views)) {
    element.hidden = key !== name;
  }
  if (name === "goMax") startWatching();
  else stopWatching();
  if (name === "home") startHome();
  else stopHome();
  window.scrollTo(0, 0);
}

function say(element, text, calm = false) {
  element.textContent = text;
  element.classList.toggle("calm", calm);
}

function describe(problem) {
  if (problem && typeof problem === "object" && "message" in problem) {
    return problem.cancelled ? "" : problem.message;
  }
  return String(problem);
}

const numbers = new Intl.NumberFormat("en-US");

function zec(zats) {
  const whole = Math.floor(zats / 1e8);
  const fraction = String(zats % 1e8).padStart(8, "0").replace(/0+$/, "");
  return `${numbers.format(whole)}${fraction ? "." + fraction : ""} ZEC`;
}

// ---------- Creating: the words ----------

let words = [];
let madeIn = null;

$("welcome-create").addEventListener("click", async () => {
  try {
    words = await invoke("wallet_new_words");
    madeIn = null;
    const list = $("words-list");
    list.textContent = "";
    for (const word of words) {
      const item = document.createElement("li");
      item.textContent = word;
      list.append(item);
    }
    show("words");
  } catch (problem) {
    alertProblem(problem);
  }
});

function forgetWords() {
  words = [];
  $("words-list").textContent = "";
  $("check-form").textContent = "";
  $("restore-words").value = "";
  invoke("wallet_forget_words");
}

$("words-back").addEventListener("click", () => {
  forgetWords();
  show("welcome");
});

let checkPositions = [];

$("words-done").addEventListener("click", () => {
  const positions = new Set();
  while (positions.size < 3) positions.add(Math.floor(Math.random() * 24));
  checkPositions = [...positions].sort((a, b) => a - b);
  const form = $("check-form");
  form.textContent = "";
  for (const position of checkPositions) {
    const label = document.createElement("label");
    label.className = "field";
    const span = document.createElement("span");
    span.textContent = `Word ${position + 1}`;
    const input = document.createElement("input");
    input.spellcheck = false;
    input.autocomplete = "off";
    input.dataset.position = position;
    label.append(span, input);
    form.append(label);
  }
  // The words leave the screen while checking.
  $("words-list").textContent = "";
  say($("check-message"), "");
  show("check");
  form.querySelector("input").focus();
});

$("check-back").addEventListener("click", () => {
  const list = $("words-list");
  for (const word of words) {
    const item = document.createElement("li");
    item.textContent = word;
    list.append(item);
  }
  show("words");
});

$("check-submit").addEventListener("click", () => {
  const inputs = [...$("check-form").querySelectorAll("input")];
  const right = inputs.every((input) => input.value.trim().toLowerCase() === words[Number(input.dataset.position)]);
  if (!right) {
    say($("check-message"), "Not quite. Look at your paper again.");
    return;
  }
  for (const input of inputs) input.value = "";
  words = [];
  openSetup();
});

// ---------- Restoring ----------

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
  show("restore");
  $("restore-words").focus();
});

$("restore-back").addEventListener("click", () => {
  forgetWords();
  show("welcome");
});

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

// ---------- Choosing the keys ----------

const setupForm = $("setup-form");
const setupSubmit = $("setup-submit");
const setupMessage = $("setup-message");
const setupPassphrase = $("setup-passphrase");
const newPassphrase = $("new-passphrase");
const newPassphraseAgain = $("new-passphrase-again");

function chosenKinds() {
  return [...setupForm.querySelectorAll('input[name="kind"]:checked')].map((box) => box.value);
}

function refreshSetup() {
  const kinds = chosenKinds();
  setupPassphrase.hidden = !kinds.includes("passphrase");
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
  const reasons = {
    device_hello: "Needs Windows Hello set up on this PC",
    security_key: "Needs Windows 10 version 1903 or later",
  };
  for (const row of setupForm.querySelectorAll(".key")) {
    const kind = row.dataset.kind;
    const box = row.querySelector("input");
    box.disabled = !available[kind];
    if (!available[kind]) row.querySelector("[data-note]").textContent = reasons[kind] || "Not available here";
  }
  refreshSetup();
  show("setup");
}

setupForm.addEventListener("change", refreshSetup);

$("setup-back").addEventListener("click", () => {
  forgetWords();
  show("welcome");
});

setupForm.addEventListener("submit", async (event) => {
  event.preventDefault();
  if (busy) return;

  const kinds = chosenKinds();
  let passphrase = null;
  if (kinds.includes("passphrase")) {
    if ([...newPassphrase.value].length < MIN_PASSPHRASE) {
      say(setupMessage, `The passphrase needs at least ${MIN_PASSPHRASE} characters.`);
      return;
    }
    if (newPassphrase.value !== newPassphraseAgain.value) {
      say(setupMessage, "The two passphrases are not the same.");
      return;
    }
    passphrase = newPassphrase.value;
  }

  const keys = kinds.map((kind) => ({
    kind,
    label: KEY_NAMES[kind],
    passphrase: kind === "passphrase" ? passphrase : null,
  }));

  busy = true;
  refreshSetup();
  say(setupMessage, "Asking the network where to start, then each key in turn. Follow the prompts from Windows.", true);
  try {
    await invoke("wallet_create", { keys, madeIn });
    newPassphrase.value = "";
    newPassphraseAgain.value = "";
    madeIn = null;
    say(setupMessage, "");
    show("home");
  } catch (problem) {
    say(setupMessage, describe(problem) || "Nothing was set up.");
  } finally {
    passphrase = null;
    busy = false;
    refreshSetup();
  }
});

// ---------- Home ----------

let homeTimer = null;
let lockKeys = [];
let unsentWaiting = false;

const SYNC_WORDS = {
  syncing: "Checking new blocks",
  up_to_date: "Up to date",
  problem: "Could not reach the server",
  starting: "Starting",
};

async function refreshHome() {
  try {
    const overview = await invoke("wallet_overview");
    if (!overview.exists) return start();
    lockKeys = overview.keys;
    unsentWaiting = overview.unsent;
    const balance = overview.balance;
    if (balance) {
      $("home-balance").textContent = zec(balance.total);
      $("home-pending").textContent =
        balance.pending > 0 ? `${zec(balance.spendable)} can be sent now, ${zec(balance.pending)} waiting for confirmations` : "";
      const percent = balance.chain_tip > 0 && balance.progress < 0.999 ? ` · ${Math.floor(balance.progress * 100)}% checked` : "";
      const sync = overview.sync;
      const words = SYNC_WORDS[sync.state] || "Starting";
      $("home-sync").textContent = `${words}${percent}${sync.server ? " · from " + sync.server : ""}`;
      $("home-sync").title = sync.problem || "";
    }
    $("home-keys").textContent = `Opens with: ${lockKeys.map((key) => key.label || KEY_NAMES[key.kind]).join(" · ")}`;
    if (unsentWaiting) say($("home-message"), "A signed payment is waiting to be handed to the network. Open Send to try again.");
    refreshActivity();
  } catch (problem) {
    say($("home-message"), describe(problem));
  }
}

async function refreshActivity() {
  const list = $("home-activity");
  const items = await invoke("wallet_activity").catch(() => []);
  list.textContent = "";
  if (items.length === 0) {
    const empty = document.createElement("li");
    empty.className = "quiet";
    empty.textContent = "Nothing yet.";
    list.append(empty);
    return;
  }
  for (const item of items) {
    const line = document.createElement("li");
    const what = document.createElement("span");
    const when = document.createElement("span");
    when.className = "quiet";
    const amount = document.createElement("span");
    amount.className = item.change < 0 ? "out" : "in";
    what.textContent = item.change < 0 ? "Sent" : "Received";
    when.textContent = item.expired ? "did not go through" : item.height ? new Date((item.time || 0) * 1000).toLocaleDateString() : "waiting";
    amount.textContent = `${item.change < 0 ? "−" : "+"}${zec(Math.abs(item.change))}`;
    line.title = item.txid;
    line.append(what, when, amount);
    list.append(line);
  }
}

function startHome() {
  if (homeTimer) return;
  refreshHome();
  homeTimer = setInterval(refreshHome, 5000);
}

function stopHome() {
  clearInterval(homeTimer);
  homeTimer = null;
}

$("home-receive").addEventListener("click", async () => {
  show("receive");
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

for (const button of document.querySelectorAll("[data-home]")) {
  button.addEventListener("click", () => show("home"));
}

// ---------- Send ----------

let sendStage = "form"; // form, plan, confirm, unsent, done
const sendMessage = $("send-message");
const sendNext = $("send-next");

function sendLayout() {
  const form = sendStage === "form";
  for (const input of $("send-form").querySelectorAll("input")) input.disabled = !form;
  $("send-plan").hidden = !(sendStage === "plan" || sendStage === "confirm");
  $("send-confirm").hidden = sendStage !== "confirm";
  $("send-cancel").textContent = sendStage === "done" ? "Back home" : "Cancel";
  sendNext.hidden = sendStage === "done";
  sendNext.textContent = { form: "Work it out", plan: "Send", confirm: "Open the lock and send", unsent: "Try again" }[sendStage] || "";
  const needsPassphrase = lockKeys.some((key) => key.kind === "passphrase");
  $("send-passphrase-field").hidden = !(sendStage === "confirm" && needsPassphrase);
}

$("home-send").addEventListener("click", () => {
  $("send-form").reset();
  $("send-passphrase").value = "";
  sendStage = unsentWaiting ? "unsent" : "form";
  say(sendMessage, unsentWaiting ? "A signed payment is waiting. Try again, or send it without Tor below." : "");
  sendLayout();
  show("send");
  if (!unsentWaiting) $("send-to").focus();
});

$("send-cancel").addEventListener("click", () => {
  $("send-passphrase").value = "";
  show("home");
});

function sent(result) {
  sendStage = "done";
  sendLayout();
  const via = result.accepted_by.length ? ` Accepted by ${result.accepted_by.join(" and ")}.` : "";
  say(sendMessage, `Sent.${via} It counts once it is in a block, in about a minute.`, true);
  sendMessage.title = result.txids.join(" ");
}

function notSent(problem) {
  if (problem && problem.unsent) {
    sendStage = "unsent";
    sendLayout();
    say(sendMessage, `${describe(problem)}. Nothing was paid twice: "Try again" sends the same payment.`);
    offerWithoutTor();
  } else {
    say(sendMessage, describe(problem) || "Kept locked. Nothing was sent.", !describe(problem));
  }
}

function offerWithoutTor() {
  if ($("send-plain")) return;
  const plain = document.createElement("button");
  plain.type = "button";
  plain.className = "link";
  plain.id = "send-plain";
  plain.textContent = "Send without Tor (the server sees your internet address)";
  plain.addEventListener("click", () => sendAgain(false));
  $("send-buttons").after(plain);
}

async function sendAgain(overTor) {
  if (busy) return;
  busy = true;
  sendNext.disabled = true;
  say(sendMessage, overTor ? "Connecting over Tor. This can take a minute." : "Sending.", true);
  try {
    sent(await invoke("wallet_send_again", { overTor }));
    $("send-plain")?.remove();
  } catch (problem) {
    notSent(problem);
  } finally {
    busy = false;
    sendNext.disabled = false;
  }
}

sendNext.addEventListener("click", async () => {
  if (busy) return;
  if (sendStage === "unsent") return sendAgain($("send-tor").checked);
  if (sendStage === "plan") {
    sendStage = "confirm";
    sendLayout();
    say(sendMessage, "");
    if (!$("send-passphrase-field").hidden) $("send-passphrase").focus();
    return;
  }
  busy = true;
  sendNext.disabled = true;
  try {
    if (sendStage === "form") {
      say(sendMessage, "Working it out.", true);
      const plan = await invoke("wallet_prepare", {
        to: $("send-to").value,
        amount: $("send-amount").value,
        memo: $("send-memo").value || null,
      });
      $("plan-amount").textContent = zec(plan.amount);
      $("plan-fee").textContent = zec(plan.fee);
      $("plan-total").textContent = zec(plan.total);
      sendStage = "plan";
      sendLayout();
      say(sendMessage, "");
    } else if (sendStage === "confirm") {
      const needsPassphrase = lockKeys.some((key) => key.kind === "passphrase");
      const passphrase = needsPassphrase ? $("send-passphrase").value : null;
      $("send-passphrase").value = "";
      const overTor = $("send-tor").checked;
      say(sendMessage, `Follow the prompts from Windows, then signing${overTor ? " and connecting over Tor" : ""}. This can take a minute.`, true);
      sent(await invoke("wallet_send", { passphrase, overTor }));
    }
  } catch (problem) {
    if (sendStage === "confirm" && !(problem && problem.unsent)) {
      sendStage = "plan";
      sendLayout();
    }
    notSent(problem);
  } finally {
    busy = false;
    sendNext.disabled = false;
  }
});

function alertProblem(problem) {
  const text = describe(problem);
  if (text) say($("home-message"), text);
}

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


function gigabytes(bytes) {
  return bytes > 0 ? `${(bytes / 1e9).toFixed(1)} GB` : "Nothing yet";
}

function setRing(state, fraction) {
  nodeRing.dataset.state = state;
  const percent = Math.max(0, Math.min(100, fraction * 100));
  ringProgress.setAttribute("stroke-dasharray", `${percent} 100`);
}

function render(view) {
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
    if (!view.installed) {
      nodeMessage.textContent = "This build has no node programs inside.";
      nodeMessage.classList.add("calm");
    } else if (view.problem) {
      nodeMessage.textContent = view.problem;
      nodeMessage.classList.remove("calm");
    }
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

  // Running.
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
  $("fact-wallet").textContent = view.wallet_ready ? "Ready" : (view.wallet_note ? "Waiting for the node" : "Waiting for blocks");
  $("fact-wallet").title = view.wallet_note || "";
  $("fact-disk").textContent = gigabytes(status.size_on_disk);

  const ONION_WORDS = {
    starting: "Connecting to Tor",
    reachable: "Ready",
    degraded: "Ready, partly",
    unreachable: "Not reachable right now",
  };
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
    render(await invoke("go_max_view"));
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

if (window.__TAURI__.event) {
  window.__TAURI__.event.listen("go-max", () => {
    if (!views.goMax.hidden) refreshGoMax();
  });
}

nodeToggle.addEventListener("click", async () => {
  nodeToggle.disabled = true;
  const command = lastView && lastView.state === "on" ? "go_max_stop" : "go_max_start";
  try {
    await invoke(command);
  } catch (problem) {
    nodeMessage.textContent = describe(problem);
    nodeMessage.classList.remove("calm");
  }
  refreshGoMax();
});

for (const button of document.querySelectorAll("[data-go-max]")) {
  button.addEventListener("click", () => show("goMax"));
}
$("go-max-back").addEventListener("click", () => start());

// ---------- Start ----------

async function start() {
  const overview = await invoke("wallet_overview");
  show(overview.exists ? "home" : "welcome");
}

start().catch((problem) => {
  show("loading");
  views.loading.querySelector("p").textContent = describe(problem) || "The app could not start.";
});
