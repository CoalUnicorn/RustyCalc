// Run after `env -u NO_COLOR trunk build`. Uses system ChromeDriver and Chromium.
import assert from "node:assert/strict";
import { createServer } from "node:http";
import { readFile } from "node:fs/promises";
import { resolve, extname } from "node:path";
import { spawn } from "node:child_process";

const root = process.cwd();
const port = Number(process.env.PORT ?? 8137);
const driverPort = Number(process.env.WEBDRIVER_PORT ?? 9527);
const fixtureName = process.env.RCAL_FIXTURE ?? "legacy-93f3407.rcal";
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
    await wd(`/session/${session}/url`, { url: `${base}/storage-tests.html` });
    const result = await runAsync(`
        const {createStorage} = await import('/src/storage/indexeddb.js');
        const check = (value, message) => { if (!value) throw new Error(message); };
        const rejects = async (promise, message) => {
            try { await promise; } catch { return; }
            throw new Error(message);
        };
        const id = '550e8400-e29b-41d4-a716-446655440000';
        const fixture = new Uint8Array(await (await fetch(arguments[0])).arrayBuffer());
        const original = { name: 'Legacy workbook', group: 'Old group', modified: 1234, shared_from_link: true };
        const encoded = JSON.stringify(btoa(String.fromCharCode(...fixture)));
        const entries = new Map([['models', JSON.stringify({[id]: original})], [id, encoded]]);
        const legacy = { getItem: key => entries.get(key) ?? null, removeItem: key => entries.delete(key) };
        const db = createStorage('rustycalc-browser-tests', legacy);
        await db.initialize();
        check(JSON.stringify(JSON.parse(db.registry())[id]) === JSON.stringify(original), 'migration metadata');
        check((await db.load(id)).every((value, index) => value === fixture[index]), 'migration bytes');
        check(!entries.has('models') && !entries.has(id), 'cleanup after commit');
        await db.close();
        await db.initialize();
        check((await db.load(id)).length === fixture.length, 'reopen');
        const large = new Uint8Array(6_000_000).fill(17);
        await db.save(id, 'Large', large, 'existing');
        check((await db.load(id)).length === large.length, 'larger than old localStorage limit');
        await db.update(id, JSON.stringify({group: 'New group', shared_from_link: false}));
        await db.save(id, 'Renamed', fixture, 'existing');
        check(JSON.parse(db.registry())[id].group === 'New group', 'save preserves group');
        check(!JSON.parse(db.registry())[id].shared_from_link, 'save preserves trust');
        const before = db.registry();
        const put = IDBObjectStore.prototype.put;
        IDBObjectStore.prototype.put = function(...args) {
            if (this.name === 'workbook_bytes') throw new DOMException('Injected quota failure', 'QuotaExceededError');
            return put.apply(this, args);
        };
        try { await rejects(db.save(id, 'Must roll back', large, 'existing'), 'write should fail'); }
        finally { IDBObjectStore.prototype.put = put; }
        check(db.registry() === before, 'cache unchanged after abort');
        await db.close();
        await db.initialize();
        check(db.registry() === before, 'metadata rollback');
        check((await db.load(id)).length === fixture.length, 'bytes rollback');
        const writes = [db.save(id, 'First', new Uint8Array([1]), 'existing'), db.save(id, 'Second', new Uint8Array([2]), 'existing')];
        await Promise.all(writes);
        check((await db.load(id))[0] === 2 && JSON.parse(db.registry())[id].name === 'Second', 'ordered saves');
        await Promise.all([db.save(id, 'Last', fixture, 'existing'), db.delete(id)]);
        await rejects(db.save(id, 'Stale', fixture, 'existing'), 'deleted workbook must not return');
        check(await db.load(id) === undefined && !JSON.parse(db.registry())[id], 'delete both stores');
        entries.set('models', JSON.stringify({[id]: original})); entries.set(id, encoded);
        await db.close(); await db.initialize();
        check(await db.load(id) === undefined, 'migration marker prevents resurrection');
        await db.save(id, 'Recovered queue', fixture, 'shared');
        check(JSON.parse(db.registry())[id].shared_from_link, 'queue recovers after failure');
        await db.close();

        const badEntries = new Map([['models', JSON.stringify({[id]: original})], [id, JSON.stringify('%%%')]]);
        const bad = createStorage('rustycalc-bad-migration', {getItem: key => badEntries.get(key) ?? null, removeItem: key => badEntries.delete(key)});
        await rejects(bad.initialize(), 'bad migration must fail');
        check(badEntries.has(id) && badEntries.has('models'), 'bad migration preserves legacy');
        badEntries.set(id, encoded);
        await bad.initialize();
        check((await bad.load(id)).length === fixture.length, 'retry migration');
        await bad.close();

        const abortedEntries = new Map([['models', JSON.stringify({[id]: original})], [id, encoded]]);
        const aborted = createStorage('rustycalc-aborted-migration', {getItem: key => abortedEntries.get(key) ?? null, removeItem: key => abortedEntries.delete(key)});
        IDBObjectStore.prototype.put = function(...args) {
            if (this.name === 'workbook_bytes') throw new DOMException('Injected quota failure', 'QuotaExceededError');
            return put.apply(this, args);
        };
        try { await rejects(aborted.initialize(), 'migration transaction must fail'); }
        finally { IDBObjectStore.prototype.put = put; }
        check(abortedEntries.has(id) && abortedEntries.has('models'), 'failed migration retains source');
        await aborted.initialize();
        check((await aborted.load(id)).length === fixture.length, 'failed migration does not mark complete');
        await aborted.close();

        localStorage.setItem('models', JSON.stringify({[id]: original}));
        localStorage.setItem(id, encoded);
        localStorage.setItem('selected', JSON.stringify(id));
        return 'PASS: migration, reopen, 6 MB bytes, metadata preservation, rollback, ordered saves, deletion, retry, and migration marker';
    `, [`/src/storage/fixtures/${fixtureName}`]);
    console.log(result);
    await wd(`/session/${session}/url`, { url: `${base}/RustyCalc/` });
    await until(() => run("return !!document.querySelector('.workbook')"), "app startup");
    console.log('PASS: mounted app starts with migrated legacy workbook');
    assert.equal(await run("return localStorage.getItem('models')"), null);
    assert.equal(await run("return document.querySelector('#app').inert"), false);
    const selected = () => run("return JSON.parse(localStorage.getItem('selected'))");
    const initial = await selected();
    const records = () => runAsync(`
        const request = indexedDB.open('rustycalc', 1);
        const db = await new Promise((resolve, reject) => { request.onsuccess = () => resolve(request.result); request.onerror = () => reject(request.error); });
        const tx = db.transaction(['workbooks', 'workbook_bytes'], 'readonly');
        const read = store => new Promise((resolve, reject) => { const req = tx.objectStore(store).getAll(); req.onsuccess = () => resolve(req.result); req.onerror = () => reject(req.error); });
        const [metadata, bytes] = await Promise.all([read('workbooks'), read('workbook_bytes')]);
        db.close();
        return {metadata, bytes: bytes.map(b => Array.from(b))};
    `);
    assert.equal(await run("return document.querySelector('#formula-bar textarea').value"), '42');
    await run(`
        const field = document.querySelector('#formula-bar textarea');
        field.focus(); field.value = '123';
        field.dispatchEvent(new Event('input', {bubbles: true}));
        field.dispatchEvent(new KeyboardEvent('keydown', {key:'Enter', code:'Enter', bubbles:true, cancelable:true}));
    `);
    await run("document.querySelector('#workbook').dispatchEvent(new KeyboardEvent('keydown', {key:'ArrowUp', bubbles:true, cancelable:true}))");
    await until(() => run("return document.querySelector('#formula-bar textarea').value === '123'"), 'edited cell');
    assert.equal(await run("return window.dispatchEvent(new Event('beforeunload', {cancelable:true}))"), false, 'unsaved edits must warn on unload');
    await run("Object.defineProperty(document, 'hidden', {value:true, configurable:true}); document.dispatchEvent(new Event('visibilitychange')); delete document.hidden");
    await until(async () => (await records()).bytes[0][4] === 3, 'hidden-page save writes new format');
    await until(() => run("return window.dispatchEvent(new Event('beforeunload', {cancelable:true}))"), 'unload warning clears after commit');
    await run("document.querySelector('.tb-ham').click()");
    await until(() => run("return !!document.querySelector('.ld-btn-add')"), 'sidebar');
    await run(`
        window.originalPut = IDBObjectStore.prototype.put;
        IDBObjectStore.prototype.put = function(...args) {
            if (this.name === 'workbook_bytes') throw new DOMException('Injected quota failure', 'QuotaExceededError');
            return window.originalPut.apply(this, args);
        };
        document.querySelector('.ld-btn-add').click();
    `);
    await until(() => run("return !document.querySelector('#app').inert && document.body.textContent.includes('Cannot save workbook')"), 'save failure shown');
    assert.equal(await selected(), initial, 'failed outgoing save must prevent switch');
    assert.equal((await records()).metadata.length, 1, 'failed create must not add workbook');
    await run("IDBObjectStore.prototype.put = window.originalPut; document.querySelector('.ld-btn-add').click()");
    await until(async () => (await selected()) !== initial && await run("return !document.querySelector('#app').inert"), 'create after failure');
    const second = await selected();
    await run("[...document.querySelectorAll('.ld-entry')].find(el => el.textContent.includes('Legacy workbook')).click()");
    await until(async () => (await selected()) === initial && await run("return !document.querySelector('#app').inert"), 'switch to edited workbook');
    assert.equal(await run("return document.querySelector('#formula-bar textarea').value"), '123');
    await run(`document.querySelector('.ld-entry.active').dispatchEvent(new MouseEvent('dblclick', {bubbles:true}))`);
    await until(() => run("return !!document.querySelector('.ld-rename')"), 'rename input');
    await run(`
        const input = document.querySelector('.ld-rename');
        input.value = 'Renamed workbook'; input.dispatchEvent(new Event('input', {bubbles:true}));
        input.dispatchEvent(new KeyboardEvent('keydown', {key:'Enter', bubbles:true, cancelable:true}));
    `);
    await until(async () => (await records()).metadata.some(m => m.name === 'Renamed workbook'), 'rename stored');
    await until(() => run("return !document.querySelector('#app').inert"), 'rename complete');
    const renamed = (await records()).metadata.find(m => m.name === 'Renamed workbook');
    assert.equal(renamed.group, 'Old group');
    assert.equal(renamed.shared_from_link, false);
    await new Promise(resolve => setTimeout(resolve, 1200));
    await wd(`/session/${session}/refresh`, {});
    await until(() => run("return !!document.querySelector('.workbook')"), 'reload');
    assert.equal(await selected(), initial);
    assert.equal(await run("return document.querySelector('#formula-bar textarea').value"), '123');
    await run("document.querySelector('.tb-ham').click()");
    await until(() => run("return !!document.querySelector('.ld-entry.active .ld-del')"), 'delete control');
    await run("window.confirm = () => true; document.querySelector('.ld-entry.active .ld-del').click()");
    await until(async () => (await selected()) === second && await run("return !document.querySelector('#app').inert"), 'delete activates replacement');
    assert.equal((await records()).metadata.length, 1);
    await new Promise(resolve => setTimeout(resolve, 1200));
    assert.equal((await records()).bytes.length, 1, 'autosave cannot resurrect deleted record');
    console.log('PASS: mounted edit, autosave, quota failure, create, switch, rename, reload, delete, and no resurrection');
    await run("[...document.querySelectorAll('.tb-tab')].find(el => el.textContent.trim() === 'File').click()");
    await until(() => run("return !!document.querySelector('input[type=file]')"), 'file input');
    const fileInput = await wd(`/session/${session}/element`, {using:'css selector', value:'input[type=file]'});
    const elementId = fileInput['element-6066-11e4-a52e-4f735466cecf'];
    await run("document.querySelector('input[type=file]').style.display = 'block'");
    await wd(`/session/${session}/element/${elementId}/value`, {text:resolve(root, 'IronCalc/xlsx/tests/link_test.xlsx')});
    await until(async () => (await records()).metadata.some(m => m.name === 'link_test') && await run("return !document.querySelector('#app').inert"), 'XLSX import');
    const imported = await selected();
    assert.notEqual(imported, second);
    await new Promise(resolve => setTimeout(resolve, 1200));
    await wd(`/session/${session}/refresh`, {});
    await until(() => run("return !!document.querySelector('.workbook')"), 'import reload');
    assert.equal(await selected(), imported);
    const sharedRecords = await records();
    const sourceIndex = sharedRecords.metadata.findIndex(m => m.name === 'link_test');
    const payload = Buffer.from([0, ...sharedRecords.bytes[sourceIndex].slice(5)]).toString('base64url');
    assert.ok(sharedRecords.bytes[sourceIndex].length < 30000, 'share fixture fits the URL limit');
    await run("window.location.hash = arguments[0]", [`share=${payload}`]);
    await wd(`/session/${session}/refresh`, {});
    await until(() => run("return !!document.querySelector('.sv-submit-btn')"), 'share consent');
    assert.equal((await records()).metadata.length, 2, 'share must wait for consent');
    await run("document.querySelector('.sv-submit-btn').click()");
    await until(async () => (await selected()) !== imported && await run("return !document.querySelector('#app').inert && !document.querySelector('.sv-submit-btn')"), 'share accepted');
    assert.equal((await records()).metadata.filter(m => m.shared_from_link).length, 1);
    assert.equal(await run("return window.location.hash"), '');
    console.log('PASS: XLSX import, imported-workbook reload, and consent-gated share persistence');

} finally {
    if (session) await wd(`/session/${session}`, undefined, "DELETE").catch(() => {});
    driver?.kill();
    server.close();
}
