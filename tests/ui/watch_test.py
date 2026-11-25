"""UI test: Watch a wallet, from its viewing key (Lock it Mac 5d9c0ad,
0c0f3cb): the sheet and its errors, scanning the key, and the watch
wallet itself: eye chip, never Send, shielded-only Receive, no words, the
removal text and "Received" when Zcash arrives.
Run from the app folder: python3 tests/ui/watch_test.py"""
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
    server = subprocess.Popen([sys.executable, "-m", "http.server", "8777"], cwd=root, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    time.sleep(1)
    base = "http://localhost:8777"
    try:
        async with async_playwright() as p:
            b = await p.chromium.launch(channel="chrome", args=["--no-sandbox", "--use-fake-device-for-media-stream", "--use-fake-ui-for-media-stream"])

            MAIN = [{"id": "main", "name": "Lock it", "birthday": 3100000, "watch_only": False, "main": True}]
            SHOP = MAIN + [{"id": "watch-1", "name": "Shop", "birthday": 1687104, "watch_only": False, "view_only": True, "main": False}]
            KEY = "uview1examplekeyforthetest"

            # The menu and the sheet, with the Mac's words.
            pg = await page_with(b, base, {"wallets": MAIN, "current": None})
            await pg.click(".ring-round.plus")
            menu = await pg.evaluate("[...document.querySelectorAll('button')].map(b => b.textContent)")
            check("menu: Watch a wallet", "Watch a wallet" in menu)
            await pg.click("text=Watch a wallet"); await pg.wait_for_timeout(300)
            sheet = await pg.inner_text("#sheet-watch")
            check("sheet: title and sub", "Watch a wallet" in sheet and "Lock it can never spend from it." in sheet)
            check("sheet: since when", "Since when has this wallet been used?" in sheet)
            check("sheet: note for a month", (await pg.text_content("#watch-since-note")).startswith("Lock it checks from a little before then."))
            await pg.check("#watch-unknown")
            check("I don't know: Orchard's start", (await pg.text_content("#watch-since-note")).startswith("Lock it checks from Orchard's start (2022)."))
            check("I don't know: month and year off", (await pg.get_attribute("#sheet-watch .seal-dates", "aria-disabled")) == "true")
            # The Mac's errors.
            async def attempt(key, name):
                await pg.fill("#watch-key", key); await pg.fill("#watch-name", name)
                await pg.click("#watch-go"); await pg.wait_for_timeout(300)
                return await pg.text_content("#watch-message")
            check("error: not a key", (await attempt("u1notakey", "Shop")).startswith("That is not a Zcash viewing key."))
            check("error: testnet", (await attempt("uviewtest1abc", "Shop")).startswith("That is a testnet viewing key."))
            check("error: no name", (await attempt(KEY, "  ")) == "Give the wallet a name, for example Shop.")
            await pg.evaluate(f"window.__m.knownKey = '{KEY}'")
            check("error: already here", (await attempt(KEY, "Shop")) == "That wallet is already in Lock it.")
            await pg.evaluate("window.__m.knownKey = null")
            # Added: "I don't know" sends no month; the sheet closes, the wallet is chosen.
            await attempt(KEY, "Shop")
            check("added: no month for I don't know", await pg.evaluate("window.__m.lastWatch.madeIn === null"))
            check("added: sheet closed", not await pg.is_visible("#sheet-watch"))
            check("added: chosen", (await pg.evaluate("window.__m.current")) == "watch-1")
            check("no page errors (add)", not pg.errors, pg.errors)
            await pg.close()

            # A month: sent as year and month.
            pg = await page_with(b, base, {"wallets": MAIN, "current": None})
            await pg.click(".ring-round.plus"); await pg.click("text=Watch a wallet")
            await pg.select_option("#watch-year", "2024"); await pg.select_option("#watch-month", "3")
            await pg.fill("#watch-key", KEY); await pg.fill("#watch-name", "Shop")
            await pg.click("#watch-go"); await pg.wait_for_timeout(300)
            check("month: year and month sent", await pg.evaluate("JSON.stringify(window.__m.lastWatch.madeIn)") == '{"year":2024,"month":3}')
            await pg.close()

            # Scanning the key: back to the form with the key, the name kept.
            pg = await page_with(b, base, {"wallets": MAIN, "current": None, "scannedKey": KEY})
            await pg.click(".ring-round.plus"); await pg.click("text=Watch a wallet")
            await pg.fill("#watch-name", "Shop")
            await pg.click("#watch-scan"); await pg.wait_for_timeout(300)
            check("scan: camera screen says Watch a wallet", (await pg.text_content("#seal-title")) == "Watch a wallet")
            await pg.click("#seal-paste-box summary")
            await pg.fill("#seal-paste", KEY); await pg.click("#seal-paste-go"); await pg.wait_for_timeout(400)
            check("scan: back on the form", await pg.is_visible("#sheet-watch"))
            check("scan: key filled", (await pg.input_value("#watch-key")) == KEY)
            check("scan: name kept", (await pg.input_value("#watch-name")) == "Shop")
            check("no page errors (scan)", not pg.errors, pg.errors)
            await pg.close()

            # The watch wallet itself.
            pg = await page_with(b, base, {"wallets": SHOP, "current": "watch-1", "transparent": 50000, "orchard": 70000})
            chip_icon = await pg.evaluate("document.querySelector('.ring-chip.on svg').outerHTML")
            check("chip: an eye", "M12 5c5 0" in chip_icon)
            check("chip: title", (await pg.get_attribute(".ring-chip.on", "title")) == "Watch only: Lock it can never spend from it")
            check("home: says watch only", (await pg.text_content("#held-line")) == "Watch only · Lock it can never spend from it")
            check("home: no signing-device send", not await pg.is_visible("#home-seal-send"))
            check("home: no lock moves", not await pg.is_visible("#move-lockin") and not await pg.is_visible("#move-ironwood"))
            await pg.click("#padlock"); await pg.wait_for_timeout(400)
            check("padlock: opens nothing", not await pg.is_visible("#sheet-seal") and await pg.evaluate("!document.querySelector('.scrim:not([hidden])')"))
            await pg.click("#home-receive"); await pg.wait_for_timeout(300)
            check("receive: no Transparent tab", not await pg.is_visible("#receive-tab-transparent"))
            check("receive: the Mac's words", (await pg.text_content("#receive-sub")) == "Payments to this shielded address show in this watch-only wallet. Lock it cannot spend them.")
            await pg.click("#sheet-receive [data-close]"); await pg.wait_for_timeout(200)
            await pg.click(".ring-chip.on", button="right"); await pg.wait_for_timeout(200)
            menu = await pg.evaluate("[...document.querySelectorAll('button')].filter(b => b.offsetParent).map(b => b.textContent)")
            check("menu: no Show its words", not any("Show its words" in t for t in menu), menu)
            await pg.keyboard.press("Escape"); await pg.wait_for_timeout(200)
            await pg.evaluate("openSettings ? openSettings('phrase') : null") if await pg.evaluate("typeof openSettings === 'function'") else None
            await pg.evaluate("document.querySelector('[data-page=phrase]').hidden = false; document.getElementById('phrase-toggle').click()")
            await pg.wait_for_timeout(300)
            note = await pg.text_content("#phrase-note")
            check("settings: no words to show, no keys asked", note == "Shop is watch only: Lock it has only its viewing key, no words, so there is nothing to show here.", note)
            check("no page errors (watch wallet)", not pg.errors, pg.errors)
            await pg.close()

            # Zcash arrived in it: Received, not Locked in.
            pg = await page_with(b, base, {"wallets": SHOP, "current": None,
                                           "arrivals": [{"txid": "w1", "account": "watch-1", "kind": "locked_in", "amount": 1000000}]})
            for _ in range(50):
                if await pg.is_visible("#arrival"):
                    break
                await pg.wait_for_timeout(100)
            check("arrival: Received · Shop", (await pg.text_content("#arrival-title")) == "Received · Shop")
            await pg.close()
    finally:
        server.terminate()
    failed = [n for n, ok in results if not ok]
    print(f"{len(results) - len(failed)} passed, {len(failed)} failed")
    sys.exit(1 if failed else 0)

asyncio.run(main())
