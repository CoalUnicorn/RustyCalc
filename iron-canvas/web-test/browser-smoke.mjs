import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL(".", import.meta.url));
const webPort = Number(process.env.PORT || 8123);
const driverPort = Number(process.env.WEBDRIVER_PORT || 9515);
const workbook = process.env.WORKBOOK || "dynamic_arrays";
// Opt-in device-pixel-ratio emulation for the raster-parity check. Real
// browser emulation (not a JS override of `window.devicePixelRatio`), applied
// before the page loads so the canvas is created at that DPR.
const dpr = Number(process.env.DPR || 0);
const webOrigin = `http://127.0.0.1:${webPort}`;
const driverOrigin = `http://127.0.0.1:${driverPort}`;
const children = [];
let sessionId;

function launch(command, args) {
    const child = spawn(command, args, {
        cwd: root,
        stdio: ["ignore", "pipe", "pipe"],
    });
    children.push(child);
    return child;
}

async function waitFor(url, label, child) {
    const deadline = Date.now() + 15_000;
    while (Date.now() < deadline) {
        if (child.exitCode !== null) {
            throw new Error(`${label} exited with code ${child.exitCode}`);
        }
        try {
            const response = await fetch(url);
            if (response.ok) return;
        } catch {
            // The process may still be binding its socket.
        }
        await new Promise((resolve) => setTimeout(resolve, 100));
    }
    throw new Error(`${label} did not become ready at ${url}`);
}

async function webdriver(path, method = "GET", body) {
    const response = await fetch(`${driverOrigin}${path}`, {
        method,
        headers: body ? { "content-type": "application/json" } : undefined,
        body: body ? JSON.stringify(body) : undefined,
    });
    const payload = await response.json();
    if (!response.ok || payload.value?.error) {
        throw new Error(
            `WebDriver ${method} ${path} failed: ${JSON.stringify(payload.value || payload)}`,
        );
    }
    return payload.value;
}

async function main() {
    const server = launch("python3", [
        "-m",
        "http.server",
        String(webPort),
        "--bind",
        "127.0.0.1",
    ]);
    const driver = launch("chromedriver", [
        `--port=${driverPort}`,
        "--allowed-ips=127.0.0.1",
    ]);
    await Promise.all([
        waitFor(`${webOrigin}/index.html`, "HTTP server", server),
        waitFor(`${driverOrigin}/status`, "ChromeDriver", driver),
    ]);

    const session = await webdriver("/session", "POST", {
        capabilities: {
            alwaysMatch: {
                browserName: "chrome",
                "goog:chromeOptions": {
                    args: [
                        "--headless=new",
                        "--no-sandbox",
                        "--disable-dev-shm-usage",
                        "--window-size=1440,1000",
                    ],
                },
            },
        },
    });
    sessionId = session.sessionId;
    await webdriver(`/session/${sessionId}/timeouts`, "POST", { script: 120_000 });
    if (dpr > 0) {
        // Chrome-specific WebDriver extension. The override must precede the
        // page load: the harness reads `window.devicePixelRatio` when it sizes
        // the canvas, and the raster-parity check reports the DPR it saw.
        await webdriver(`/session/${sessionId}/chromium/send_command`, "POST", {
            cmd: "Emulation.setDeviceMetricsOverride",
            params: { width: 1440, height: 1000, deviceScaleFactor: dpr, mobile: false },
        });
    }
    await webdriver(`/session/${sessionId}/url`, "POST", {
        url: `${webOrigin}/index.html?workbook=${encodeURIComponent(workbook)}`,
    });

    const result = await webdriver(`/session/${sessionId}/execute/async`, "POST", {
        script: `
            const done = arguments[arguments.length - 1];
            const deadline = Date.now() + 15000;
            const wait = () => {
                const harness = window.ironCanvasHarness;
                if (harness) {
                    harness.ready
                        .then(() => harness.runChecks())
                        .then((report) => done({ report }))
                        .catch((error) => done({ error: error.stack || String(error) }));
                    return;
                }
                if (Date.now() >= deadline) {
                    done({ error: "window.ironCanvasHarness was not installed" });
                    return;
                }
                setTimeout(wait, 50);
            };
            wait();
        `,
        args: [],
    });

    if (result.error) throw new Error(result.error);
    const report = result.report;
    for (const check of report.checks) {
        process.stdout.write(`${check.pass ? "PASS" : "FAIL"} ${check.name}\n`);
        // A passing check prints its detail only when the detail carries a
        // KNOWN-DEFECT line, so an accepted pre-existing defect stays visible
        // in the harness output instead of being hidden by the pass.
        if (check.detail && (!check.pass || check.detail.includes("KNOWN-DEFECT"))) {
            process.stdout.write(`  ${check.detail}\n`);
        }
    }
    process.stdout.write(
        `browser smoke: ${report.passed} passed, ${report.failed} failed (${report.workbook})\n`,
    );
    if (report.failed) process.exitCode = 1;
}

try {
    await main();
} finally {
    if (sessionId) {
        await webdriver(`/session/${sessionId}`, "DELETE").catch(() => {});
    }
    for (const child of children) {
        if (child.exitCode === null) child.kill("SIGTERM");
    }
}
