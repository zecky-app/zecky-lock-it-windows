// Zecky Lock it: the window. It draws and asks; it never holds a secret.
// The only sensitive thing that passes through here is a passphrase on its
// way to the Rust side, and the fields are cleared as soon as it is sent.

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
  setup: $("view-setup"),
  locked: $("view-locked"),
  open: $("view-open"),
  goMax: $("view-go-max"),
};

let lockKeys = [];
let busy = false;

function show(name) {
  for (const [key, element] of Object.entries(views)) {
    element.hidden = key !== name;
  }
  if (name === "goMax") startWatching();
  else stopWatching();
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

// ---------- Setting up ----------

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
  setupSubmit.disabled = busy || kinds.length === 0;
}

async function prepareSetup() {
  const available = await invoke("availability");
  const reasons = {
    device_hello: "Needs Windows Hello set up on this PC",
    security_key: "Needs Windows 11",
  };
  for (const row of setupForm.querySelectorAll(".key")) {
    const kind = row.dataset.kind;
    const box = row.querySelector("input");
    if (!available[kind]) {
      box.checked = false;
      box.disabled = true;
      row.querySelector("[data-note]").textContent = reasons[kind] || "Not available here";
    }
  }
  refreshSetup();
}

setupForm.addEventListener("change", refreshSetup);

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
  say(setupMessage, "Follow the prompts from Windows. Each key is asked in turn.", true);
  try {
    const opened = await invoke("create_practice_lock", { keys });
    newPassphrase.value = "";
    newPassphraseAgain.value = "";
    say(setupMessage, "");
    showOpen(opened.fingerprint);
    await refreshState(false);
  } catch (problem) {
    say(setupMessage, describe(problem) || "Nothing was set up.");
  } finally {
    passphrase = null;
    busy = false;
    refreshSetup();
  }
});

// ---------- Locked ----------

const lockedKeys = $("locked-keys");
const lockedIdle = $("locked-idle");
const lockedConfirm = $("locked-confirm");
const lockedMessage = $("locked-message");
const openPassphraseField = $("open-passphrase-field");
const openPassphrase = $("open-passphrase");
const openConfirm = $("open-confirm");

function showLocked() {
  const names = lockKeys.map((key) => key.label || KEY_NAMES[key.kind]);
  lockedKeys.textContent =
    names.length === 1 ? `Opens with: ${names[0]}` : `Opens with all of: ${names.join(" · ")}`;
  lockedIdle.hidden = false;
  lockedConfirm.hidden = true;
  openPassphrase.value = "";
  say(lockedMessage, "");
  show("locked");
}

$("open-ask").addEventListener("click", () => {
  lockedIdle.hidden = true;
  lockedConfirm.hidden = false;
  const needsPassphrase = lockKeys.some((key) => key.kind === "passphrase");
  openPassphraseField.hidden = !needsPassphrase;
  if (needsPassphrase) openPassphrase.focus();
  else $("open-cancel").focus();
});

$("open-cancel").addEventListener("click", showLocked);

lockedConfirm.addEventListener("submit", async (event) => {
  event.preventDefault();
  if (busy) return;
  const needsPassphrase = lockKeys.some((key) => key.kind === "passphrase");
  const passphrase = needsPassphrase ? openPassphrase.value : null;
  openPassphrase.value = "";

  busy = true;
  openConfirm.disabled = true;
  say(lockedMessage, "Follow the prompts from Windows.", true);
  try {
    const opened = await invoke("open_lock", { passphrase });
    showOpen(opened.fingerprint);
  } catch (problem) {
    showLocked();
    say(lockedMessage, describe(problem) || "Kept locked.", !describe(problem));
  } finally {
    busy = false;
    openConfirm.disabled = false;
  }
});

$("remove-lock").addEventListener("click", async () => {
  if (busy) return;
  try {
    await invoke("remove_practice_lock");
    await refreshState();
  } catch (problem) {
    say(lockedMessage, describe(problem));
  }
});

// ---------- Open ----------

function showOpen(fingerprint) {
  $("open-fingerprint").textContent = fingerprint;
  show("open");
}

$("lock-again").addEventListener("click", () => refreshState());

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

const numbers = new Intl.NumberFormat("en-US");

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
  $("fact-wallet").textContent = view.wallet_ready ? "Ready" : "Waiting for blocks";
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

$("go-max-open").addEventListener("click", () => show("goMax"));
$("go-max-back").addEventListener("click", showLocked);

// ---------- Start ----------

async function refreshState(navigate = true) {
  const state = await invoke("lock_state");
  lockKeys = state.keys;
  if (!navigate) return;
  if (state.exists) {
    showLocked();
  } else {
    setupForm.reset();
    await prepareSetup();
    show("setup");
  }
}

refreshState().catch((problem) => {
  show("loading");
  views.loading.querySelector("p").textContent = describe(problem) || "The app could not start.";
});
