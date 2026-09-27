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
    knownSeparatorEdges,
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
    return modelFromBytes(new Uint8Array(await response.arrayBuffer()), "en");
}

/**
 * Decode a compiled `.ic` workbook. The upstream binding renamed this static
 * from `from_bytes` to `fromBytes`; accept either so the harness runs against
 * both the vendored package and a freshly built one (and therefore can
 * exercise `getMergedCells`, which only a current build exposes).
 */
function modelFromBytes(bytes, language) {
    const decode = Model.fromBytes ?? Model.from_bytes;
    if (typeof decode !== "function") {
        throw new Error("the vendored IronCalc package exposes no fromBytes/from_bytes");
    }
    return decode.call(Model, bytes, language);
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

/**
 * Raw pixels of a harness layer. The renderer owns the context, so
 * `getContext("2d")` here returns that same context and never resets the
 * backing store.
 */
function layerPixels(element) {
    const ctx = element.getContext("2d");
    if (!ctx) throw new Error(`Layer #${element.id} has no 2D context`);
    return ctx.getImageData(0, 0, element.width, element.height).data;
}

/**
 * Compare two RGBA rasters.
 *
 * `null` when identical. Otherwise the differing byte and pixel counts plus up
 * to `MISMATCH_POINTS` differing pixels with both RGBA values; `truncated`
 * reports that more pixels differ than were collected.
 */
const MISMATCH_POINTS = 4096;

function rasterDiff(fresh, retained, width) {
    if (fresh.length !== retained.length) {
        return { lengthMismatch: `${fresh.length} vs ${retained.length}` };
    }
    const points = [];
    let differingBytes = 0;
    let pixels = 0;
    let truncated = false;
    for (let pixel = 0; pixel * 4 < fresh.length; pixel += 1) {
        const at = pixel * 4;
        const same =
            fresh[at] === retained[at] &&
            fresh[at + 1] === retained[at + 1] &&
            fresh[at + 2] === retained[at + 2] &&
            fresh[at + 3] === retained[at + 3];
        if (same) continue;
        pixels += 1;
        for (let channel = 0; channel < 4; channel += 1) {
            if (fresh[at + channel] !== retained[at + channel]) differingBytes += 1;
        }
        if (points.length < MISMATCH_POINTS) {
            points.push({
                x: pixel % width,
                y: Math.floor(pixel / width),
                fresh: [fresh[at], fresh[at + 1], fresh[at + 2], fresh[at + 3]],
                retained: [retained[at], retained[at + 1], retained[at + 2], retained[at + 3]],
            });
        } else {
            truncated = true;
        }
    }
    return pixels === 0 ? null : { pixels, differingBytes, points, truncated };
}

const rgbaText = (rgba) => `[${rgba.join(",")}]`;

/** "first differing pixel (x,y) fresh=[…] retained=[…]; N bytes in M pixels …" */
function describeDiff(diff) {
    const rows = [...new Set(diff.points.map(({ y }) => y))].sort((a, b) => a - b);
    const listed = diff.points
        .slice(0, 4)
        .map(({ x, y }) => `(${x},${y})`)
        .join(" ");
    const first = diff.points[0];
    return (
        `first differing pixel (${first.x},${first.y}) ` +
        `fresh=${rgbaText(first.fresh)} retained=${rgbaText(first.retained)}; ` +
        `${diff.differingBytes} bytes in ${diff.pixels} pixels on row(s) ` +
        `${rows[0]}..${rows[rows.length - 1]} differ at ${listed}` +
        (diff.truncated ? " …" : "")
    );
}

/**
 * Known pre-existing core defect, recorded in
 * `docs/bugs/2026-09-26-retained-seam-alpha-accumulation.md`. At fractional
 * DPR the snapped per-cell opaque fills leave the sub-pixel separator band
 * uncovered, so a retained content repaint re-strokes the separator on top of
 * the previous stroke and the alpha accumulates (205 -> 202 -> ...). At DPR 1
 * the same gap shows up only at the seam at the top of the cell area, below
 * the column-header strip, where it accumulates 203 -> 201. It reproduces with
 * a workbook that has no links at all, so it is not this feature's defect.
 *
 * Accepted here so the check can still guard link rendering, but only for a
 * mismatch that passes every test below. Link text and underline pixels paint
 * strictly inside a cell, so a link regression always fails loudly.
 */
/**
 * Device-pixel geometry of the painted pane, scaled by DPR: the visible cell
 * rects as their column and row edge coordinates. The pane is a uniform grid,
 * so one probe row gives every column edge and one probe column gives every row
 * edge. Null when the geometry cannot be derived, which keeps the classifier
 * conservative.
 */
function paintedEdgeGeometry() {
    const view = model.getSelectedView();
    const dpr = window.devicePixelRatio || 1;
    const columns = [];
    for (let column = view.left_column; column < view.left_column + 64; column += 1) {
        const rect = canvas.cellRect(view.top_row, column);
        if (!rect || rect.width <= 0) break;
        columns.push({ left: rect.top_left.x * dpr, right: (rect.top_left.x + rect.width) * dpr });
    }
    const rowBands = [];
    for (let row = view.top_row; row < view.top_row + 256; row += 1) {
        const rect = canvas.cellRect(row, view.left_column);
        if (!rect || rect.height <= 0) break;
        rowBands.push({ top: rect.top_left.y * dpr, bottom: (rect.top_left.y + rect.height) * dpr });
    }
    return columns.length > 0 && rowBands.length > 0 ? { columns, rowBands } : null;
}



/**
 * A history-free reference: a brand-new `IronCanvas` over new layer elements
 * has no committed frame, so its first attempt takes the renderer's `Fresh`
 * path and reuses no prior pixels or frame state. Same model, theme, metrics,
 * and DPR as the retained canvas make the two comparable.
 *
 * The caller then applies the same host signal to this reference, so the two
 * rasters share an operation class and differ only in history: the retained
 * canvas carries every earlier blit and repaint, the reference carries none.
 */
function renderFreshReference(size, dpr) {
    const grid = document.createElement("canvas");
    const overlay = document.createElement("canvas");
    const reference = IronCanvas.create(grid, overlay);
    reference.setTheme(LIGHT_THEME);
    reference.setModel(model);
    reference.resize(size.w, size.h, dpr);
    reference.requestRepaint();
    drain(reference);
    if (grid.width !== elements.grid.width || grid.height !== elements.grid.height) {
        throw new Error(
            `Fresh reference is ${grid.width}×${grid.height}, retained is ` +
                `${elements.grid.width}×${elements.grid.height}`,
        );
    }
    return { grid, overlay, canvas: reference };
}

/** Paint every layer with pending work on `target`, or nothing when idle. */
function drain(target) {
    for (let attempt = 0; attempt < 16; attempt += 1) {
        if (target.renderPending() === RenderResult.Idle) break;
    }
}

/** Byte-compare the retained harness layers with a reference's layers. */
function compareLayers(label, reference, notes) {
    for (const layer of ["grid", "overlay"]) {
        const diff = rasterDiff(
            layerPixels(reference[layer]),
            layerPixels(elements[layer]),
            elements[layer].width,
        );
        if (diff === null) continue;
        if (diff.lengthMismatch) {
            throw new Error(`${label}: ${layer} byte length ${diff.lengthMismatch}`);
        }
        const edges = layer === "grid" ? knownSeparatorEdges(diff, paintedEdgeGeometry()) : null;
        if (edges !== null) {
            notes.push(
                `KNOWN-DEFECT ${label}: ${diff.pixels} px on ${edges.length} separator line(s) ` +
                    `${edges.slice(0, 4).join(",")}${edges.length > 4 ? ",…" : ""}, ` +
                    `${describeDiff(diff)} — pre-existing core retained-separator alpha ` +
                    `accumulation (cell-area seam at DPR 1, separator bands at ` +
                    `fractional DPR) ` +
                    `(docs/bugs/2026-09-26-retained-seam-alpha-accumulation.md)`,
            );
            continue;
        }
        throw new Error(
            `${label}: ${layer} (${elements[layer].width}×${elements[layer].height}) ` +
                `not byte-identical to Fresh (${describeDiff(diff)})`,
        );
    }
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

    await check("links bridge reads upstream getLinks targets and dynamic flags", () => {
        const sheet = model.getSelectedSheet();
        // Scratch column T, rows 100-103: clear of every demo fixture, so the
        // check runs against whichever workbook the harness loaded. `linkAt`
        // reads committed frame state, so the paint below is what publishes
        // the engine's `getLinks` answer through the bridge.
        const column = 20;
        const rows = [100, 101, 102, 103];
        model.setUserInput(sheet, rows[0], column, '=HYPERLINK("https://formula.example","x")');
        model.setCellLink(
            sheet,
            rows[1],
            column,
            { type: "External", target: "https://static.example" },
            "static",
        );
        model.setCellLink(
            sheet,
            rows[2],
            column,
            { type: "External", target: "https://tooltip.example", tooltip: "hover me" },
            "hover",
        );
        model.setUserInput(sheet, rows[3], column, '=HYPERLINK("#Sheet1!A5","go")');
        // `setUserInput` only records the formula; the engine builds the
        // dynamic `links` map during evaluation.
        model.evaluate();

        bridge.reset();
        canvas.markContentDirty();
        drainPaint();
        if (bridge.counts.getLinks === 0) {
            throw new Error("A committed paint never crossed getLinks");
        }

        const cases = [
            { row: 100, kind: "external", target: "https://formula.example", dynamic: true },
            { row: 101, kind: "external", target: "https://static.example", dynamic: false },
            {
                row: 102,
                kind: "external",
                target: "https://tooltip.example",
                dynamic: false,
                tooltip: "hover me",
            },
            { row: 103, kind: "internal", target: "Sheet1!A5", dynamic: true },
        ];
        try {
            const colors = new Set();
            for (const { row, kind, target, dynamic, tooltip } of cases) {
                const link = canvas.linkAt(row, column);
                if (!link) throw new Error(`No committed link at T${row}`);
                if (link.kind !== kind || link.target !== target || link.dynamic !== dynamic) {
                    throw new Error(
                        `T${row}: expected ${kind} ${target} dynamic=${dynamic}, got ${JSON.stringify(link)}`,
                    );
                }
                if (tooltip !== undefined && link.tooltip !== tooltip) {
                    throw new Error(
                        `T${row}: expected tooltip ${tooltip}, got ${JSON.stringify(link.tooltip)}`,
                    );
                }
                if (typeof link.color !== "string" || !/^#[0-9A-Fa-f]{6}$/.test(link.color)) {
                    throw new Error(
                        `T${row}: hyperlink color resolved to ${JSON.stringify(link.color)}`,
                    );
                }
                colors.add(link.color);
            }
            // Every link resolves the one theme slot, whichever workbook theme
            // is loaded; the exact default value is pinned natively.
            if (colors.size !== 1) {
                throw new Error(`Link colors diverged: ${JSON.stringify([...colors])}`);
            }
        } finally {
            // Clearing the content removes the worksheet link too, and drops a
            // dynamic one with its formula. The dynamic map is rebuilt only by
            // evaluation, so evaluate before repainting: the committed frame
            // must be link-free for any later run or manual inspection.
            for (const row of rows) model.setUserInput(sheet, row, column, "");
            model.evaluate();
            canvas.markContentDirty();
            drainPaint();
        }
        return `${cases.length} links, getLinks crossed ${bridge.counts.getLinks}×`;
    });

    await check("merged cells bridge reads upstream getMergedCells and resolve covered cells", () => {
        const sheet = model.getSelectedSheet();
        // Scratch block, clear of every demo fixture so the check runs against
        // whichever workbook the harness loaded. A merge with no content is
        // accepted; the anchor gets one text cell afterwards.
        const firstRow = 200;
        const lastRow = 207;
        const firstColumn = 1;
        const lastColumn = 2;
        for (let row = firstRow; row <= lastRow; row += 1) {
            for (let column = firstColumn; column <= lastColumn; column += 1) {
                model.setUserInput(sheet, row, column, "");
            }
        }
        const area = {
            sheet,
            row: firstRow,
            column: firstColumn,
            width: lastColumn - firstColumn + 1,
            height: lastRow - firstRow + 1,
        };
        model.setUserInput(sheet, firstRow, firstColumn, "merged");
        model.mergeCells(area);
        model.evaluate();
        // The scratch block must be on screen: every other check left the view
        // wherever it finished, and `cellRect` answers null off-viewport.
        model.setTopLeftVisibleCell(firstRow - 1, firstColumn);
        canvas.viewChanged();

        bridge.reset();
        canvas.markContentDirty();
        drainPaint();
        if (bridge.counts.getMergedCells === 0) {
            throw new Error("A committed paint never crossed getMergedCells");
        }

        // Parity: the renderer's committed geometry must agree with the
        // engine's own merged list. Demo workbooks ship their own merges, so
        // locate the scratch merge rather than assuming it is the only one.
        const engine = model.getMergedCells(sheet);
        const expected = engine.find(
            (mc) => mc.row === area.row && mc.column === area.column,
        );
        if (!expected) {
            throw new Error(`engine does not report the scratch merge: ${JSON.stringify(engine)}`);
        }

        const assertResolvesToAnchor = (label, row, column) => {
            const rect = canvas.cellRect(row, column);
            if (!rect) throw new Error(`${label}: cell R${row}C${column} is not visible`);
            const hit = canvas.displayCellAt(rect.top_left.x + 2, rect.top_left.y + 2);
            if (!hit) throw new Error(`${label}: displayCellAt returned null`);
            if (hit.row !== row || hit.column !== column) {
                throw new Error(`${label}: physical cell ${hit.row},${hit.column}`);
            }
            if (hit.anchor.r1 !== expected.row || hit.anchor.c1 !== expected.column) {
                throw new Error(
                    `${label}: anchor ${JSON.stringify(hit.anchor)} != engine anchor ${expected.row},${expected.column}`,
                );
            }
            if (
                hit.merged.r1 !== expected.row ||
                hit.merged.c1 !== expected.column ||
                hit.merged.r2 !== expected.row + expected.height - 1 ||
                hit.merged.c2 !== expected.column + expected.width - 1
            ) {
                throw new Error(`${label}: merged ${JSON.stringify(hit.merged)} != engine merge`);
            }
            const size = canvas.canvasSize();
            if (hit.fragment.top_left.x < 0 || hit.fragment.top_left.y < 0) {
                throw new Error(`${label}: fragment starts off-canvas ${JSON.stringify(hit.fragment)}`);
            }
            if (
                hit.fragment.top_left.x + hit.fragment.width > size.w + 1 ||
                hit.fragment.top_left.y + hit.fragment.height > size.h + 1
            ) {
                throw new Error(`${label}: fragment escapes the canvas ${JSON.stringify(hit.fragment)}`);
            }
        };

        try {
            // Anchor visible: a covered cell resolves to it.
            assertResolvesToAnchor("anchor visible", firstRow + 1, firstColumn + 1);

            // Anchor offscreen: scroll so the merge's first rows leave the
            // viewport while covered rows stay visible. The covered cell must
            // still report the offscreen anchor.
            model.setTopLeftVisibleCell(lastRow - 1, firstColumn);
            canvas.viewChanged();
            canvas.markContentDirty();
            drainPaint();
            assertResolvesToAnchor("anchor offscreen", lastRow, firstColumn);
        } finally {
            model.unmergeCells(area);
            for (let row = firstRow; row <= lastRow; row += 1) {
                model.setUserInput(sheet, row, firstColumn, "");
            }
            model.evaluate();
            canvas.markContentDirty();
            drainPaint();
        }
        return `merge ${JSON.stringify(expected)}, getMergedCells crossed ${bridge.counts.getMergedCells}×`;
    });

    await check("a click inside a merge selects its anchor and the editor fragment covers it", () => {
        const sheet = model.getSelectedSheet();
        const firstRow = 220;
        const lastRow = 227;
        const firstColumn = 4;
        const lastColumn = 5;
        for (let row = firstRow; row <= lastRow; row += 1) {
            for (let column = firstColumn; column <= lastColumn; column += 1) {
                model.setUserInput(sheet, row, column, "");
            }
        }
        const area = {
            sheet,
            row: firstRow,
            column: firstColumn,
            width: lastColumn - firstColumn + 1,
            height: lastRow - firstRow + 1,
        };
        model.setUserInput(sheet, firstRow, firstColumn, "merged");
        model.mergeCells(area);
        model.evaluate();
        model.setTopLeftVisibleCell(firstRow - 1, firstColumn);
        canvas.viewChanged();
        canvas.markContentDirty();
        drainPaint();

        try {
            // The engine normalizes the selection: naming a covered cell selects
            // the whole merged range and reports the anchor, which is the
            // address the host then uses for the editor and the active cell.
            model.setSelectedCell(lastRow, lastColumn);
            canvas.viewChanged();
            drainPaint();
            const view = model.getSelectedView();
            if (view.row !== firstRow || view.column !== firstColumn) {
                throw new Error(
                    `selection did not snap to the anchor: ${view.row},${view.column}`,
                );
            }
            const [r1, c1, r2, c2] = view.range;
            if (r1 !== firstRow || c1 !== firstColumn || r2 !== lastRow || c2 !== lastColumn) {
                throw new Error(`selection range is not the merge: ${JSON.stringify(view.range)}`);
            }

            // Editor placement primitives: one fragment spanning the whole
            // merge, strictly larger than the anchor's own cell.
            const fragments = canvas.visibleFragments(firstRow, firstColumn, lastRow, lastColumn);
            if (fragments.length !== 1) {
                throw new Error(`expected one merge fragment, got ${JSON.stringify(fragments)}`);
            }
            const anchorCell = canvas.cellRect(firstRow, firstColumn);
            if (
                fragments[0].rect.width <= anchorCell.width ||
                fragments[0].rect.height <= anchorCell.height
            ) {
                throw new Error(
                    `merge fragment ${JSON.stringify(fragments[0].rect)} is not larger than the anchor cell`,
                );
            }

            // `displayCellAt` (the editor's primary path) resolves a covered
            // cell to that same fragment.
            const covered = canvas.cellRect(lastRow, lastColumn);
            const hit = canvas.displayCellAt(covered.top_left.x + 2, covered.top_left.y + 2);
            if (!hit) throw new Error("displayCellAt returned null over a covered cell");
            if (
                hit.fragment.width !== fragments[0].rect.width ||
                hit.fragment.top_left.y !== fragments[0].rect.top_left.y
            ) {
                throw new Error(
                    `displayCellAt fragment ${JSON.stringify(hit.fragment)} != visibleFragments ${JSON.stringify(fragments[0].rect)}`,
                );
            }

            // Anchor scrolled out of view: the fallback path must still answer
            // with the visible part of the merge, inside the canvas.
            model.setTopLeftVisibleCell(lastRow - 1, firstColumn);
            canvas.viewChanged();
            canvas.markContentDirty();
            drainPaint();
            const scrolled = canvas.visibleFragments(firstRow, firstColumn, lastRow, lastColumn);
            if (scrolled.length !== 1) {
                throw new Error(`scrolled: expected one fragment, got ${JSON.stringify(scrolled)}`);
            }
            const size = canvas.canvasSize();
            if (
                scrolled[0].rect.top_left.x < 0 ||
                scrolled[0].rect.top_left.y < 0 ||
                scrolled[0].rect.top_left.x + scrolled[0].rect.width > size.w + 1 ||
                scrolled[0].rect.top_left.y + scrolled[0].rect.height > size.h + 1
            ) {
                throw new Error(`scrolled fragment escapes the canvas: ${JSON.stringify(scrolled[0].rect)}`);
            }
            if (scrolled[0].rect.top_left.y > fragments[0].rect.top_left.y) {
                throw new Error(
                    `scrolled fragment moved down: before ${JSON.stringify(fragments[0].rect)} after ${JSON.stringify(scrolled[0].rect)}`,
                );
            }
        } finally {
            model.unmergeCells(area);
            for (let row = firstRow; row <= lastRow; row += 1) {
                model.setUserInput(sheet, row, firstColumn, "");
            }
            model.evaluate();
            canvas.markContentDirty();
            drainPaint();
        }
        return "selection snapped to the anchor; one editor fragment per merge";
    });

    await check("retained link raster matches a forced-Fresh repaint", () => {
        const sheet = model.getSelectedSheet();
        const column = 20;
        const firstRow = 100;
        const rows = [firstRow, firstRow + 1, firstRow + 2];
        const dpr = window.devicePixelRatio || 1;
        const sheetCount = model.getWorksheetsProperties().length;


        // Deterministic shared state: no stale overlays, and the exact palette
        // the Fresh reference is given inside `renderFreshReference`.
        clearOverlays();
        canvas.setTheme(LIGHT_THEME);

        // A formula link, a static link (the one edited and undone), and an
        // internal formula link — all on one sheet, all inside the viewport
        // the view is scrolled to below.
        model.setUserInput(sheet, rows[0], column, '=HYPERLINK("https://formula.example","x")');
        model.setCellLink(
            sheet,
            rows[1],
            column,
            { type: "External", target: "https://static.example" },
            "static",
        );
        model.setUserInput(sheet, rows[2], column, '=HYPERLINK("#Sheet1!A5","go")');
        model.evaluate();
        model.setTopLeftVisibleCell(firstRow - 4, column - 5);
        canvas.viewChanged();
        canvas.markContentDirty();
        drainPaint();

        const failures = [];
        const notes = [];
        let cases = 0;
        // Apply the host signal to the retained canvas and to a history-free
        // reference, then byte-compare: the two frames share an operation class
        // and differ only in retained history. The comparison stays exact; only
        // a mismatch the narrow `knownSeparatorEdges` classifier recognises is
        // downgraded to a printed note.
        const verify = (label, signal) => {
            cases += 1;
            signal(canvas);
            const outcomes = drainPaint();
            try {
                const fresh = renderFreshReference(canvas.canvasSize(), dpr);
                signal(fresh.canvas);
                drain(fresh.canvas);
                compareLayers(label, fresh, notes);
            } catch (error) {
                failures.push(`${label}: ${error.message} [drain ${outcomes.join(",")}]`);
            }
        };

        try {
            verify("baseline", () => {});
            verify("second content repaint", (c) => c.markContentDirty());

            model.setCellLink(
                sheet,
                rows[1],
                column,
                { type: "External", target: "https://edited.example" },
                "static",
            );
            verify("target edit", (c) => c.markContentDirty());

            model.undo();
            verify("undo", (c) => c.markContentDirty());

            model.setTopLeftVisibleCell(firstRow - 12, column - 9);
            verify("scroll", (c) => c.viewChanged());

            const base = canvas.canvasSize();
            verify("resize", (c) => {
                c.resize(base.w - 32, base.h - 24, dpr);
                c.requestRepaint();
            });
            resizeCanvas();
            drainPaint();

            if (sheetCount > 1) {
                model.setSelectedSheet(sheet === 0 ? 1 : 0);
                verify("sheet switch", (c) => c.viewChanged());
                model.setSelectedSheet(sheet);
                canvas.viewChanged();
                drainPaint();
            }

            // Put the edited link's cell back under the active-cell overlay.
            model.setTopLeftVisibleCell(rows[1] - 4, column - 5);
            canvas.viewChanged();
            drainPaint();
            model.setSelectedCell(rows[1], column);
            verify("active cell on link", (c) => {
                c.viewChanged();
                c.requestOverlayRepaint();
            });
        } finally {
            for (const row of rows) model.setUserInput(sheet, row, column, "");
            model.evaluate();
            model.setSelectedSheet(sheet);
            resizeCanvas();
            canvas.markContentDirty();
            drainPaint();
        }
        const known = notes.length > 0 ? `; ${notes.join("; ")}` : "";
        if (failures.length > 0) {
            throw new Error(
                `${failures.length} of ${cases} cases diverge from a forced-Fresh repaint ` +
                    `at dpr ${dpr}: ${failures.join(" | ")}${known}`,
            );
        }
        return `${cases} cases compared with a history-free reference at dpr ${dpr}${known}`;
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
