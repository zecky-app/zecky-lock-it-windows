"""UI test for where to keep the node: the drive list in "Before you start Go
Max", the recommendation, and move or start fresh. Run from the app folder:
python3 tests/ui/drive_test.py"""
import asyncio, json, os, shutil, subprocess, sys, tempfile, time
from playwright.async_api import async_playwright

HERE = os.path.dirname(os.path.abspath(__file__))
APP = os.path.dirname(os.path.dirname(HERE))
results = []

def check(name, ok, detail=""):
    results.append((name, ok))
    print(("PASS " if ok else "FAIL ") + name + ("" if ok else f"  ({detail})"))

async def page_with(browser, base, m):
    pg = await browser.new_page(viewport={"width": 720, "height": 900})
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
GOOD = {"free_gb": 120, "needed_gb": 30, "enough_space": True, "memory_gb": 16, "low_memory": False, "disk": "hdd", "has_battery": False, "on_battery": False, "metered": False, "download_gb": 350}
PREFS = {"plugged_in_only": False, "has_battery": False, "checked": False, "paused": False}

def drives(data_gb=0.0, running=False):
    return {"data_gb": data_gb, "running": running, "drives": [
        {"root": "C:\\", "label": None, "free_gb": 120, "disk": "hdd", "system": True, "current": True, "fits": True, "recommended": False, "path": "C:\\Users\\x\\AppData\\Local\\app.zecky.lockit\\go-max"},
        {"root": "D:\\", "label": "Fast", "free_gb": 400, "disk": "ssd", "system": False, "current": False, "fits": True, "recommended": True, "path": "D:\\ZeckyLockIt\\go-max"},
        {"root": "E:\\", "label": None, "free_gb": 12, "disk": "unknown", "system": False, "current": False, "fits": False, "recommended": False, "path": "E:\\ZeckyLockIt\\go-max"}]}

async def open_check(pg):
    await pg.click("#go-max-coin"); await pg.wait_for_timeout(300)
    await pg.click("#node-toggle"); await pg.wait_for_timeout(400)

async def main():
    root = tempfile.mkdtemp()
    shutil.copytree(os.path.join(APP, "ui"), os.path.join(root, "ui"))
    shutil.copy(os.path.join(HERE, "mock.js"), os.path.join(root, "mock.js"))
    index = os.path.join(root, "ui", "index.html")
    html = open(index).read().replace("<head>", '<head><script src="../mock.js"></script>', 1)
    open(index, "w").write(html)
    server = subprocess.Popen([sys.executable, "-m", "http.server", "8801"], cwd=root, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    time.sleep(1)
    base = "http://localhost:8801"
    try:
        async with async_playwright() as p:
            b = await p.chromium.launch(channel="chrome", args=["--no-sandbox"])
            # No node data yet: the list, the recommendation, choosing applies at once.
            pg = await page_with(b, base, {"goMax": OFF, "check": GOOD, "prefs": PREFS, "drives": drives()})
            await open_check(pg)
            names = await pg.eval_on_selector_all(".pc-drive", "bs => bs.map(b => b.textContent)")
            check("three drives listed", len(names) == 3 and names[0].startswith("C:") and "Fast" in names[1], names)
            check("recommended on the SSD", "Recommended" in names[1] and "Recommended" not in names[0])
            check("current drive marked", await pg.get_attribute(".pc-drive:nth-child(1)", "aria-checked") == "true")
            check("too small drive disabled", await pg.is_disabled(".pc-drive:nth-child(3)") and "not enough space" in names[2])
            check("recommendation said why", "SSD with room to spare" in await pg.inner_text("#pc-where-note"))
            await pg.screenshot(path="/tmp/drive-check.png")
            await pg.click(".pc-drive:nth-child(2)"); await pg.wait_for_timeout(400)
            calls = await pg.evaluate("window.__calls.filter(c => c[0] === 'go_max_set_data_drive').map(c => c[1])")
            check("no data: chosen at once", calls == [{"root": "D:\\", "mode": "fresh"}], calls)
            check("now on D:", await pg.get_attribute(".pc-drive:nth-child(2)", "aria-checked") == "true")
            check("no page errors", not pg.errors, pg.errors)

            # Node data on C:: move or start fresh, the owner chooses.
            pg = await page_with(b, base, {"goMax": dict(OFF, has_data=True), "check": GOOD, "prefs": PREFS, "drives": drives(20.1)})
            await pg.click("#go-max-coin"); await pg.wait_for_timeout(300)
            await pg.click("#gm-check-link"); await pg.wait_for_timeout(400)
            await pg.click(".pc-drive:nth-child(2)"); await pg.wait_for_timeout(300)
            text = await pg.inner_text("#pc-move-text")
            check("move choice shown", await pg.is_visible("#pc-move") and "20.1 GB" in text and "it is copied to D: and checked" in text and "D::" not in text, text)
            await pg.screenshot(path="/tmp/drive-move.png")
            nothing = await pg.evaluate("window.__calls.filter(c => c[0] === 'go_max_set_data_drive').length")
            check("nothing changes before choosing", nothing == 0)
            await pg.click("#pc-move-it"); await pg.wait_for_timeout(400)
            calls = await pg.evaluate("window.__calls.filter(c => c[0] === 'go_max_set_data_drive').map(c => c[1])")
            check("move asked", calls == [{"root": "D:\\", "mode": "move"}], calls)
            check("sheet closes for the move", not await pg.is_visible("#pc-check"))

            # Go Max on: the place cannot change.
            pg = await page_with(b, base, {"goMax": dict(OFF, has_data=True), "check": GOOD, "prefs": PREFS, "drives": drives(20.1, running=True)})
            await pg.click("#go-max-coin"); await pg.wait_for_timeout(300)
            await pg.click("#gm-check-link"); await pg.wait_for_timeout(400)
            check("locked while running", await pg.is_disabled(".pc-drive:nth-child(2)") and "Switch Go Max off" in await pg.inner_text("#pc-where-note"))

            # Moving shows on the page.
            pg = await page_with(b, base, {"goMax": {"state": "starting", "step": "Moving the node's data · 4.2 of 20.1 GB"}, "check": GOOD, "prefs": PREFS, "drives": drives(20.1)})
            await pg.click("#go-max-coin"); await pg.wait_for_timeout(300)
            check("moving state", await pg.inner_text("#node-state") == "Moving" and "4.2 of 20.1 GB" in await pg.inner_text("#node-lead"))
            await b.close()
    finally:
        server.terminate()
    failed = [n for n, ok in results if not ok]
    print(f"{len(results) - len(failed)} passed, {len(failed)} failed")
    sys.exit(1 if failed else 0)

asyncio.run(main())
