"""UI tests for Lock it Mac build 6 parity (items 1, 3-9), against a mock
backend. Run from the app folder:  python3 tests/ui/build6_test.py
Needs Python Playwright with Chrome. Serves ui/ and tests/ui/ over http."""
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

async def main():
    root = tempfile.mkdtemp()
    shutil.copytree(os.path.join(APP, "ui"), os.path.join(root, "ui"))
    shutil.copy(os.path.join(HERE, "mock.js"), os.path.join(root, "mock.js"))
    index = os.path.join(root, "ui", "index.html")
    html = open(index).read().replace("<head>", '<head><script src="../mock.js"></script>', 1)
    open(index, "w").write(html)
    server = subprocess.Popen([sys.executable, "-m", "http.server", "8799"], cwd=root, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    time.sleep(1)
    base = "http://localhost:8799"
    fine = {"state": {"state": "fine"}, "text": None, "ask": False, "choice": None, "date": None}
    try:
        async with async_playwright() as p:
            b = await p.chromium.launch(channel="chrome", args=["--no-sandbox"])

            # 5. Node stops while starting: calm line, Try again, no generic problem.
            pg = await page_with(b, base, {"goMax": {"state": "off", "installed": True, "problem": "zakurad stopped unexpectedly (exit code 1)", "stopped_while_starting": True, "retrying": False, "has_data": True, "support": fine}})
            await pg.click("#go-max-coin"); await pg.wait_for_timeout(400)
            text = await pg.text_content("#gm-stopped-text")
            check("5 calm line", text == "The node stopped while starting. Your wallet is fine: Lock it reads from its server meanwhile.", text)
            check("5 generic problem hidden", (await pg.text_content("#node-message")) == "")
            check("5 Try again shown", await pg.is_visible("#gm-try-again"))
            await pg.click("#gm-still-toggle")
            still = await pg.text_content("#gm-still")
            check("5 still not working", still.startswith("Quit Lock it and open it again.") and await pg.is_visible("#gm-still"), still)
            check("5 remove node offered", await pg.is_visible("#gm-remove"))
            check("5 coin calm", (await pg.text_content("#coin-sub")) == "Stopped while starting")
            await pg.close()
            pg = await page_with(b, base, {"goMax": {"state": "off", "installed": True, "problem": None, "stopped_while_starting": True, "retrying": True, "has_data": True, "support": fine}})
            await pg.click("#go-max-coin"); await pg.wait_for_timeout(400)
            check("5 trying again", (await pg.text_content("#gm-stopped-text")) == "Trying again in half a minute…" and not await pg.is_visible("#gm-try-again"))
            await pg.close()

            # 1, 3, 4: the notes from the sync, on the Go Max page.
            on = {"state": "on", "wallet_ready": True, "wallet_note": None, "onion": None, "onion_problem": None,
                  "status": {"height": 3600000, "headers": 3600000, "estimated_height": 3600001, "progress": 1.0, "peers": 9, "inbound_peers": 0, "pruned": True, "size_on_disk": 2e10},
                  "reading_note": "Lock it reads older blocks (from block 2,900,000) from zec.rocks: this node keeps only recent blocks. It returns to this node by itself once they are checked.",
                  "wallet_quiet": "Lock it is not getting new blocks from this node. It has connected again; if this line stays, switch Go Max off and on.",
                  "unsettled": "Lock it could not connect to zec.rocks yet · trying again.", "support": fine}
            pg = await page_with(b, base, {"goMax": on})
            await pg.click("#go-max-coin"); await pg.wait_for_timeout(400)
            check("1 older blocks line", (await pg.text_content("#gm-reading")) == on["reading_note"] and await pg.is_visible("#gm-reading"))
            check("4 quiet line", (await pg.text_content("#gm-quiet")) == on["wallet_quiet"] and await pg.is_visible("#gm-quiet"))
            check("3 unsettled line", (await pg.text_content("#gm-unsettled")) == on["unsettled"])
            check("4 coin quiet", (await pg.text_content("#coin-sub")) == "No new blocks · check connection")
            await pg.close()

            # 6. The stop: notice two weeks ahead; at the stop, the choice.
            soon = dict(on, reading_note=None, wallet_quiet=None, unsettled=None,
                        support={"state": {"state": "soon", "at": 1796472000}, "text": "Go Max needs an update before Dec 5. This node version follows the chain up to block 3,643,309; a newer Lock it brings the next one.", "ask": False, "choice": None, "date": "Dec 5"})
            pg = await page_with(b, base, {"goMax": soon})
            check("6 coin update before", (await pg.text_content("#coin-sub")) == "Update before Dec 5")
            await pg.click("#go-max-coin"); await pg.wait_for_timeout(400)
            check("6 notice", (await pg.text_content("#gm-support")) == soon["support"]["text"])
            check("6 no choice before the stop", not await pg.is_visible("#gm-choice"))
            await pg.close()
            ended = {"state": "off", "installed": True, "problem": None, "stopped_while_starting": False, "retrying": False, "has_data": True,
                     "support": {"state": {"state": "ended"}, "text": "This node version follows the Zcash chain up to block 3,643,309, and the network is now past it. Go Max carries on after an update of Lock it; the node's data is kept. Your Zcash is not affected.", "ask": True, "choice": None, "date": None}}
            pg = await page_with(b, base, {"goMax": ended})
            await pg.click("#go-max-coin"); await pg.wait_for_timeout(400)
            choice = await pg.text_content("#gm-choice-text")
            check("6 asks at the stop", "Nothing is switched without you." in choice and await pg.is_visible("#gm-wait") and await pg.is_visible("#gm-use-server"), choice)
            check("6 coin needs update", (await pg.text_content("#coin-sub")) == "Needs an update")
            await pg.click("#gm-wait"); await pg.wait_for_timeout(200)
            check("6 wait is sent", ["go_max_choose_at_stop", {"choice": "wait"}] in await pg.evaluate("window.__calls"))
            await pg.close()

            # 7. Send names the paying wallet (two wallets), and not with one.
            pg = await page_with(b, base, {"lockOpen": True})
            await pg.click("#home-send"); await pg.wait_for_timeout(300)
            check("7 available in", (await pg.text_content("#send-available")) == "0.0224 ZEC available in Savings", await pg.text_content("#send-available"))
            await pg.fill("#send-to", "u1test"); await pg.fill("#send-amount", "0.01")
            await pg.click("#send-next"); await pg.wait_for_timeout(400)
            check("7 receipt from", await pg.is_visible("#plan-from-row") and (await pg.text_content("#plan-from")) == "Savings")
            check("7 button from", (await pg.text_content("#send-next")) == "Send 0.010 ZEC from Savings", await pg.text_content("#send-next"))
            await pg.close()
            single = [{"id": "main", "name": "Lock it", "birthday": 3100000, "watch_only": False, "main": True}]
            pg = await page_with(b, base, {"lockOpen": True, "wallets": single, "current": None})
            await pg.click("#home-send"); await pg.wait_for_timeout(300)
            check("7 one wallet: no name", (await pg.text_content("#send-available")) == "0.0224 ZEC available")
            await pg.fill("#send-to", "u1test"); await pg.fill("#send-amount", "0.01")
            await pg.click("#send-next"); await pg.wait_for_timeout(400)
            check("7 one wallet: plain button", (await pg.text_content("#send-next")) == "Send 0.010 ZEC" and not await pg.is_visible("#plan-from-row"))
            await pg.close()

            # 8. One lock for all wallets: the capsule on the prompt, the words in Settings.
            pg = await page_with(b, base, {"goMax": dict(fine and {"state": "off", "installed": True, "problem": None, "stopped_while_starting": False, "retrying": False, "has_data": False, "support": fine})})
            await pg.evaluate("overview.settings.wait = 10")
            await pg.click("#padlock"); await pg.wait_for_timeout(300)
            check("8 capsule names the wallet", await pg.is_visible("#prompt-wallet") and (await pg.text_content("#prompt-wallet")) == "Savings")
            body = await pg.content()
            check("8 settings say every wallet", "The keys and the time lock apply to every wallet on this PC." in body and "It applies to every wallet on this PC." in body)
            await pg.close()

            # 9. Keep out of sight: labels, star, locking hides them again.
            pg = await page_with(b, base, {"hidden": ["sav-2"], "current": None})
            chips = await pg.evaluate("[...document.querySelectorAll('#key-ring .ring-chip')].map(c => c.textContent)")
            check("9 hidden wallet out of the row", chips == ["This PC"], chips)
            await pg.click("#key-ring .ring-round:not(.plus)"); await pg.wait_for_timeout(200)
            await pg.click(".ring-chip:nth-child(2)", button="right")
            labels = await pg.evaluate("[...document.querySelectorAll('#ring-menu button')].map(b => b.textContent)")
            check("9 keep in sight", "Keep in sight" in labels, labels)
            await pg.keyboard.press("Escape")
            await pg.evaluate("showAppLocked()"); await pg.wait_for_timeout(200)
            chips = await pg.evaluate("[...document.querySelectorAll('#key-ring .ring-chip')].map(c => c.textContent)")
            check("9 locking hides them", chips == ["This PC"], chips)
            await pg.close()

            await b.close()
    finally:
        server.terminate()
        shutil.rmtree(root, ignore_errors=True)
    failed = [n for n, ok in results if not ok]
    print(f"\n{len(results) - len(failed)} passed, {len(failed)} failed")
    sys.exit(1 if failed else 0)

asyncio.run(main())
