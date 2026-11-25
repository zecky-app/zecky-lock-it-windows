// A stand-in for the app's backend, for the UI tests. Each test sets
// window.__m before the page loads (see build6_test.py).
window.__calls = [];
(function () {
  const m = Object.assign({
    wallets: [
      { id: "main", name: "Lock it", birthday: 3100000, watch_only: false, main: true },
      { id: "sav-2", name: "Savings", birthday: 3500000, watch_only: false, main: false },
    ],
    current: "sav-2", hidden: [], lockOpen: false, goMax: { state: "off", installed: true, problem: null, stopped_while_starting: false, retrying: false, has_data: true, support: { state: { state: "fine" }, text: null, ask: false, choice: null, date: null } },
  }, window.__m || {});
  window.__m = m;
  const overview = () => ({
    exists: true, keys: [{ kind: "device_hello", label: "This PC" }],
    balance: { spendable: 2240000, pending: 0, total: 2240000, scanned_height: m.scanned || 3600000, chain_tip: 3600000, progress: 1, birthday: 3100000, scan_from: m.scanFrom || 3100000, to_scan: m.toScan || [], orchard: m.orchard || 0, ironwood: 0, sapling: 0, transparent: m.transparent || 0 },
    sync: { state: m.syncing ? "syncing" : "up_to_date", server: "zec.rocks", problem: null, older: Boolean(m.older) }, unsent: false,
    pace: m.pace || { open: 0, minutes: null, newest_checked: true, own_checked: true }, lock_open: m.lockOpen, problems: [],
    settings: { server: null, tor: true, wait: 0, ask_on_start: false, current: m.current, names: {}, order: [], hidden: m.hidden },
    wallets: m.wallets,
  });
  window.__TAURI__ = { event: { listen: async () => () => {} }, core: { invoke: async (cmd, args) => {
    window.__calls.push([cmd, args]);
    switch (cmd) {
      case "wallet_overview": return overview();
      case "first_run": return { terms_accepted: true, tour_seen: true };
      case "lock_rules": return { phase: { phase: "none" }, ask_on_start: false, contacts: [], only_saved: false, security_keys: [], log: [], time_lock_hours: 0 };
      case "wallet_activity": return [];
      case "wallet_receive": return { address: "u1shieldedexample", qr: "<svg xmlns='http://www.w3.org/2000/svg'/>" };
      // Scans: every read is kept with its time, for the camera tests.
      case "sealit_scan_start": m.scanned = []; return null;
      case "sealit_scan_push": (m.scanned = m.scanned || []).push([performance.now(), args.text]); return { done: args.text.startsWith("uview"), seen: 0, of: 1, cupcake: false };
      case "wallet_receive_transparent": return { address: (m.tAddresses && m.tAddresses[m.current || "main"]) || "t1TransparentExampleAddress0000000", qr: "<svg xmlns='http://www.w3.org/2000/svg'/>" };
      // Watch a wallet: the Rust side's checks and words.
      case "watch_scanned_key": return m.scannedKey || "uview1scanned";
      case "watch_add": {
        const key = (args.key || "").trim();
        if (key.toLowerCase().startsWith("uviewtest")) throw { message: "That is a testnet viewing key. Lock it works on mainnet (uview1…)." };
        if (!(args.name || "").trim()) throw { message: "Give the wallet a name, for example Shop." };
        if (!key.startsWith("uview1")) throw { message: "That is not a Zcash viewing key. It starts with uview1…; in Lock it: Settings › Viewing key." };
        if (key === m.knownKey) throw { message: "That wallet is already in Lock it." };
        m.lastWatch = args;
        m.wallets = (m.wallets || []).concat([{ id: "watch-1", name: args.name.trim(), birthday: 1687104, watch_only: false, view_only: true, main: false }]);
        m.current = "watch-1";
        return "watch-1";
      }
      case "wallet_take_arrival": return (m.arrivals || []).shift() || null;
      case "sealit_start": return { frames: ["<svg xmlns='http://www.w3.org/2000/svg'/>"], cupcake: Boolean(m.cupcakeSigner), large_kb: m.largeKb || null };
      case "go_max_view": return m.goMax;
      case "settings_get": return { settings: { tor: true, wait: 0 }, reading_from: "zec.rocks", version: "0.1.5" };
      case "wallet_prepare": return { to: args.to, amount: 1000000, fee: 15000, total: 1015000 };
      case "keyring_layout": m.hidden = args.hidden; return null;
      case "keyring_select": m.current = args.id; return null;
      case "price_get": return { usd: 40, eur: 37 };
      case "go_max_prefs": return m.prefs || { plugged_in_only: false, has_battery: false, checked: true, paused: false };
      case "go_max_check_computer": return m.check;
      case "go_max_drives": if (!m.drives) throw "no drives"; return m.drives;
      case "go_max_set_data_drive":
        if (m.drives) for (const d of m.drives.drives) d.current = d.root === args.root;
        if (m.drives && args.mode === "fresh") m.drives.data_gb = 0;
        return null;
      case "check_for_updates": if (m.update instanceof Error || typeof m.update === "string") throw m.update; return m.update;
      default: return null;
    }
  } } };
})();
