"""UI tests for beta 2: the check before Go Max, paused on battery, the
plugged-in option, the up-to-date wording, not connected, phone access off,
and Check for updates. Run from the app folder: python3 tests/ui/beta2_test.py"""
import asyncio, json, os, shutil, subprocess, sys, tempfile, time
from playwright.async_api import async_playwright

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

async def main():
    root = tempfile.mkdtemp()
    shutil.copytree(os.path.join(APP, "ui"), os.path.join(root, "ui"))
    shutil.copy(os.path.join(HERE, "mock.js"), os.path.join(root, "mock.js"))
    index = os.path.join(root, "ui", "index.html")
    html = open(index).read().replace("<head>", '<head><script src="../mock.js"></script>', 1)
    open(index, "w").write(html)
    server = subprocess.Popen([sys.executable, "-m", "http.server", "8798"], cwd=root, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    time.sleep(1)
    base = "http://localhost:8798"
    try:
        async with async_playwright() as p:
            b = await p.chromium.launch(channel="chrome", args=["--no-sandbox"])

            # The check shows at the first switch-on; all fine.
            pg = await page_with(b, base, {"goMax": OFF, "check": GOOD, "prefs": {"plugged_in_only": True, "has_battery": True, "checked": False, "paused": False}})
            await pg.click("#go-max-coin"); await pg.wait_for_timeout(300)
            await pg.click("#node-toggle"); await pg.wait_for_timeout(300)
            check("check shown at first switch-on", await pg.is_visible("#pc-check"))
            lines = await pg.inner_text("#pc-lines")
            check("check lines", "Free space: 120 GB (Go Max needs about 30 GB)." in lines and "Memory: 16 GB." in lines and "Drive: SSD." in lines and "Plugged in." in lines, lines)
            check("start enabled", await pg.is_enabled("#pc-start"))
            calls = await pg.evaluate("window.__calls.map(c => c[0])")
            check("not started before the choice", "go_max_start" not in calls)
            await pg.click("#pc-start"); await pg.wait_for_timeout(300)
            calls = await pg.evaluate("window.__calls.map(c => c[0])")
            check("Start Go Max starts and remembers", "go_max_checked" in calls and "go_max_start" in calls)
            check("plugged-in option on a laptop", await pg.is_visible("#gm-plugged-row") and await pg.is_checked("#gm-plugged"))
            check("no page errors (check)", not pg.errors, pg.errors)
            await pg.close()

            # Too little space blocks; the rest only warns.
            small = dict(GOOD, free_gb=18, enough_space=False, memory_gb=4, low_memory=True, disk="hdd", on_battery=True)
            pg = await page_with(b, base, {"goMax": OFF, "check": small, "prefs": {"plugged_in_only": True, "has_battery": True, "checked": False, "paused": False}})
            await pg.click("#go-max-coin"); await pg.wait_for_timeout(300)
            await pg.click("#node-toggle"); await pg.wait_for_timeout(300)
            lines = await pg.inner_text("#pc-lines")
            check("too little space named", "Not enough free space: 18 GB free, about 30 GB needed." in lines, lines)
            check("start blocked", not await pg.is_enabled("#pc-start"))
            check("warnings", "Memory: 4 GB. This PC may feel slow" in lines and "Drive: hard drive. The first check can take several days." in lines and "On battery: plug in to sync." in lines, lines)
            await pg.click("#pc-not-now"); await pg.wait_for_timeout(200)
            calls = await pg.evaluate("window.__calls.map(c => c[0])")
            check("Not now closes without starting", not await pg.is_visible("#pc-check") and "go_max_start" not in calls)
            await pg.close()

            # Desktop with enough room: no power line; enough memory and space only.
            desk = dict(GOOD, has_battery=False, disk="unknown")
            pg = await page_with(b, base, {"goMax": OFF, "check": desk})
            await pg.click("#go-max-coin"); await pg.wait_for_timeout(300)
            await pg.click("#gm-check-link"); await pg.wait_for_timeout(300)
            lines = await pg.inner_text("#pc-lines")
            check("desktop: no power line, no drive line when unknown", "Plugged in" not in lines and "Drive" not in lines, lines)
            check("plugged-in option hidden on a desktop", not await pg.is_visible("#gm-plugged-row"))
            await pg.close()

            # Paused on battery.
            pg = await page_with(b, base, {"goMax": dict(OFF, paused=True)})
            await pg.click("#go-max-coin"); await pg.wait_for_timeout(300)
            check("paused state", (await pg.text_content("#node-state")) == "Paused")
            check("paused line", (await pg.text_content("#node-lead")).startswith("Paused while on battery."))
            check("paused coin", (await pg.text_content("#coin-sub")) == "Paused · on battery")
            await pg.close()

            # Up to date but Lock it still on its server; phone access hidden.
            st = {"height": 3600000, "estimated_height": 3600003, "progress": 1.0, "peers": 12, "size_on_disk": 19000000000}
            pg = await page_with(b, base, {"goMax": on(st)})
            await pg.click("#go-max-coin"); await pg.wait_for_timeout(400)
            check("up to date", (await pg.text_content("#node-state")) == "Up to date")
            lead = await pg.text_content("#node-lead")
            check("says why still on the server", lead.startswith("Lock it still reads the chain from zec.rocks. It moves to this node by itself"), lead)
            check("phone access row hidden", not await pg.is_visible("#fact-onion-rows"))
            await pg.close()

            # A little behind.
            st2 = dict(st, height=3599950)
            pg = await page_with(b, base, {"goMax": on(st2)})
            await pg.click("#go-max-coin"); await pg.wait_for_timeout(400)
            check("a little behind", (await pg.text_content("#node-state")) == "A little behind" and (await pg.text_content("#node-lead")).startswith("50 blocks to catch up"))
            await pg.close()

            # No peers for over a minute: not connected, Rock it named.
            st3 = dict(st, height=1800000, progress=0.5, peers=0)
            pg = await page_with(b, base, {"goMax": on(st3)})
            await pg.click("#go-max-coin"); await pg.wait_for_timeout(400)
            await pg.evaluate("noPeersSince = Date.now() - 70000; refreshGoMax()"); await pg.wait_for_timeout(300)
            check("not connected", (await pg.text_content("#node-state")) == "Not connected" and "such as Rock it" in await pg.text_content("#node-lead"))
            await pg.close()

            # Check for updates.
            pg = await page_with(b, base, {"update": {"current": "0.1.0-beta.2", "newest": "0.1.0-beta.3", "newer": True}})
            await pg.click("#open-settings") if await pg.query_selector("#open-settings") else None
            await pg.evaluate("openSheet('settings'); goSettings('about')"); await pg.wait_for_timeout(300)
            await pg.click("#update-check"); await pg.wait_for_timeout(300)
            check("newer version shown", (await pg.text_content("#update-result")) == "Lock it 0.1.0-beta.3 is out. You have 0.1.0-beta.2." and await pg.is_visible("#update-get"))
            check("terms and privacy rows", await pg.is_visible("#about-terms") and await pg.is_visible("#about-privacy"))
            await pg.close()
            pg = await page_with(b, base, {"update": {"current": "0.1.0-beta.2", "newest": "0.1.0-beta.2", "newer": False}})
            await pg.evaluate("openSheet('settings'); goSettings('about')"); await pg.wait_for_timeout(300)
            await pg.click("#update-check"); await pg.wait_for_timeout(300)
            check("newest", (await pg.text_content("#update-result")) == "You have the newest version." and not await pg.is_visible("#update-get"))
            calls = await pg.evaluate("window.__calls.map(c => c[0])")
            check("asked only when pressed", calls.count("check_for_updates") == 1)
            await pg.close()
            await b.close()
    finally:
        server.terminate()
    failed = [n for n, ok in results if not ok]
    print(f"\n{len(results) - len(failed)} of {len(results)} passed")
    sys.exit(1 if failed else 0)

asyncio.run(main())
