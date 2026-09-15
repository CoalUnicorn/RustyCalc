import initIronCanvas, {
    IronCanvas,
    RenderResult,
} from "./vendor/iron-canvas/iron_canvas_web.js";
import initIronCalc, { Model } from "./vendor/ironcalc/wasm.js";
import {
    DEMO_WORKBOOKS,
    autofitPlanFor,
    columnLabel,
    detailFor,
    installDenseRangeMethods,
    queryOptions,
    rectCenter,
    shouldRescheduleAfterDrain,
} from "./harness-core.js";

const LIGHT_THEME = Object.freeze({
    gridColor: "#d9d9d9",
    gridSeparatorColor: "#a6a6a6",
    headerBg: "#f3f3f3",
    headerBorderColor: "#c9c9c9",
    headerTextColor: "#444444",
    headerSelectedBg: "#e2f0d9",
    headerSelectedColor: "#107c41",
    defaultTextColor: "#222222",
    errorTextColor: "#b3261e",
    selectionColor: "#107c41",
    cellBg: "#ffffff",
    pointing: "#4472c4",
    selectionFill: "rgba(16, 124, 65, 0.08)",
    pointingTint: "rgba(68, 114, 196, 0.10)",
});

const REQUIRED_MODEL_METHODS = [
    "getSelectedView",
    "getSelectedSheet",
    "getFrozenRowsCount",
    "getFrozenColumnsCount",
    "getRowHeight",
    "getColumnWidth",
    "getShowGridLines",
    "getCellStyle",
    "getCellType",
    "getFormattedCellValue",
];

const elements = Object.fromEntries(
    [
        "status",
        "workbook-select",
        "load-workbook",
        "workbook-source",
        "canvas-stack",
        "grid",
        "overlay",
        "tabs",
        "cursor-readout",
        "frame-trace",
        "run-checks",
        "check-results",
        "autofit-columns",
        "autofit-result",
        "point-range",
        "clipboard",
        "formula-refs",
        "clear-overlays",
        "theme-light",
        "theme-dark",
        "swap-workbook-theme",
        "save-svg",
        "reset-bridge",
        "bridge-counts",
    ].map((id) => [id, document.getElementById(id)]),
);

const options = queryOptions(window.location.search);
let canvas;
let model;
let bridge;
let workbookId = "sample";
let rendererTheme = "light";
let accentSwapped = false;
let paintFrame = 0;
let checksPromise = null;
let lastReport = null;
let lastAutofit = null;

let resolveReady;
let rejectReady;
const ready = new Promise((resolve, reject) => {
    resolveReady = resolve;
    rejectReady = reject;
});

window.ironCanvasHarness = {
    ready,
    async loadWorkbook(id) {
        await ready;
        return loadWorkbook(id);
    },
    async runChecks() {
        await ready;
        return runChecks();
    },
    async autofitColumns() {
        await ready;
        return autofitColumns();
    },
    getReport() {
        return lastReport;
    },
    getState() {
        return {
            workbook: workbookId,
            bridge: bridge?.snapshot() ?? null,
            frameTrace: canvas?.frameTrace() ?? "",
            autofit: lastAutofit,
        };
    },
};

function setStatus(message, kind = "ready") {
    elements.status.className = `status ${kind}`;
    elements.status.textContent = message;
}

function createSampleModel() {
    const next = new Model("Web API sample", "en", "UTC", "en");
    next.renameSheet(0, "Numbers");
    next.setUserInput(0, 1, 1, "1");
    next.setUserInput(0, 2, 1, "2");
    next.setUserInput(0, 3, 1, "3");
    next.setUserInput(0, 1, 2, "=A1+10");
    next.setUserInput(0, 2, 2, "=A2+10");
    next.setUserInput(0, 3, 2, "=A3+10");
    next.updateRangeStyle(
        { sheet: 0, row: 1, column: 3, width: 1, height: 3 },
        "fill.color",
        "[4, 0]",
    );
    next.updateRangeStyle(
        { sheet: 0, row: 1, column: 4, width: 1, height: 3 },
        "fill.color",
        "[4, 0.4]",
    );

    next.newSheet();
    next.renameSheet(1, "Text");
    next.setUserInput(1, 1, 1, "hello");
    next.setUserInput(1, 2, 1, "world");
    next.setUserInput(1, 1, 2, "iron-canvas");

    next.newSheet();
    next.renameSheet(2, "Math");
    next.setUserInput(2, 1, 1, "3.14159");
    next.setUserInput(2, 2, 1, "2.71828");
    next.setUserInput(2, 3, 1, "=A1*A2");
    return next;
}

async function fetchDemoModel(id) {
    const demo = DEMO_WORKBOOKS[id];
    if (!demo) throw new Error(`Unknown demo workbook: ${id}`);
    const response = await fetch(demo.compiled, { cache: "no-store" });
    if (!response.ok) {
        throw new Error(
            `Could not load ${demo.compiled} (${response.status}). Run \"make demos\" or \"make serve\" first.`,
        );
    }
    return Model.from_bytes(new Uint8Array(await response.arrayBuffer()), "en");
}

async function loadWorkbook(id) {
    if (id !== "sample" && !(id in DEMO_WORKBOOKS)) {
        throw new Error(`Unknown workbook: ${id}`);
    }
    setStatus(`Loading ${id === "sample" ? "generated sample" : DEMO_WORKBOOKS[id].label}…`, "loading");
    elements["workbook-select"].value = id;

    const next = id === "sample" ? createSampleModel() : await fetchDemoModel(id);
    bridge = installDenseRangeMethods(next, renderBridgeCounts);
    model = next;
    workbookId = id;
    accentSwapped = false;
    lastAutofit = null;
    elements["autofit-result"].textContent = "Original workbook widths are unchanged";
    canvas.setModel(model);
    canvas.setThemeName(rendererTheme);
    resizeCanvas();
    canvas.requestRepaint();
    drainPaint();
    renderTabs();
    renderBridgeCounts();
    elements["workbook-source"].textContent =
        id === "sample"
            ? "Generated in JavaScript"
            : `${DEMO_WORKBOOKS[id].source} → ${DEMO_WORKBOOKS[id].compiled}`;
    setStatus(`${model.getName()} ready — ${model.getWorksheetsProperties().length} sheet(s)`);
    return { workbook: id, sheets: model.getWorksheetsProperties().length };
}

function resizeCanvas() {
    if (!canvas) return;
    const rect = elements["canvas-stack"].getBoundingClientRect();
    canvas.resize(rect.width, rect.height, window.devicePixelRatio || 1);
    schedulePaint();
}

function drainPaint(maxAttempts = 16) {
    if (!canvas) return [];
    const outcomes = [];
    for (let attempt = 0; attempt < maxAttempts; attempt += 1) {
        const outcome = canvas.renderPending();
        outcomes.push(outcome);
        if (outcome === RenderResult.Idle) break;
    }
    elements["frame-trace"].textContent = canvas.frameTrace() || "No frame trace available";
    return outcomes;
}

function schedulePaint() {
    if (paintFrame || !canvas) return;
    paintFrame = requestAnimationFrame(() => {
        paintFrame = 0;
        const outcomes = drainPaint();
        if (shouldRescheduleAfterDrain(outcomes.at(-1), RenderResult)) schedulePaint();
    });
}

function renderTabs() {
    const sheets = model.getWorksheetsProperties();
    const active = model.getSelectedSheet();
    elements.tabs.replaceChildren();
    sheets.forEach((sheet, index) => {
        const button = document.createElement("button");
        button.type = "button";
        button.textContent = sheet.name;
        button.classList.toggle("active", index === active);
        button.setAttribute("aria-current", index === active ? "page" : "false");
        button.addEventListener("click", () => {
            model.setSelectedSheet(index);
            canvas.viewChanged();
            renderTabs();
            schedulePaint();
        });
        elements.tabs.append(button);
    });
}

function renderBridgeCounts() {
    if (!bridge) return;
    const fragment = document.createDocumentFragment();
    for (const [name, value] of Object.entries(bridge.counts)) {
        const row = document.createElement("div");
        const term = document.createElement("dt");
        const detail = document.createElement("dd");
        term.textContent = name;
        detail.textContent = String(value);
        row.append(term, detail);
        fragment.append(row);
    }
    elements["bridge-counts"].replaceChildren(fragment);
}

function setOverlays(overlays) {
    canvas.setOverlays(overlays);
    canvas.requestOverlayRepaint();
    schedulePaint();
}

function clearOverlays() {
    setOverlays({});
}

function autofitColumns() {
    const sheet = model.getSelectedSheet();
    const plan = autofitPlanFor(workbookId, sheet);
    if (!plan) throw new Error(`No autofit fixture range for sheet ${sheet + 1}`);

    const fitted = [];
    for (let column = plan.firstColumn; column <= plan.lastColumn; column += 1) {
        const before = model.getColumnWidth(sheet, column);
        const width = canvas.fitColumnWidth(column, plan.firstRow, plan.lastRow);
        if (!Number.isFinite(width)) continue;
        model.setColumnsWidth(sheet, column, column, width);
        fitted.push({ column, before, width });
    }
    if (fitted.length === 0) throw new Error("No populated columns were measured");

    canvas.requestRepaint();
    drainPaint();
    lastAutofit = { sheet, plan, columns: fitted };
    elements["autofit-result"].textContent = fitted
        .map(({ column, before, width }) =>
            `${columnLabel(column)} ${before.toFixed(1)}→${width.toFixed(1)} px`,
        )
        .join(" · ");
    return lastAutofit;
}

function downloadSvg() {
    const size = canvas.canvasSize();
    const svg = canvas.exportSvg(size.w, size.h);
    const url = URL.createObjectURL(new Blob([svg], { type: "image/svg+xml" }));
    const anchor = Object.assign(document.createElement("a"), {
        href: url,
        download: `${model.getName() || "sheet"}.svg`,
    });
    document.body.append(anchor);
    anchor.click();
    anchor.remove();
    URL.revokeObjectURL(url);
}

function renderCheckResults(results) {
    const fragment = document.createDocumentFragment();
    for (const result of results) {
        const item = document.createElement("li");
        const body = document.createElement("span");
        item.className = `check-result ${result.pass ? "pass" : "fail"}`;
        body.textContent = result.name;
        if (result.detail) {
            const detail = document.createElement("small");
            detail.textContent = result.detail;
            body.append(detail);
        }
        item.append(body);
        fragment.append(item);
    }
    elements["check-results"].replaceChildren(fragment);
}

async function runChecks() {
    if (checksPromise) return checksPromise;
    checksPromise = runChecksOnce().finally(() => {
        checksPromise = null;
    });
    return checksPromise;
}

async function runChecksOnce() {
    const results = [];
    const check = async (name, callback) => {
        try {
            const detail = await callback();
            results.push({ name, pass: true, detail: detail == null ? "" : String(detail) });
        } catch (error) {
            results.push({ name, pass: false, detail: detailFor(error) });
        }
        renderCheckResults(results);
    };

    elements["run-checks"].disabled = true;
    elements["check-results"].replaceChildren();
    setStatus("Running browser API checks…", "loading");
    resizeCanvas();
    drainPaint();

    await check("IronCalc model satisfies the canvas contract", () => {
        const missing = REQUIRED_MODEL_METHODS.filter((name) => typeof model[name] !== "function");
        if (missing.length) throw new Error(`Missing: ${missing.join(", ")}`);
        return `${REQUIRED_MODEL_METHODS.length} required methods`;
    });

    await check("canvasSize matches the visible canvas", () => {
        const actual = canvas.canvasSize();
        const rect = elements["canvas-stack"].getBoundingClientRect();
        if (Math.abs(actual.w - rect.width) > 1 || Math.abs(actual.h - rect.height) > 1) {
            throw new Error(`Expected ${rect.width}×${rect.height}, got ${actual.w}×${actual.h}`);
        }
        return `${actual.w}×${actual.h} CSS px`;
    });

    let a1Rect;
    await check("cellRect returns the painted A1 geometry", () => {
        a1Rect = canvas.cellRect(1, 1);
        const center = rectCenter(a1Rect);
        if (a1Rect.width <= 0 || a1Rect.height <= 0) throw new Error("A1 has no area");
        return `center ${center.x},${center.y}`;
    });

    await check("pixelToCell resolves through grid geometry", () => {
        const center = rectCenter(a1Rect);
        const cell = canvas.pixelToCell(center.x, center.y);
        if (cell?.row !== 1 || cell?.column !== 1) {
            throw new Error(`Expected A1, got ${JSON.stringify(cell)}`);
        }
        return JSON.stringify(cell);
    });

    await check("hitTest returns a tagged cell result", () => {
        const center = rectCenter(a1Rect);
        const hit = canvas.hitTest(center.x, center.y);
        if (hit?.kind !== "cell" || hit.row !== 1 || hit.column !== 1) {
            throw new Error(`Expected A1 cell hit, got ${JSON.stringify(hit)}`);
        }
        return JSON.stringify(hit);
    });

    await check("off-screen cellRect returns null", () => {
        const result = canvas.cellRect(1_048_576, 16_384);
        if (result !== null) throw new Error(`Expected null, got ${JSON.stringify(result)}`);
    });

    await check("resizeHandleAt identifies the first column edge", () => {
        const x = a1Rect.top_left.x + a1Rect.width;
        const y = Math.max(1, a1Rect.top_left.y / 2);
        const target = canvas.resizeHandleAt(x, y, 4);
        if (target?.kind !== "column" || target.column !== 1) {
            throw new Error(`Expected column 1, got ${JSON.stringify(target)}`);
        }
        return JSON.stringify(target);
    });

    await check("fitColumnWidth measures and applies workbook columns", () => {
        const result = autofitColumns();
        for (const entry of result.columns) {
            const applied = model.getColumnWidth(result.sheet, entry.column);
            if (Math.abs(applied - entry.width) > 0.01) {
                throw new Error(
                    `${columnLabel(entry.column)} expected ${entry.width}, model has ${applied}`,
                );
            }
        }
        if (workbookId === "forensics") {
            const narrow = result.columns.filter(({ before }) => before < 24).length;
            const widened = result.columns.filter(({ before, width }) => width > before + 1).length;
            if (narrow < 4 || widened < 3) {
                throw new Error(`Expected narrow imported columns to widen: ${JSON.stringify(result)}`);
            }
            return `${narrow} narrow imports, ${widened} widened by core autofit`;
        }
        return `${result.columns.length} populated columns measured and applied`;
    });

    await check("overlay setters accept their camelCase wire shapes", () => {
        const sheet = model.getSelectedSheet();
        canvas.setPointRange({ r1: 1, c1: 1, r2: 5, c2: 3 });
        canvas.setClipboard({ sheet, range: { r1: 2, c1: 2, r2: 4, c2: 4 } });
        canvas.setFormulaRefs([
            {
                sheetArea: { sheet, range: { r1: 1, c1: 1, r2: 3, c2: 2 } },
                colorIdx: 0,
                kind: { kind: "direct" },
            },
        ]);
        canvas.setOverlays({ pointRange: { r1: 1, c1: 1, r2: 2, c2: 2 } });
        canvas.requestOverlayRepaint();
        drainPaint();
        clearOverlays();
        drainPaint();
        return "setPointRange, setClipboard, setFormulaRefs, setOverlays";
    });

    await check("malformed overlay input throws a JavaScript Error", () => {
        let threw = false;
        try {
            canvas.setPointRange({ r1: "bad", c1: 1, r2: 2, c2: 2 });
        } catch (error) {
            threw = error instanceof Error;
        }
        if (!threw) throw new Error("Malformed range was accepted");
        canvas.setPointRange(null);
    });

    await check("theme setters repaint with full and partial payloads", () => {
        canvas.setThemeVariables({ selectionColor: "#7b2cbf" });
        canvas.setTheme(LIGHT_THEME);
        canvas.setThemeName(rendererTheme);
        drainPaint();
        return "setThemeVariables, setTheme, setThemeName";
    });

    await check("SVG export returns a standalone document", () => {
        const size = canvas.canvasSize();
        const svg = canvas.exportSvg(size.w, size.h);
        if (!svg.startsWith("<svg") || !svg.includes("</svg>")) {
            throw new Error("exportSvg did not return an SVG document");
        }
        return `${svg.length.toLocaleString()} characters`;
    });

    await check("pane reads use bulk fetches with bounded scalar probes", () => {
        bridge.reset();
        canvas.markContentDirty();
        drainPaint();
        const counts = bridge.snapshot();
        const bulk = counts.getCellStylesIn + counts.getFormattedCellValuesIn + counts.getCellTypesIn;
        const scalar = counts.getCellStyle + counts.getFormattedCellValue + counts.getCellType;
        if (bulk === 0) throw new Error(`No bulk calls observed: ${JSON.stringify(counts)}`);
        // Frame capture reads the active cell through scalar accessors; the
        // dense pane itself must stay on the three range methods.
        if (scalar > 4) throw new Error(`Observed ${scalar} scalar crossings`);
        return `${bulk} bulk calls, ${scalar} per-cell calls`;
    });

    await check("frameTrace and recordingSupported are always callable", () => {
        const trace = canvas.frameTrace();
        const recording = IronCanvas.recordingSupported();
        if (typeof trace !== "string" || typeof recording !== "boolean") {
            throw new Error("Unexpected diagnostics types");
        }
        return `recording ${recording ? "enabled" : "disabled"}`;
    });

    const passed = results.filter((result) => result.pass).length;
    lastReport = {
        workbook: workbookId,
        passed,
        failed: results.length - passed,
        checks: results,
    };
    elements["run-checks"].disabled = false;
    setStatus(
        lastReport.failed === 0
            ? `${passed}/${results.length} browser API checks passed`
            : `${lastReport.failed}/${results.length} browser API checks failed`,
        lastReport.failed === 0 ? "ready" : "error",
    );
    window.dispatchEvent(new CustomEvent("iron-canvas-checks", { detail: lastReport }));
    return lastReport;
}

function wireInteractions() {
    elements["load-workbook"].addEventListener("click", () => {
        loadWorkbook(elements["workbook-select"].value).catch(showFatalError);
    });
    elements["run-checks"].addEventListener("click", () => {
        runChecks().catch(showFatalError);
    });
    elements["autofit-columns"].addEventListener("click", () => {
        try {
            autofitColumns();
            setStatus("Column widths measured by core autofit and applied");
        } catch (error) {
            showFatalError(error);
        }
    });
    elements["point-range"].addEventListener("click", () => {
        setOverlays({ pointRange: { r1: 1, c1: 1, r2: 5, c2: 3 } });
    });
    elements.clipboard.addEventListener("click", () => {
        setOverlays({
            clipboard: {
                sheet: model.getSelectedSheet(),
                range: { r1: 2, c1: 2, r2: 4, c2: 4 },
            },
        });
    });
    elements["formula-refs"].addEventListener("click", () => {
        setOverlays({
            formulaRefs: [
                {
                    sheetArea: {
                        sheet: model.getSelectedSheet(),
                        range: { r1: 1, c1: 1, r2: 4, c2: 2 },
                    },
                    colorIdx: 0,
                    kind: { kind: "direct" },
                },
            ],
        });
    });
    elements["clear-overlays"].addEventListener("click", clearOverlays);
    elements["theme-light"].addEventListener("click", () => {
        rendererTheme = "light";
        canvas.setThemeName(rendererTheme);
        schedulePaint();
    });
    elements["theme-dark"].addEventListener("click", () => {
        rendererTheme = "dark";
        canvas.setThemeName(rendererTheme);
        schedulePaint();
    });
    elements["swap-workbook-theme"].addEventListener("click", () => {
        const theme = model.getTheme();
        theme.accent1 = accentSwapped ? "#4472c4" : "#e91e63";
        accentSwapped = !accentSwapped;
        model.setTheme(theme);
        canvas.themeChanged();
        schedulePaint();
    });
    elements["save-svg"].addEventListener("click", downloadSvg);
    elements["reset-bridge"].addEventListener("click", () => bridge.reset());

    let pointerFrame = 0;
    elements.grid.addEventListener("pointermove", (event) => {
        if (pointerFrame) return;
        pointerFrame = requestAnimationFrame(() => {
            pointerFrame = 0;
            const rect = elements.grid.getBoundingClientRect();
            const hit = canvas.hitTest(event.clientX - rect.left, event.clientY - rect.top);
            elements["cursor-readout"].textContent = JSON.stringify(hit);
        });
    });
    elements.grid.addEventListener("click", (event) => {
        const rect = elements.grid.getBoundingClientRect();
        const hit = canvas.hitTest(event.clientX - rect.left, event.clientY - rect.top);
        if (hit.kind !== "cell") return;
        model.setSelectedCell(hit.row, hit.column);
        canvas.viewChanged();
        schedulePaint();
    });
    elements["canvas-stack"].addEventListener(
        "wheel",
        (event) => {
            event.preventDefault();
            const view = model.getSelectedView();
            const rowDelta = event.shiftKey ? 0 : Math.sign(event.deltaY) * 3;
            const columnDelta = event.shiftKey ? Math.sign(event.deltaY) : Math.sign(event.deltaX);
            model.setTopLeftVisibleCell(
                Math.max(1, view.top_row + rowDelta),
                Math.max(1, view.left_column + columnDelta),
            );
            canvas.viewChanged();
            schedulePaint();
        },
        { passive: false },
    );
}

function showFatalError(error) {
    const detail = detailFor(error);
    console.error(error);
    setStatus(detail, "error");
}

async function start() {
    let phase = "initialize WebAssembly";
    try {
        await Promise.all([initIronCanvas(), initIronCalc()]);
        phase = "create IronCanvas";
        canvas = IronCanvas.create(elements.grid, elements.overlay);
        wireInteractions();
        new ResizeObserver(resizeCanvas).observe(elements["canvas-stack"]);
        document.fonts?.addEventListener("loadingdone", () => {
            canvas.fontsChanged();
            schedulePaint();
        });

        phase = "load initial workbook";
        await loadWorkbook(options.workbook);
        resolveReady(window.ironCanvasHarness);
        if (options.autorun) await runChecks();
    } catch (error) {
        const wrapped = new Error(`[${phase}] ${detailFor(error)}`);
        rejectReady(wrapped);
        showFatalError(wrapped);
    }
}

start();
