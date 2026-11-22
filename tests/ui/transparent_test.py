"""UI test for Receive › Transparent and the "Zcash arrived on your
transparent address" question (Lock it Mac 658dc7b).
Run from the app folder: python3 tests/ui/transparent_test.py"""
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
    server = subprocess.Popen([sys.executable, "-m", "http.server", "8798"], cwd=root, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    time.sleep(1)
    base = "http://localhost:8798"
    try:
        async with async_playwright() as p:
            b = await p.chromium.launch(channel="chrome", args=["--no-sandbox"])

            # Receive: Shielded first, then the Transparent tab.
            pg = await page_with(b, base, {})
            await pg.click("#home-receive"); await pg.wait_for_timeout(300)
            check("receive opens on the shielded tab", await pg.is_visible("#receive-shielded") and not await pg.is_visible("#receive-transparent"))
            await pg.click("#receive-tab-transparent"); await pg.wait_for_timeout(300)
            check("transparent tab shows the t1 address",
                  (await pg.text_content("#receive-t-address")) == "t1TransparentExampleAddress0000000")
            check("it is marked PUBLIC", await pg.is_visible(".public-badge") and (await pg.text_content(".public-badge")) == "PUBLIC")
            check("the title says transparent", (await pg.text_content("#sheet-receive .sheet-title")) == "Transparent address")
            check("a warning is shown", "Anyone can see" in (await pg.text_content(".public-warning")))
            check("no request or all-addresses options", await pg.query_selector("#receive-transparent [id*='request']") is None)
            await pg.click("#receive-t-back"); await pg.wait_for_timeout(200)
            check("back to the shielded address", await pg.is_visible("#receive-shielded") and (await pg.text_content("#sheet-receive .sheet-title")) == "Receive")
            check("no page errors (receive)", not pg.errors, pg.errors)
            await pg.close()

            # Zcash arrived: asked once.
            pg = await page_with(b, base, {"transparent": 5_000_000})
            check("asked when Zcash arrived", await pg.is_visible("#outside-prompt"))
            check("the question names it", "Zcash arrived on your transparent address" in (await pg.text_content("#outside-title")))
            await pg.click("#outside-later"); await pg.wait_for_timeout(200)
            check("Later closes it", not await pg.is_visible("#outside-prompt"))
            stored = await pg.evaluate("localStorage.getItem('outside.lastAsked')")
            check("remembered the amount asked about", stored == "5000000", stored)
            await pg.close()

            # The same amount again: not asked again. More: asked.
            pg = await page_with(b, base, {"transparent": 5_000_000}, stored=5_000_000)
            check("same amount: not asked again", not await pg.is_visible("#outside-prompt"))
            await pg.close()
            pg = await page_with(b, base, {"transparent": 7_000_000}, stored=5_000_000)
            check("more arrived: asked again", await pg.is_visible("#outside-prompt"))
            await pg.click("#outside-lockin"); await pg.wait_for_timeout(300)
            check("Lock it in opens the lock-in step", await pg.is_visible("#sheet-move"))
            await pg.close()

            # After locking in (amount went down), the next arrival is news again.
            pg = await page_with(b, base, {"transparent": 0}, stored=7_000_000)
            stored = await pg.evaluate("localStorage.getItem('outside.lastAsked')")
            check("locked in: the remembered amount goes down", stored == "0", stored)
            await pg.close()

            # Not while syncing.
            pg = await page_with(b, base, {"transparent": 5_000_000, "syncing": True})
            check("not asked while catching up", not await pg.is_visible("#outside-prompt"))
            check("no page errors", not pg.errors, pg.errors)
            await pg.close()
            await b.close()
    finally:
        server.terminate()
    failed = [n for n, ok in results if not ok]
    print(f"{len(results) - len(failed)} passed, {len(failed)} failed")
    sys.exit(1 if failed else 0)

asyncio.run(main())
