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
};

let lockKeys = [];
let busy = false;

function show(name) {
  for (const [key, element] of Object.entries(views)) {
    element.hidden = key !== name;
  }
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
    security_key: "Needs an up-to-date Windows",
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
