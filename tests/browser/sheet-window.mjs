// Run after `env -u NO_COLOR trunk build --public-url /`.
import assert from "node:assert/strict";
import { createServer } from "node:http";
import { readFile, writeFile } from "node:fs/promises";
import { resolve, extname } from "node:path";
import { spawn } from "node:child_process";

const root = process.cwd();
const port = Number(process.env.PORT ?? 8141);
const driverPort = Number(process.env.WEBDRIVER_PORT ?? 9531);
const base = `http://127.0.0.1:${port}`;
const mime = { ".js": "text/javascript", ".html": "text/html", ".css": "text/css", ".wasm": "application/wasm" };
const server = createServer(async (req, res) => {
    const path = new URL(req.url, base).pathname;
    if (path === "/storage-tests.html") {
        res.writeHead(200, { "content-type": "text/html" }).end("<!doctype html><title>Storage tests</title>");
        return;
    }
    const dir = path.startsWith("/src/storage/") ? root : resolve(root, "dist");
    const relative = path.startsWith("/RustyCalc/") ? path.slice(11) : path.slice(1);
    const file = resolve(dir, relative || "index.html");
    if (!file.startsWith(`${dir}/`)) { res.writeHead(403).end(); return; }
    try {
        const data = await readFile(file);
        res.writeHead(200, { "content-type": mime[extname(file)] ?? "application/octet-stream" });
        res.end(data);
    } catch { res.writeHead(404).end(); }
});
let session;
let driver;
async function wd(path, body, method = body === undefined ? "GET" : "POST") {
    const response = await fetch(`http://127.0.0.1:${driverPort}${path}`, {
        method, headers: { "content-type": "application/json" }, body: body === undefined ? undefined : JSON.stringify(body),
    });
    const result = await response.json();
    if (!response.ok || result.value?.error) throw new Error(JSON.stringify(result.value));
    return result.value;
}
const run = (script, args = []) => wd(`/session/${session}/execute/sync`, { script, args });
const runAsync = (script, args = []) => wd(`/session/${session}/execute/async`, {
    script: `const done = arguments[arguments.length - 1]; (async () => { ${script} })().then(value => done({value}), error => done({error: String(error), stack: error.stack}));`, args,
}).then(result => { assert.ok(!result.error, JSON.stringify(result)); return result.value; });
async function until(check, description) {
    const end = Date.now() + 30000;
    while (Date.now() < end) {
        if (await check()) return;
        await new Promise(resolve => setTimeout(resolve, 100));
    }
    throw new Error(`Timed out: ${description}`);
}
try {
    await new Promise((resolve, reject) => { server.once("error", reject); server.listen(port, "127.0.0.1", resolve); });
    driver = spawn("chromedriver", [`--port=${driverPort}`, "--allowed-ips=127.0.0.1"], { stdio: "ignore" });
    await until(async () => { try { return (await wd("/status")).ready; } catch { return false; } }, "ChromeDriver");
    session = (await wd("/session", { capabilities: { alwaysMatch: { browserName: "chrome", "goog:chromeOptions": {
        args: ["--headless=new", "--no-sandbox", "--disable-dev-shm-usage", "--window-size=1440,1000"],
    } } } })).sessionId;
    await wd(`/session/${session}/timeouts`, { script: 60000 });
    await wd(`/session/${session}/url`, { url: base });
    await until(() => run("return !!document.querySelector('.ws-grid')"), 'workbook');
    await run("[...document.querySelectorAll('.tb-tab')].find(el => el.textContent.trim() === 'View').click()");
    await until(() => run("return [...document.querySelectorAll('.tb-btn')].some(el => el.textContent.trim() === 'Sheet window')"), 'View controls');
    const openWindow = () => run("[...document.querySelectorAll('.tb-btn')].find(el => el.textContent.trim() === 'Sheet window').click()");
    await openWindow();
    await until(() => run("return document.querySelector('.sw-canvas')?.width > 300"), 'painted sheet window');
    await run("window.beforeWindow = document.querySelector('.sw-canvas').toDataURL()");
    await run(`
        const field = document.querySelector('#formula-bar textarea');
        field.focus(); field.value = '12345';
        field.dispatchEvent(new Event('input', {bubbles:true}));
        field.dispatchEvent(new KeyboardEvent('keydown', {key:'Enter', bubbles:true, cancelable:true}));
    `);
    await until(() => run("return document.querySelector('.sw-canvas').toDataURL() !== window.beforeWindow"), 'live edit in window');
    await run("window.mainPixels = document.querySelector('.ws-grid').toDataURL()");
    await run(`
        const canvas = document.querySelector('.sw-canvas');
        canvas.focus();
        for (const key of ['PageDown', 'ArrowRight', 'Delete', 'x']) {
            canvas.dispatchEvent(new KeyboardEvent('keydown', {key, bubbles:true, cancelable:true}));
        }
    `);
    await until(() => run("return document.querySelector('.sw-footer').textContent.includes('Row 21 · Column 2')"), 'independent scroll');
    assert.equal(await run("return document.querySelector('.ws-grid').toDataURL() === window.mainPixels"), true, 'window input must not change the main grid');
    assert.equal(await run("return document.querySelector('.ce')?.value ?? ''"), '', 'window typing must not start cell editing');
    const beforeDrag = await run("const r = document.querySelector('.sw').getBoundingClientRect(); return {x:r.x, y:r.y}");
    await run(`
        const title = document.querySelector('.sw-title');
        const r = title.getBoundingClientRect();
        title.dispatchEvent(new MouseEvent('mousedown', {clientX:r.x+10, clientY:r.y+10, button:0, buttons:1, bubbles:true}));
        window.dispatchEvent(new MouseEvent('mousemove', {clientX:r.x+90, clientY:r.y+50, buttons:1}));
        window.dispatchEvent(new MouseEvent('mouseup', {buttons:0}));
    `);
    await until(() => run("const r = document.querySelector('.sw').getBoundingClientRect(); return r.x > arguments[0].x + 50 && r.y > arguments[0].y + 20", [beforeDrag]), 'dragged window');
    const beforeResize = await run("const c=document.querySelector('.sw-canvas'); return {width:c.width, height:c.height}");
    await run("const w=document.querySelector('.sw'); w.style.width='720px'; w.style.height='480px'");
    await until(() => run("const c=document.querySelector('.sw-canvas'); return c.width > arguments[0].width && c.height > arguments[0].height", [beforeResize]), 'resized canvas');
    await run("document.querySelector('.sw-canvas').dispatchEvent(new KeyboardEvent('keydown', {key:'Home', bubbles:true, cancelable:true}))");
    await until(() => run("return document.querySelector('.sw-footer').textContent.includes('Row 1 · Column 1')"), 'home scroll');
    const screenshot = await wd(`/session/${session}/screenshot`);
    await writeFile('/tmp/rustycalc-sheet-window.png', Buffer.from(screenshot, 'base64'));
    await run("document.querySelector('[aria-label=\"Close sheet window\"]').click()");
    await until(() => run("return !document.querySelector('.sw')"), 'closed window');
    await openWindow();
    await until(() => run("return !!document.querySelector('.sw-canvas')"), 'reopened window');
    await run("document.querySelector('.sw-canvas').dispatchEvent(new KeyboardEvent('keydown', {key:'Escape', bubbles:true, cancelable:true}))");
    await until(() => run("return !document.querySelector('.sw')"), 'Escape close');
    console.log('PASS: View button, live edit, independent scroll, read-only keyboard, drag, resize, close, reopen, and Escape');
} finally {
    if (session) await wd(`/session/${session}`, undefined, 'DELETE').catch(() => {});
    driver?.kill();
    server.close();
}
