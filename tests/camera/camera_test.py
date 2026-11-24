"""Camera test: Lock it's QR scan loop reads a moving code from a webcam.

A moving code (Keystone style: 6 KB, fountain parts, 5 frames a second) is
filmed as a 1280x720 webcam picture: about 240 px wide, tilted, blurred,
noisy. Chrome plays it as a fake camera; Lock it's own scan loop reads it.
Passes when at least 80% of the frames shown are read, and nothing is
loaded from outside Lock it. Then `cargo test -p lockit-wallet keystone_scan`
checks that such parts complete a message.

Needs: node + npm (installs bc-ur and qrcode here), ffmpeg, Chrome, Playwright.
Run from the app folder: python3 tests/camera/camera_test.py"""
import asyncio, json, os, shutil, subprocess, sys, tempfile, time
from playwright.async_api import async_playwright

HERE = os.path.dirname(os.path.abspath(__file__))
APP = os.path.dirname(os.path.dirname(HERE))
SECONDS, FPS = 20, 5

def make_video(work):
    if not os.path.isdir(os.path.join(HERE, "node_modules")):
        subprocess.run(["npm", "install", "--silent", "--no-audit", "--no-fund"], cwd=HERE, check=True)
    frames = os.path.join(work, "frames")
    info = json.loads(subprocess.run(["node", os.path.join(HERE, "make_frames.js"), frames, "6000", "keystone", "120"],
                                     cwd=HERE, check=True, capture_output=True, text=True).stdout)
    video = os.path.join(work, "camera.mjpeg")
    subprocess.run(["ffmpeg", "-loglevel", "error", "-y", "-framerate", str(FPS), "-i", os.path.join(frames, "f%04d.png"),
                    "-f", "lavfi", "-i", "color=c=0x8a8f96:s=1280x720:r=30", "-filter_complex",
                    "[0:v]fps=30,scale=240:240,format=rgba,rotate=4*PI/180:c=0x8a8f96:ow=rotw(4*PI/180):oh=roth(4*PI/180),"
                    "eq=contrast=0.75:brightness=0.03[q];[1:v][q]overlay=(W-w)/2+60:(H-h)/2-20:shortest=1,"
                    "gblur=sigma=1.6,noise=alls=10:allf=t,format=yuvj420p", "-t", str(SECONDS + 4), "-q:v", "5", video], check=True)
    return video, info, json.load(open(os.path.join(frames, "texts.json")))

async def scan(video, base):
    async with async_playwright() as p:
        browser = await p.chromium.launch(channel="chrome", args=["--no-sandbox", "--use-fake-ui-for-media-stream",
                                          "--use-fake-device-for-media-stream", "--use-file-for-fake-video-capture=" + video])
        page = await (await browser.new_context(viewport={"width": 720, "height": 820}, permissions=["camera"])).new_page()
        errors, outside = [], []
        page.on("pageerror", lambda e: errors.append(str(e)))
        page.on("request", lambda r: None if r.url.startswith(base) else outside.append(r.url))
        await page.add_init_script("window.__m = {};")
        await page.goto(base + "/ui/index.html")
        await page.add_style_tag(content="*{animation:none!important;transition:none!important}")
        await page.wait_for_timeout(900)
        await page.click(".ring-round.plus")
        await page.click("text=Pair a signing")
        await page.wait_for_timeout(SECONDS * 1000)
        reads = await page.evaluate("window.__m.scanned || []")
        zxing = await page.evaluate("zxingReady()")
        await browser.close()
        return reads, errors, outside, zxing

def main():
    work = tempfile.mkdtemp()
    root = os.path.join(work, "site")
    os.makedirs(root)
    shutil.copytree(os.path.join(APP, "ui"), os.path.join(root, "ui"))
    shutil.copy(os.path.join(APP, "tests", "ui", "mock.js"), os.path.join(root, "mock.js"))
    index = os.path.join(root, "ui", "index.html")
    html = open(index).read().replace("<head>", '<head><script src="../mock.js"></script>', 1)
    open(index, "w").write(html)
    video, info, texts = make_video(work)
    server = subprocess.Popen([sys.executable, "-m", "http.server", "8789"], cwd=root, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    time.sleep(1)
    try:
        reads, errors, outside, zxing = asyncio.run(scan(video, "http://localhost:8789"))
    finally:
        server.terminate()
        shutil.rmtree(work, ignore_errors=True)
    known = set(texts)
    good = [t for _, t in reads if t in known]
    shown = SECONDS * FPS
    results = [
        ("ZXing started (no fallback to jsQR)", zxing is True, ""),
        ("every read is a real part", len(good) == len(reads), f"{len(reads) - len(good)} not"),
        ("at least 80% of the frames shown were read", len(good) >= 0.8 * shown, f"{len(good)} of about {shown}"),
        ("nothing loaded from outside Lock it", not outside, outside[:3]),
        ("no page errors", not errors, errors[:3]),
    ]
    for name, ok, detail in results:
        print(("PASS " if ok else "FAIL ") + name + (f" ({detail})" if detail != "" else ""))
    print(f"{sum(ok for _, ok, _ in results)}/{len(results)} passed; {len(good)} parts read in {SECONDS} s ({info['fragments']} parts make the message)")
    sys.exit(0 if all(ok for _, ok, _ in results) else 1)

main()
