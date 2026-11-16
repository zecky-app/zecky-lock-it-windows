"""UI test for the expected finish and the spam-years line on the Go Max
page. Run from the app folder: python3 tests/ui/finish_test.py"""
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

t = types.SimpleNamespace(APP=APP, HERE=HERE, page_with=page_with, on=on, GOOD=GOOD, check=check, results=results)


STATUS = {"height": 2156174, "headers": 2156174, "estimated_height": 3579134, "progress": 0.6, "peers": 82,
          "inbound_peers": 0, "pruned": True, "size_on_disk": 11_900_000_000}
SPAM = "Now in the spam years, blocks 1,710,000 to 2,100,000 (2022–23): 50% of this stretch done."
DONE = "The spam years are done. The rest goes much faster."

async def main():
    root = tempfile.mkdtemp()
    shutil.copytree(os.path.join(t.APP, "ui"), os.path.join(root, "ui"))
    shutil.copy(os.path.join(t.HERE, "mock.js"), os.path.join(root, "mock.js"))
    index = os.path.join(root, "ui", "index.html")
    html = open(index).read().replace("<head>", '<head><script src="../mock.js"></script>', 1)
    open(index, "w").write(html)
    server = subprocess.Popen([sys.executable, "-m", "http.server", "8799"], cwd=root, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    time.sleep(1)
    base = "http://localhost:8799"
    prefs = {"plugged_in_only": True, "has_battery": True, "checked": True, "paused": False}
    try:
        async with async_playwright() as p:
            b = await p.chromium.launch(channel="chrome", args=["--no-sandbox"])
            # Just started: no speed yet.
            pg = await t.page_with(b, base, {"goMax": t.on(STATUS, finish_at=None, spam_line=DONE), "check": t.GOOD, "prefs": prefs})
            await pg.click("#go-max-coin"); await pg.wait_for_timeout(400)
            t.check("finish row while measuring", await pg.is_visible("#fact-finish-row") and await pg.inner_text("#fact-finish") == "Measuring speed…", await pg.inner_text("#fact-finish"))
            t.check("spam line shown", await pg.inner_text("#gm-spam") == DONE)
            # With a speed: a rounded date and time.
            finish = int(time.time()) + 20 * 3600
            pg = await t.page_with(b, base, {"goMax": t.on(STATUS, finish_at=finish, spam_line=SPAM), "check": t.GOOD, "prefs": prefs})
            await pg.click("#go-max-coin"); await pg.wait_for_timeout(400)
            text = await pg.inner_text("#fact-finish")
            t.check("finish shows about a time", text.startswith("About ") and ":00" in text, text)
            t.check("spam line in the spam years", (await pg.inner_text("#gm-spam")).startswith("Now in the spam years"))
            await pg.screenshot(path="/tmp/finish.png")
            # Caught up: neither shows.
            up = dict(STATUS, height=3579134, headers=3579134, progress=1.0)
            pg = await t.page_with(b, base, {"goMax": t.on(up), "check": t.GOOD, "prefs": prefs})
            await pg.click("#go-max-coin"); await pg.wait_for_timeout(400)
            t.check("hidden when caught up", not await pg.is_visible("#fact-finish-row") and not await pg.is_visible("#gm-spam"))
            t.check("no page errors", not pg.errors, pg.errors)
            await b.close()
    finally:
        server.terminate()
    failed = [n for n, ok in t.results if not ok]
    print(f"{len(t.results) - len(failed)} passed, {len(failed)} failed")
    sys.exit(1 if failed else 0)

asyncio.run(main())
