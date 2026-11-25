"""UI test: Settings › Business (Lock it Mac e669411): the row's summary,
the watch-only wallets and the button, the viewing key as text and QR code
after the keys and hidden from screen capture while it shows, Bookkeeping
export with its note, and How Zecky works › For business.
Run from the app folder: python3 tests/ui/business_test.py"""
import asyncio, json, os, shutil, subprocess, sys, tempfile, time
from playwright.async_api import async_playwright

HERE = os.path.dirname(os.path.abspath(__file__))
APP = os.path.dirname(os.path.dirname(HERE))
results = []

def check(name, ok, detail=""):
    results.append((name, ok))
    print(("PASS " if ok else "FAIL ") + name + ("" if ok else f"  ({detail})"))

async def page_with(browser, base, m, stored=None):
    pg = await (await browser.new_context(timezone_id="Europe/Amsterdam", viewport={"width": 720, "height": 820})).new_page()
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
    server = subprocess.Popen([sys.executable, "-m", "http.server", "8773"], cwd=root, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    time.sleep(1)
    base = "http://localhost:8773"
    try:
        async with async_playwright() as p:
            b = await p.chromium.launch(channel="chrome", args=["--no-sandbox", "--use-fake-device-for-media-stream", "--use-fake-ui-for-media-stream"])

            MAIN = [{"id": "main", "name": "Lock it", "birthday": 3100000, "watch_only": False, "main": True}]
            TWO = MAIN + [{"id": "w1", "name": "Shop", "birthday": 1687104, "watch_only": False, "view_only": True, "main": False},
                          {"id": "w2", "name": "Café", "birthday": 1687104, "watch_only": False, "view_only": True, "main": False},
                          {"id": "k1", "name": "Keystone", "birthday": 3600000, "watch_only": True, "main": False}]
            ACT = [{"txid": "ab", "height": 3609479, "change": 50000000, "time": 1795589352, "expired": False, "fee": None, "tex_receiver": None, "tex_step": None, "title": "Locked"}]
            async def business(pg):
                await pg.click("#open-settings"); await pg.wait_for_timeout(300)
                return await pg.text_content("#row-business")

            pg = await page_with(b, base, {"wallets": MAIN, "current": None})
            check("row: summary with none", (await business(pg)) == "Watch only · viewing key · CSV")
            await pg.close()

            pg = await page_with(b, base, {"wallets": TWO, "current": None, "activity": ACT})
            await pg.wait_for_timeout(4500)
            check("row: two watch-only wallets", (await business(pg)) == "2 watch-only wallets")
            await pg.click("[data-go=business]"); await pg.wait_for_timeout(300)
            page = await pg.inner_text("[data-page=business]")
            check("page: the Mac's watch text", "Add a wallet from its viewing key, such as your shop's" in page)
            listed = await pg.evaluate("[...document.querySelectorAll('#biz-watch-list li')].map(li => li.textContent)")
            check("page: lists the watch-only wallets only", listed == ["Shop", "Café"], listed)
            check("page: the not-encrypted note", "The file is not encrypted and shows every payment" in page)
            check("page: names the wallet to export", "Save every payment of the wallet This PC as a CSV file" in page)
            # The viewing key: after the keys, text and QR, hidden from capture.
            await pg.click("#biz-viewkey-toggle"); await pg.wait_for_timeout(200)
            check("viewing key: asks the keys first", await pg.is_visible("#confirm"))
            await pg.click("#confirm-go"); await pg.wait_for_timeout(400)
            check("viewing key: as text", (await pg.text_content("#biz-viewkey-text")) == "uview1shopkeyfortheTest")
            check("viewing key: as a QR code", await pg.is_visible("#biz-viewkey-qr"))
            check("viewing key: window hidden from capture", (await pg.evaluate("window.__m.capture")) == [True])
            await pg.click("#biz-viewkey-toggle"); await pg.wait_for_timeout(200)
            check("hidden again: text and QR gone", not await pg.is_visible("#biz-viewkey-text") and not await pg.is_visible("#biz-viewkey-qr"))
            check("hidden again: capture allowed again", (await pg.evaluate("window.__m.capture")) == [True, False])
            # Bookkeeping export.
            await pg.click("#biz-csv"); await pg.wait_for_timeout(300)
            check("export: saved note", (await pg.text_content("#biz-csv-note")) == "Saved zecky-this-pc-2026-11-25.csv")
            # Watch a wallet from here.
            await pg.click("#biz-watch"); await pg.wait_for_timeout(300)
            check("button: opens Watch a wallet", await pg.is_visible("#sheet-watch"))
            check("no page errors (business)", not pg.errors, pg.errors)
            await pg.close()

            # Settings › Security › Camera (Lock it Mac 54e0101).
            pg = await page_with(b, base, {"wallets": MAIN, "current": None, "cameraText": "Allowed · only while scanning"})
            await pg.click("#open-settings"); await pg.wait_for_timeout(300)
            await pg.click("[data-go=security]"); await pg.wait_for_timeout(300)
            check("camera: the state", (await pg.text_content("#camera-state")) == "Allowed · only while scanning")
            check("camera: the window's own permission is asked", (await pg.evaluate("window.__m.cameraAsked")) in ("prompt", "granted", "denied"))
            sec = await pg.inner_text("[data-page=security]")
            check("camera: the Mac's sentence, for Windows", "Lock it uses the camera only while you scan a code" in sec and "Windows Settings › Privacy & security › Camera" in sec)
            await pg.click("#camera-settings"); await pg.wait_for_timeout(200)
            check("camera: opens Windows Settings", await pg.evaluate("window.__m.cameraOpened === true"))
            check("no page errors (camera)", not pg.errors, pg.errors)
            await pg.close()

            # How Zecky works: For business.
            pg = await page_with(b, base, {"wallets": MAIN, "current": None})
            await pg.click("#open-settings"); await pg.wait_for_timeout(300)
            await pg.click("[data-go=how]"); await pg.wait_for_timeout(300)
            how = await pg.inner_text("[data-page=how]")
            check("How Zecky works: For business", "For business" in how and "Watch a shop wallet, share a viewing key, export." in how, how[:200])
            await pg.evaluate("[...document.querySelectorAll('[data-page=how] details, [data-page=how] button')].forEach(e => { if (e.tagName === 'DETAILS') e.open = true; })")
            check("How Zecky works: the camera sentence", "camera's light shows it" in await pg.evaluate("document.querySelector('[data-page=how]').textContent"))
            await pg.close()
    finally:
        server.terminate()
    failed = [n for n, ok in results if not ok]
    print(f"{len(results) - len(failed)} passed, {len(failed)} failed")
    sys.exit(1 if failed else 0)

asyncio.run(main())
