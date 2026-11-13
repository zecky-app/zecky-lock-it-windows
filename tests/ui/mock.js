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
    balance: { spendable: 2240000, pending: 0, total: 2240000, scanned_height: 3600000, chain_tip: 3600000, progress: 1, birthday: 3100000, to_scan: [], orchard: 0, ironwood: 0, sapling: 0, transparent: 0 },
    sync: { state: "up_to_date", server: "zec.rocks", problem: null }, unsent: false, lock_open: m.lockOpen, problems: [],
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
      case "go_max_view": return m.goMax;
      case "settings_get": return { settings: { tor: true, wait: 0 }, reading_from: "zec.rocks", version: "0.1.5" };
      case "wallet_prepare": return { to: args.to, amount: 1000000, fee: 15000, total: 1015000 };
      case "keyring_layout": m.hidden = args.hidden; return null;
      case "keyring_select": m.current = args.id; return null;
      case "price_get": return { usd: 40, eur: 37 };
      default: return null;
    }
  } } };
})();
