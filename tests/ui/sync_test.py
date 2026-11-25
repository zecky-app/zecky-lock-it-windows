"""UI test: the newest blocks decide "up to date" (Lock it Mac 74d88bd +
5d9c0ad): the status while older blocks are checked, the Sync panel's
"N blocks still to check", its time left, and the "Older blocks" row.
Run from the app folder: python3 tests/ui/sync_test.py"""
import asyncio, json, os, shutil, subprocess, sys, tempfile, time
from playwright.async_api import async_playwright

HERE = os.path.dirname(os.path.abspath(__file__))
APP = os.path.dirname(os.path.dirname(HERE))
results = []

def check(name, ok, detail=""):
    results.append((name, ok))
    print(("PASS " if ok else "FAIL ") + name + ("" if ok else f"  ({detail})"))

async def page_with(browser, base, m, stored=None):
    pg = await browser.new_page(viewport={"width": 720, "height": 820})
    errors = []
    pg.on("pageerror", lambda e: errors.append(str(e)))
    script = "window.__m = " + json.dumps(m) + ";"
    if stored is not None:
        script += f"localStorage.setItem('outside.lastAsked', '{stored}');"
    else:
        script += "localStorage.removeItem('outside.lastAsked');"
    await pg.add_init_script(script)
    await pg.goto(base + "/ui/index.html")
    await pg.add_style_tag(content="*{animation:none!important;transition:none!important}")
    await pg.wait_for_timeout(900)
    pg.errors = errors
    return pg

async def main():
    root = tempfile.mkdtemp()
    shutil.copytree(os.path.join(APP, "ui"), os.path.join(root, "ui"))
    shutil.copy(os.path.join(HERE, "mock.js"), os.path.join(root, "mock.js"))
    index = os.path.join(root, "ui", "index.html")
    html = open(index).read().replace("<head>", '<head><script src="../mock.js"></script>', 1)
    open(index, "w").write(html)
    server = subprocess.Popen([sys.executable, "-m", "http.server", "8781"], cwd=root, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    time.sleep(1)
    base = "http://localhost:8781"
    try:
        async with async_playwright() as p:
            b = await p.chromium.launch(channel="chrome", args=["--no-sandbox", "--use-fake-device-for-media-stream", "--use-fake-ui-for-media-stream"])

            # A watch wallet just added from Orchard's start: its older blocks
            # are open, the newest are checked. The main wallet starts at 3,100,000.
            older = {"older": True, "scanned": 1687104, "scanFrom": 1687104,
                     "toScan": [[1687104, 2950000]],
                     "pace": {"open": 1262896, "minutes": 70, "newest_checked": True}}
            pg = await page_with(b, base, older)
            status = await pg.text_content("#status-text")
            check("home: up to date, checking older blocks", status.startswith("Up to date · checking older blocks"), status)
            await pg.click("#status-text"); await pg.wait_for_timeout(300)
            check("sync: subtitle", (await pg.text_content("#sync-sub")) == "Up to date · checking older blocks")
            behind = await pg.text_content("#sync-behind")
            check("sync: Behind = blocks still to check + time", behind == "1,262,896 blocks still to check, about 70 min", behind)
            check("sync: Older blocks row shown", await pg.is_visible("#sync-older-line"))
            row = await pg.text_content("#sync-older")
            check("sync: Older blocks share, start and time", row == "10% checked · from 1,687,104 · about 70 min", row)
            note = await pg.text_content("#sync-ring-note")
            check("sync: ring note counts blocks still to check", note == "Of all blocks since block 1,687,104. 1,262,896 blocks still to check.", note)
            check("sync: ring not at 100%", (await pg.text_content("#sync-percent")) != "100%")
            check("sync: ring word checked", (await pg.text_content("#sync-ring-word")) == "checked")
            # Halfway through the older blocks: the share grows, the start stays.
            await pg.evaluate("window.__m.toScan = [[2318552, 2950000]]; window.__m.pace = {open: 631448, minutes: 35, newest_checked: true}")
            await pg.wait_for_timeout(4500)
            row = await pg.text_content("#sync-older")
            check("sync: older share grows, start stays", row == "55% checked · from 1,687,104 · about 35 min", row)
            check("no page errors (older)", not pg.errors, pg.errors)
            await pg.close()

            # The wallet whose older blocks are open: its balance may grow.
            WALLETS = [{"id": "main", "name": "Lock it", "birthday": 3100000, "watch_only": False, "main": True},
                       {"id": "w-1", "name": "Shop", "birthday": 1687104, "watch_only": True, "main": False}]
            hint = "Lock it checks the newest blocks first. Payments from older blocks are still being added to this wallet."
            pg = await page_with(b, base, dict(older, wallets=WALLETS, current="w-1",
                                               pace={"open": 1262896, "minutes": 70, "newest_checked": True, "own_checked": False}))
            check("may grow: shown for the wallet with older blocks open", await pg.is_visible("#older-line"))
            check("may grow: text", (await pg.text_content("#older-line")) == "Still checking older blocks · may grow")
            check("may grow: hint", (await pg.get_attribute("#older-line", "title")) == hint)
            await pg.close()
            # The main wallet meanwhile: its own blocks are checked, no line.
            pg = await page_with(b, base, dict(older, wallets=WALLETS, current=None,
                                               pace={"open": 1262896, "minutes": 70, "newest_checked": True, "own_checked": True}))
            check("may grow: hidden for a wallet whose own blocks are checked", not await pg.is_visible("#older-line"))
            await pg.close()
            # Not at the newest blocks yet: the usual catching up, no line.
            pg = await page_with(b, base, {"syncing": True, "pace": {"open": 1000, "minutes": None, "newest_checked": False, "own_checked": False}})
            check("may grow: hidden while catching up", not await pg.is_visible("#older-line"))
            await pg.close()

            # Not yet at the newest blocks: no time until the pace is known.
            pg = await page_with(b, base, {"syncing": True, "scanned": 3599000, "toScan": [[3599000, 3600001]],
                                           "pace": {"open": 1000, "minutes": None, "newest_checked": False}})
            await pg.click("#status-text"); await pg.wait_for_timeout(300)
            check("catching up: subtitle", (await pg.text_content("#sync-sub")) == "Checking blocks")
            check("catching up: Behind without a time yet", (await pg.text_content("#sync-behind")) == "1,000 blocks still to check")
            check("catching up: no Older blocks row", not await pg.is_visible("#sync-older-line"))
            await pg.close()

            # The Go Max button follows the switch (Lock it Mac 601a6c8):
            # the coin left while off, right while on or starting; a click
            # only opens the page.
            pg = await page_with(b, base, {})
            await pg.wait_for_timeout(500)
            check("coin: left while Go Max is off", not await pg.evaluate("document.getElementById('go-max-coin').classList.contains('on-right')"))
            await pg.evaluate("window.__m.goMax = Object.assign({}, window.__m.goMax, {state: 'starting'})")
            await pg.wait_for_timeout(4500)
            check("coin: right once switched on", await pg.evaluate("document.getElementById('go-max-coin').classList.contains('on-right')"))
            title_right = await pg.evaluate("getComputedStyle(document.querySelector('#go-max-coin .coin-text')).alignItems")
            check("coin: the words beside it on the left", title_right == "flex-end", title_right)
            await pg.click("#go-max-coin"); await pg.wait_for_timeout(300)
            check("coin: a click opens Go Max", await pg.is_visible("#sheet-gomax"))
            check("coin: a click switches nothing", (await pg.evaluate("window.__m.goMax.state")) == "starting")
            check("no page errors (coin)", not pg.errors, pg.errors)
            await pg.close()

            # Everything checked.
            pg = await page_with(b, base, {})
            status = await pg.text_content("#status-text")
            check("done: home says Up to date, no older", status.startswith("Up to date") and "older" not in status, status)
            await pg.click("#status-text"); await pg.wait_for_timeout(300)
            check("done: subtitle", (await pg.text_content("#sync-sub")) == "Up to date")
            check("done: Behind = Nothing", (await pg.text_content("#sync-behind")) == "Nothing")
            check("done: ring 100%", (await pg.text_content("#sync-percent")) == "100%")
            check("done: no Older blocks row", not await pg.is_visible("#sync-older-line"))
            check("done: no may-grow line", not await pg.is_visible("#older-line"))
            check("no page errors (done)", not pg.errors, pg.errors)
            await pg.close()
    finally:
        server.terminate()
    failed = [n for n, ok in results if not ok]
    print(f"{len(results) - len(failed)} passed, {len(failed)} failed")
    sys.exit(1 if failed else 0)

asyncio.run(main())
