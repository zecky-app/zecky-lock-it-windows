"""UI test for data use on Go Max: the data line before the start (metered
or not), the data overview, and why Go Max is private. Run from the app
folder: python3 tests/ui/data_test.py"""
import asyncio, os, sys, time
from playwright.async_api import async_playwright
import shutil, subprocess, tempfile, json, types

HERE = os.path.dirname(os.path.abspath(__file__))
APP = os.path.dirname(os.path.dirname(HERE))
results = []

def check(name, ok, detail=""):
    results.append((name, ok))
    print(("PASS " if ok else "FAIL ") + name + ("" if ok else f"  ({detail})"))

async def page_with(browser, base, m):
    pg = await browser.new_page(viewport={"width": 720, "height": 820})
    errors = []
    pg.on("pageerror", lambda e: errors.append(str(e)))
    await pg.add_init_script("window.__m = " + json.dumps(m) + ";")
    await pg.goto(base + "/ui/index.html")
    await pg.add_style_tag(content="*{animation:none!important;transition:none!important}")
    await pg.wait_for_timeout(700)
    pg.errors = errors
    return pg

FINE = {"state": {"state": "fine"}, "text": None, "ask": False, "choice": None, "date": None}
OFF = {"state": "off", "installed": True, "problem": None, "stopped_while_starting": False, "retrying": False, "has_data": False, "support": FINE, "paused": False}
GOOD = {"free_gb": 120, "needed_gb": 30, "enough_space": True, "memory_gb": 16, "low_memory": False, "disk": "ssd", "has_battery": True, "on_battery": False}

def on(status, **extra):
    v = {"state": "on", "wallet_ready": True, "wallet_note": None, "onion": None, "onion_problem": None, "reading_note": None,
         "wallet_quiet": None, "unsettled": None, "support": FINE, "status": status}
    v.update(extra)
    return v

OFF_VIEW = OFF
t = types.SimpleNamespace(APP=APP, HERE=HERE, page_with=page_with, on=on, GOOD=GOOD, check=check, results=results)


STATUS = {"height": 3579134, "headers": 3579134, "estimated_height": 3579134, "progress": 1.0, "peers": 12,
          "inbound_peers": 0, "pruned": True, "size_on_disk": 18_600_000_000}
DATA = {"downloaded": 212_400_000_000, "kept": 18_600_000_000, "removed": 193_800_000_000, "uploaded": 1_200_000_000}

async def main():
    root = tempfile.mkdtemp()
    shutil.copytree(os.path.join(t.APP, "ui"), os.path.join(root, "ui"))
    shutil.copy(os.path.join(t.HERE, "mock.js"), os.path.join(root, "mock.js"))
    index = os.path.join(root, "ui", "index.html")
    html = open(index).read().replace("<head>", '<head><script src="../mock.js"></script>', 1)
    open(index, "w").write(html)
    server = subprocess.Popen([sys.executable, "-m", "http.server", "8800"], cwd=root, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    time.sleep(1)
    base = "http://localhost:8800"
    prefs = {"plugged_in_only": True, "has_battery": True, "checked": False, "paused": False}
    good = dict(t.GOOD, metered=False, download_gb=350)
    try:
        async with async_playwright() as p:
            b = await p.chromium.launch(channel="chrome", args=["--no-sandbox"])
            # The check: a calm data line, or a warning on a metered connection; never blocks.
            for metered, want, mark in [(False, "Data: the first sync downloads about 350 GB. Use a connection without a data limit.", "ok"),
                                        (True, "Data: this connection is metered. The first sync downloads about 350 GB; better to use one without a data limit.", "warn")]:
                pg = await t.page_with(b, base, {"goMax": OFF_VIEW, "check": dict(good, metered=metered), "prefs": prefs})
                await pg.click("#go-max-coin"); await pg.wait_for_timeout(300)
                await pg.click("#node-toggle"); await pg.wait_for_timeout(300)
                lines = await pg.inner_text("#pc-lines")
                marks = await pg.eval_on_selector_all("#pc-lines li", "ls => ls.filter(l => l.textContent.includes('Data:')).map(l => l.querySelector('.pc-mark').className)")
                t.check(f"data line (metered {metered})", want in lines and marks == [f"pc-mark {mark}"], f"{lines} {marks}")
                t.check(f"start not blocked (metered {metered})", await pg.is_enabled("#pc-start"))
                if metered:
                    await pg.screenshot(path="/tmp/data-check.png")
            # On battery: a node with data keeps going up to date (no warning);
            # a first start says plug in.
            for has_data, want, mark in [(True, "On battery. Catching up waits for the charger; up to date, the node keeps running.", "ok"),
                                         (False, "On battery: plug in to sync.", "warn")]:
                check = dict(good, has_battery=True, on_battery=True, has_node_data=has_data)
                pg = await t.page_with(b, base, {"goMax": OFF_VIEW, "check": check, "prefs": dict(prefs, has_battery=True)})
                await pg.click("#go-max-coin"); await pg.wait_for_timeout(300)
                await pg.click("#node-toggle"); await pg.wait_for_timeout(300)
                marks = await pg.eval_on_selector_all("#pc-lines li", "ls => ls.filter(l => l.textContent.includes('battery')).map(l => [l.textContent, l.querySelector('.pc-mark').className])")
                t.check(f"battery line (data {has_data})", len(marks) == 1 and want in marks[0][0] and marks[0][1] == f"pc-mark {mark}", marks)
            # The overview, with the numbers as given.
            pg = await t.page_with(b, base, {"goMax": t.on(STATUS, data=DATA), "check": good, "prefs": dict(prefs, checked=True)})
            await pg.click("#go-max-coin"); await pg.wait_for_timeout(400)
            text = await pg.inner_text("#gm-data")
            t.check("overview shown", await pg.is_visible("#gm-data"))
            t.check("overview numbers", all(x in text for x in ["212.4 GB", "18.6 GB", "193.8 GB", "1.2 GB"]), text)
            t.check("overview note", "Counted on this PC only; nothing is sent." in text)
            # Why private: closed, then open.
            t.check("private line", (await pg.inner_text("#gm-private")).startswith("Private: your node connects to other Zcash nodes, but nobody can connect to it."))
            t.check("why closed at first", not await pg.is_visible("#gm-why"))
            await pg.click("#gm-private-more"); await pg.wait_for_timeout(200)
            why = await pg.inner_text("#gm-why")
            t.check("why opens", await pg.is_visible("#gm-why") and "Why Go Max is private." in why and "Zecky Rock it" in why)
            await pg.screenshot(path="/tmp/data-page.png", full_page=True)
            await pg.click("#gm-private-more"); await pg.wait_for_timeout(200)
            t.check("why closes", not await pg.is_visible("#gm-why"))
            # Nothing to show: no overview.
            pg2 = await t.page_with(b, base, {"goMax": t.on(STATUS, data=None), "check": good, "prefs": dict(prefs, checked=True)})
            await pg2.click("#go-max-coin"); await pg2.wait_for_timeout(400)
            t.check("no overview without data", not await pg2.is_visible("#gm-data"))
            t.check("no page errors", not pg.errors and not pg2.errors, pg.errors + pg2.errors)
            await b.close()
    finally:
        server.terminate()
    failed = [n for n, ok in t.results if not ok]
    print(f"{len(t.results) - len(failed)} passed, {len(failed)} failed")
    sys.exit(1 if failed else 0)

asyncio.run(main())
