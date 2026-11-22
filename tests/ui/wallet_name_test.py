"""UI test: Receive and Send name the wallet they work on, under the title
(Lock it Mac 9dd3a9c), and Receive never shows the previous wallet's address.
Run from the app folder: python3 tests/ui/wallet_name_test.py"""
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
    server = subprocess.Popen([sys.executable, "-m", "http.server", "8797"], cwd=root, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    time.sleep(1)
    base = "http://localhost:8797"
    try:
        async with async_playwright() as p:
            b = await p.chromium.launch(channel="chrome", args=["--no-sandbox"])

            tag = lambda sel: pg.text_content(sel + " span")
            # Two wallets, Savings chosen: Receive names it.
            pg = await page_with(b, base, {"tAddresses": {"sav-2": "t1Savings", "main": "t1Main"}})
            await pg.click("#home-receive"); await pg.wait_for_timeout(300)
            check("receive: tag shown", await pg.is_visible("#receive-wallet"))
            check("receive: names Savings", (await tag("#receive-wallet")) == "Savings", await tag("#receive-wallet"))
            check("receive: under the title", await pg.evaluate("document.querySelector('#receive-wallet').previousElementSibling.classList.contains('sheet-title')"))
            await pg.click("#receive-tab-transparent"); await pg.wait_for_timeout(300)
            check("receive: Savings t-address", (await pg.text_content("#receive-t-address")) == "t1Savings")
            check("transparent tab keeps the tag", await pg.is_visible("#receive-wallet"))
            await pg.click("#sheet-receive [data-close]"); await pg.wait_for_timeout(300)

            # Switch to the main wallet, open Receive again: its own name and address.
            await pg.click(".ring-chip:not(.on)"); await pg.wait_for_timeout(400)
            await pg.click("#home-receive"); await pg.wait_for_timeout(300)
            check("after switch: names This PC", (await tag("#receive-wallet")) == "This PC", await tag("#receive-wallet"))
            await pg.click("#receive-tab-transparent"); await pg.wait_for_timeout(300)
            check("after switch: not the old t-address", (await pg.text_content("#receive-t-address")) == "t1Main", await pg.text_content("#receive-t-address"))
            check("no page errors (receive)", not pg.errors, pg.errors)
            await pg.close()

            # Send names the paying wallet under the title.
            pg = await page_with(b, base, {"lockOpen": True})
            await pg.click("#home-send"); await pg.wait_for_timeout(300)
            check("send: tag shown", await pg.is_visible("#send-wallet"))
            check("send: names Savings", (await tag("#send-wallet")) == "Savings", await tag("#send-wallet"))
            check("send: available line still names it", (await pg.text_content("#send-available")).endswith("in Savings"))
            check("no page errors (send)", not pg.errors, pg.errors)
            await pg.close()

            # A Seal it wallet gets the envelope, a PC wallet the lock.
            sealed = [{"id": "main", "name": "Lock it", "birthday": 3100000, "watch_only": False, "main": True},
                      {"id": "ph-1", "name": "Phone", "birthday": 3500000, "watch_only": True, "main": False}]
            pg = await page_with(b, base, {"wallets": sealed, "current": "ph-1"})
            await pg.click("#home-receive"); await pg.wait_for_timeout(300)
            check("signing phone wallet named", (await tag("#receive-wallet")) == "Phone")
            same_icon = await pg.evaluate("document.querySelector('#receive-wallet svg').outerHTML === document.querySelector('.ring-chip.on svg').outerHTML")
            check("signing phone wallet: envelope icon", same_icon)
            await pg.close()

            # One wallet: Receive and Send still name it, as on the Mac.
            single = [{"id": "main", "name": "Lock it", "birthday": 3100000, "watch_only": False, "main": True}]
            pg = await page_with(b, base, {"wallets": single, "current": None})
            await pg.click("#home-receive"); await pg.wait_for_timeout(300)
            check("one wallet: receive named", await pg.is_visible("#receive-wallet") and (await tag("#receive-wallet")) == "This PC")
            await pg.close()
    finally:
        server.terminate()
    failed = [n for n, ok in results if not ok]
    print(f"{len(results) - len(failed)} passed, {len(failed)} failed")
    sys.exit(1 if failed else 0)

asyncio.run(main())
