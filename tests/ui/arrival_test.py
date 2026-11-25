"""UI test: the message when Zcash arrives (Lock it Mac's ArrivalToast):
its words per kind, hidden amounts, one at a time, it waits while a sheet
is open, leaves by itself, and a click opens Activity for that wallet.
Run from the app folder: python3 tests/ui/arrival_test.py"""
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
    server = subprocess.Popen([sys.executable, "-m", "http.server", "8779"], cwd=root, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    time.sleep(1)
    base = "http://localhost:8779"
    try:
        async with async_playwright() as p:
            b = await p.chromium.launch(channel="chrome", args=["--no-sandbox", "--use-fake-device-for-media-stream", "--use-fake-ui-for-media-stream"])

            WALLETS = [{"id": "main", "name": "Lock it", "birthday": 3100000, "watch_only": False, "main": True},
                       {"id": "sav-2", "name": "Savings", "birthday": 3500000, "watch_only": False, "main": False}]
            def came(kind="locked_in", account="main", amount=25000000, txid="ab"):
                return {"txid": txid, "account": account, "kind": kind, "amount": amount}
            async def wait_shown(pg, ms=5000):
                for _ in range(ms // 100):
                    if await pg.is_visible("#arrival"):
                        return True
                    await pg.wait_for_timeout(100)
                return False

            # Zcash came in to the main wallet.
            pg = await page_with(b, base, {"wallets": WALLETS, "current": None, "arrivals": [came()]})
            check("shown", await wait_shown(pg))
            title, detail = await pg.text_content("#arrival-title"), await pg.text_content("#arrival-detail")
            check("title names the wallet", title == "Locked in · This PC", title)
            check("detail is the amount", detail.startswith("0.25") and "ZEC" in detail, detail)
            await pg.wait_for_timeout(5200)
            check("leaves by itself after a few seconds", not await pg.is_visible("#arrival"))
            check("no page errors (arrival)", not pg.errors, pg.errors)
            await pg.close()

            # Hidden amounts, and locking in.
            pg = await page_with(b, base, {"wallets": WALLETS, "current": None})
            await pg.click("#eye")
            await pg.evaluate("window.__m.arrivals = [{txid: 'h1', account: 'main', kind: 'locked_in', amount: 25000000}]")
            await wait_shown(pg)
            check("hidden amounts: Amount hidden", (await pg.text_content("#arrival-detail")) == "Amount hidden")
            await pg.close()
            pg = await page_with(b, base, {"wallets": WALLETS, "current": None, "arrivals": [came("shielded", amount=15000)]})
            await wait_shown(pg)
            check("lock-in: title", (await pg.text_content("#arrival-title")) == "Locked in ✓ · This PC")
            check("lock-in: detail", (await pg.text_content("#arrival-detail")) == "Your transparent Zcash is shielded now")
            await pg.close()

            # Two at once: one at a time.
            pg = await page_with(b, base, {"wallets": WALLETS, "current": None, "arrivals": [came(txid="a1"), came(account="sav-2", amount=10000000, txid="a2")]})
            await wait_shown(pg)
            check("first: main wallet", (await pg.text_content("#arrival-title")) == "Locked in · This PC")
            await pg.wait_for_timeout(5200)
            check("then the second", await wait_shown(pg, 5000) and (await pg.text_content("#arrival-title")) == "Locked in · Savings")
            await pg.close()

            # A sheet is open: it waits until it closes.
            pg = await page_with(b, base, {"wallets": WALLETS, "current": None})
            await pg.click("#home-receive"); await pg.wait_for_timeout(300)
            await pg.evaluate("window.__m.arrivals = [{txid: 'w1', account: 'main', kind: 'locked_in', amount: 5000000}]")
            await pg.wait_for_timeout(4600)
            check("waits while a sheet is open", not await pg.is_visible("#arrival"))
            await pg.click("#sheet-receive [data-close]")
            check("shown once the sheet closes", await wait_shown(pg, 5000))
            await pg.close()

            # A click opens Activity for that wallet.
            pg = await page_with(b, base, {"wallets": WALLETS, "current": None, "arrivals": [came(account="sav-2", txid="c1")]})
            await wait_shown(pg)
            await pg.click("#arrival"); await pg.wait_for_timeout(600)
            check("click: Activity open", await pg.is_visible("#sheet-activity"))
            check("click: that wallet chosen", (await pg.evaluate("window.__m.current")) == "sav-2")
            check("no page errors (click)", not pg.errors, pg.errors)
            await pg.close()
    finally:
        server.terminate()
    failed = [n for n, ok in results if not ok]
    print(f"{len(results) - len(failed)} passed, {len(failed)} failed")
    sys.exit(1 if failed else 0)

asyncio.run(main())
