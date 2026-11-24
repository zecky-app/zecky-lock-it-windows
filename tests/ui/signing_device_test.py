"""UI test: signing devices (Cupcake or Keystone), as Lock it Mac: the words
on every screen of pairing and sending, the Sync screen's "Checked from",
and no Transparent tab for a signing-device wallet.
Run from the app folder: python3 tests/ui/signing_device_test.py"""
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
    server = subprocess.Popen([sys.executable, "-m", "http.server", "8783"], cwd=root, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    time.sleep(1)
    base = "http://localhost:8783"
    try:
        async with async_playwright() as p:
            b = await p.chromium.launch(channel="chrome", args=["--no-sandbox", "--use-fake-device-for-media-stream", "--use-fake-ui-for-media-stream"])

            KEYSTONE = [{"id": "main", "name": "Lock it", "birthday": 3100000, "watch_only": False, "main": True},
                        {"id": "ks-1", "name": "Keystone · Account 0", "birthday": 3600000, "watch_only": True, "main": False}]
            bad_words = ("Seal it", "phone")
            def clean(text):
                return not any(w.lower() in text.lower() for w in bad_words)

            # Pairing: the menu and every text say signing device, Cupcake or Keystone.
            pg = await page_with(b, base, {})
            await pg.click(".ring-round.plus")
            menu = await pg.evaluate("[...document.querySelectorAll('button')].map(b => b.textContent).filter(t => t.includes('Pair a'))")
            check("menu: Pair a signing device (Cupcake or Keystone)", any("Pair a signing device (Cupcake or Keystone)" in t for t in menu), menu)
            await pg.click("text=Pair a signing device")
            await pg.wait_for_timeout(300)
            await pg.wait_for_timeout(300)
            sheet = await pg.inner_text("#sheet-seal")
            check("pair: title", (await pg.text_content("#seal-title")) == "Pair a signing device")
            check("pair: names Cupcake and Keystone", "Cupcake" in sheet and "Keystone" in sheet)
            check("pair: Keystone's menu named", "Connect Software Wallet" in sheet)
            check("pair: no Seal it, no phone", clean(sheet), repr(sheet))
            await pg.evaluate("document.querySelector('[data-seal=age]').hidden = false")
            age = await pg.inner_text("[data-seal=age]")
            check("pair: the start question has no phone in it", clean(age), age)
            check("no page errors (pair)", not pg.errors, pg.errors)
            await pg.close()

            # A Keystone wallet: home, chip and the send screens.
            pg = await page_with(b, base, {"wallets": KEYSTONE, "current": "ks-1"})
            check("home: Send with your signing device", (await pg.text_content("#home-seal-send")) == "Send with your signing device")
            chip = await pg.get_attribute(".ring-chip.on", "title")
            check("chip: Cupcake or Keystone", chip == "Signed on your signing device (Cupcake or Keystone)", chip)
            await pg.click("#home-seal-send"); await pg.wait_for_timeout(300)
            check("send: title names the wallet", (await pg.text_content("#seal-title")) == "Send from Keystone · Account 0")
            check("send: no Seal it, no phone", clean(await pg.inner_text("#sheet-seal")))
            await pg.fill("#seal-to", "u1example"); await pg.fill("#seal-amount", "0.01")
            await pg.click("#seal-work"); await pg.wait_for_timeout(300)
            check("send: show button", (await pg.text_content("#seal-show")) == "Show it to my signing device")
            await pg.click("#seal-show"); await pg.wait_for_timeout(400)
            shown = await pg.text_content("#seal-show-text")
            check("show: Keystone instructions", "Keystone" in shown and clean(shown), shown)
            check("show: signed button", (await pg.text_content("#seal-signed")) == "It is signed")
            check("no page errors (Keystone send)", not pg.errors, pg.errors)
            await pg.close()

            # Sync screen: one scan for all wallets, so "Checked from" the
            # earliest start, also with the later Keystone wallet selected.
            pg = await page_with(b, base, {"wallets": KEYSTONE, "current": "ks-1", "scanFrom": 3100000})
            await pg.click("#status-text"); await pg.wait_for_timeout(300)
            sync = await pg.inner_text("#sheet-sync")
            check("sync: Checked from, not Wallet birthday", "Checked from" in sync and "Wallet birthday" not in sync)
            check("sync: the shared start block", "Block 3,100,000" in sync, sync[:300])
            check("sync: up to date for all wallets", (await pg.text_content("#sync-ring-note")) == "Every block since block 3,100,000, for all wallets on this PC")
            check("no page errors (sync)", not pg.errors, pg.errors)
            await pg.close()

            # Cupcake keeps its own words (Cupcake is a phone app).
            pg = await page_with(b, base, {"wallets": KEYSTONE, "current": "ks-1", "cupcakeSigner": True})
            await pg.click("#home-seal-send"); await pg.wait_for_timeout(300)
            await pg.fill("#seal-to", "u1example"); await pg.fill("#seal-amount", "0.01")
            await pg.click("#seal-work"); await pg.wait_for_timeout(300)
            await pg.click("#seal-show"); await pg.wait_for_timeout(400)
            shown = await pg.text_content("#seal-show-text")
            check("show: Cupcake instructions", shown.startswith("In Cupcake") and "Seal it" not in shown, shown)
            await pg.close()
    finally:
        server.terminate()
    failed = [n for n, ok in results if not ok]
    print(f"{len(results) - len(failed)} passed, {len(failed)} failed")
    sys.exit(1 if failed else 0)

asyncio.run(main())
