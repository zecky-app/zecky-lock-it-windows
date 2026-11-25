"""UI test: Export CSV… in Activity (Lock it Mac 5ae8a7b, 9f5495a,
0c0f3cb): the button, off when there is nothing, each payment's own offset
(summer and winter time), the saved note and the may-miss note.
The file itself is tested in Rust (lockit-wallet csv).
Run from the app folder: python3 tests/ui/csv_test.py"""
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
    server = subprocess.Popen([sys.executable, "-m", "http.server", "8775"], cwd=root, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    time.sleep(1)
    base = "http://localhost:8775"
    try:
        async with async_playwright() as p:
            b = await p.chromium.launch(channel="chrome", args=["--no-sandbox", "--use-fake-device-for-media-stream", "--use-fake-ui-for-media-stream"])

            WINTER, SUMMER = 1795589352, 1782198552  # 25 Nov 2026, 24 Jun 2026
            ACTIVITY = [{"txid": "ab", "height": 3609479, "change": 50000000, "time": WINTER, "expired": False, "fee": None,
                         "tex_receiver": None, "tex_step": None, "title": "Locked"},
                        {"txid": "cd", "height": 3400000, "change": -1010000, "time": SUMMER, "expired": False, "fee": 10000,
                         "tex_receiver": None, "tex_step": None, "title": "Released"}]
            async def open_activity(pg):
                await pg.evaluate("document.querySelector('#home-activity, [data-open=activity]') && document.querySelector('#home-activity, [data-open=activity]').click()")
                if not await pg.is_visible("#sheet-activity"):
                    await pg.evaluate("renderActivity(); openSheet('activity')")
                await pg.wait_for_timeout(300)

            # Nothing yet: the button is off.
            pg = await page_with(b, base, {"activity": []})
            await open_activity(pg)
            check("empty: Export CSV… shown but off", await pg.is_visible("#export-csv") and await pg.is_disabled("#export-csv"))
            await pg.close()

            # Payments: saved, with each payment's own offset.
            pg = await page_with(b, base, {"activity": ACTIVITY, "current": None})
            await pg.wait_for_timeout(4500)
            await open_activity(pg)
            check("button on with payments", not await pg.is_disabled("#export-csv"))
            check("the Mac's hover text", (await pg.get_attribute("#export-csv", "title")).startswith("Save this wallet's payments as a CSV file for bookkeeping"))
            await pg.click("#export-csv"); await pg.wait_for_timeout(300)
            check("saved note", (await pg.text_content("#export-note")) == "Saved zecky-this-pc-2026-11-25.csv")
            sent = await pg.evaluate("window.__m.lastExport")
            check("winter time: +60", sent["offsets"].get(str(WINTER)) == 60, sent["offsets"])
            check("summer time: +120", sent["offsets"].get(str(SUMMER)) == 120, sent["offsets"])
            check("the wallet's name for the file", sent["name"] == "This PC")
            await pg.click("#tab-lock"); await pg.wait_for_timeout(200)
            check("the lock's log: no export", not await pg.is_visible("#export-csv"))
            check("no page errors (export)", not pg.errors, pg.errors)
            await pg.close()

            # Older blocks still checked: the file may miss payments.
            pg = await page_with(b, base, {"activity": ACTIVITY, "mayMiss": True})
            await pg.wait_for_timeout(4500)
            await open_activity(pg)
            await pg.click("#export-csv"); await pg.wait_for_timeout(300)
            check("may miss note", (await pg.text_content("#export-note")) == "Saved zecky-this-pc-2026-11-25.csv. Older blocks are still being checked; this file may miss payments.")
            await pg.close()

            # The Save dialog closed: no note.
            pg = await page_with(b, base, {"activity": ACTIVITY, "exportResult": None})
            await pg.wait_for_timeout(4500)
            await open_activity(pg)
            await pg.click("#export-csv"); await pg.wait_for_timeout(300)
            check("dialog closed: no note", (await pg.text_content("#export-note")) == "")
            check("button back", (await pg.text_content("#export-csv")) == "Export CSV…" and not await pg.is_disabled("#export-csv"))
            await pg.close()
    finally:
        server.terminate()
    failed = [n for n, ok in results if not ok]
    print(f"{len(results) - len(failed)} passed, {len(failed)} failed")
    sys.exit(1 if failed else 0)

asyncio.run(main())
