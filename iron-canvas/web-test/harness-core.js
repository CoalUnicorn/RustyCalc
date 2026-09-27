export const DEMO_WORKBOOKS = Object.freeze({
    dynamic_arrays: {
        label: "Dynamic arrays",
        source: "demo/dynamic_arrays.xlsx",
        compiled: "demo/dynamic_arrays.ic",
        autofit: Object.freeze({ firstRow: 1, lastRow: 51, firstColumn: 1, lastColumn: 5 }),
    },
    forensics: {
        label: "Forensics",
        source: "demo/forensics.xlsx",
        compiled: "demo/forensics.ic",
        autofit: Object.freeze({ firstRow: 1, lastRow: 30, firstColumn: 1, lastColumn: 5 }),
    },
    sales_dashboard: {
        label: "Sales dashboard",
        source: "demo/sales_dashboard.xlsx",
        compiled: "demo/sales_dashboard.ic",
        autofit: Object.freeze({ firstRow: 1, lastRow: 10, firstColumn: 1, lastColumn: 7 }),
    },
});

const SAMPLE_AUTOFIT = Object.freeze([
    Object.freeze({ firstRow: 1, lastRow: 3, firstColumn: 1, lastColumn: 4 }),
    Object.freeze({ firstRow: 1, lastRow: 2, firstColumn: 1, lastColumn: 2 }),
    Object.freeze({ firstRow: 1, lastRow: 3, firstColumn: 1, lastColumn: 1 }),
]);

export function autofitPlanFor(workbookId, sheet = 0) {
    const plan =
        workbookId === "sample"
            ? SAMPLE_AUTOFIT[sheet]
            : sheet === 0
              ? DEMO_WORKBOOKS[workbookId]?.autofit
              : undefined;
    return plan ? { ...plan } : null;
}

export function columnLabel(column) {
    if (!Number.isInteger(column) || column < 1) throw new Error("Column must be 1-based");
    let value = column;
    let label = "";
    while (value > 0) {
        value -= 1;
        label = String.fromCharCode(65 + (value % 26)) + label;
        value = Math.floor(value / 26);
    }
    return label;
}

export function denseRowMajor(r1, c1, r2, c2, fetchCell) {
    const values = [];
    for (let row = r1; row <= r2; row += 1) {
        for (let column = c1; column <= c2; column += 1) {
            values.push(fetchCell(row, column));
        }
    }
    return values;
}

export function installDenseRangeMethods(model, onChange = () => {}) {
    const counts = {
        getCellStyle: 0,
        getCellType: 0,
        getFormattedCellValue: 0,
        getCellStylesIn: 0,
        getFormattedCellValuesIn: 0,
        getCellTypesIn: 0,
        getLinks: 0,
        getMergedCells: 0,
    };
    const raw = {
        style: model.getCellStyle.bind(model),
        type: model.getCellType.bind(model),
        value: model.getFormattedCellValue.bind(model),
        links: model.getLinks.bind(model),
        merges: model.getMergedCells.bind(model),
    };
    const count = (name) => {
        counts[name] += 1;
        onChange(counts);
    };

    model.getCellStyle = (sheet, row, column) => {
        count("getCellStyle");
        return raw.style(sheet, row, column);
    };
    model.getCellType = (sheet, row, column) => {
        count("getCellType");
        return raw.type(sheet, row, column);
    };
    model.getFormattedCellValue = (sheet, row, column) => {
        count("getFormattedCellValue");
        return raw.value(sheet, row, column);
    };
    model.getCellStylesIn = (sheet, r1, c1, r2, c2) => {
        count("getCellStylesIn");
        return denseRowMajor(r1, c1, r2, c2, (row, column) =>
            raw.style(sheet, row, column),
        );
    };
    model.getFormattedCellValuesIn = (sheet, r1, c1, r2, c2) => {
        count("getFormattedCellValuesIn");
        return denseRowMajor(r1, c1, r2, c2, (row, column) =>
            raw.value(sheet, row, column),
        );
    };
    model.getCellTypesIn = (sheet, r1, c1, r2, c2) => {
        count("getCellTypesIn");
        return denseRowMajor(r1, c1, r2, c2, (row, column) =>
            raw.type(sheet, row, column),
        );
    };
    model.getLinks = (sheet) => {
        count("getLinks");
        return raw.links(sheet);
    };
    model.getMergedCells = (sheet) => {
        count("getMergedCells");
        return raw.merges(sheet);
    };

    return {
        counts,
        reset() {
            for (const key of Object.keys(counts)) counts[key] = 0;
            onChange(counts);
        },
        snapshot() {
            return { ...counts };
        },
    };
}

/**
 * Known pre-existing core defect classifier, shared with the browser harness so
 * the rule itself is unit-testable. See
 * `docs/bugs/2026-09-26-retained-seam-alpha-accumulation.md`.
 *
 * A retained content repaint re-strokes a separator line on top of the previous
 * stroke, so its alpha accumulates and the pixels darken (203 -> 201 at DPR 1,
 * 205 -> 202 at fractional DPR). Accepting that as "known" is safe only while
 * it cannot hide a link regression: link text and underline pixels paint
 * strictly inside a cell, and any pixel strictly inside a painted cell rect
 * fails here.
 */
export const SEPARATOR_EDGE_SLACK = 1;

/** The painted separator edge this pixel sits on, or null. */
export function separatorEdge(geometry, x, y) {
    for (const column of geometry.columns) {
        for (const edge of [column.left, column.right]) {
            if (Math.abs(x - edge) <= SEPARATOR_EDGE_SLACK) return `x=${edge}`;
        }
    }
    for (const band of geometry.rowBands) {
        for (const edge of [band.top, band.bottom]) {
            if (Math.abs(y - edge) <= SEPARATOR_EDGE_SLACK) return `y=${edge}`;
        }
    }
    return null;
}

/**
 * The distinct separator lines a known separator defect covers, or null when
 * the mismatch is anything else. Every differing pixel must keep its alpha, be
 * the same RGB and strictly darker by one uniform per-channel delta, sit on a
 * painted separator edge, and lie strictly inside no painted cell rect.
 */
export function knownSeparatorEdges(diff, geometry) {
    if (diff.truncated || diff.points.length === 0 || geometry === null) return null;
    const edges = [];
    for (const { x, y, fresh, retained } of diff.points) {
        if (fresh[3] !== retained[3]) return null;
        const darkening = fresh[0] - retained[0];
        if (darkening <= 0) return null;
        if (fresh[1] - retained[1] !== darkening) return null;
        if (fresh[2] - retained[2] !== darkening) return null;
        if (retained[0] >= fresh[0] || retained[1] >= fresh[1] || retained[2] >= fresh[2]) return null;
        const interior = geometry.columns.some((column) =>
            geometry.rowBands.some(
                (band) =>
                    x > column.left + SEPARATOR_EDGE_SLACK &&
                    x < column.right - SEPARATOR_EDGE_SLACK &&
                    y > band.top + SEPARATOR_EDGE_SLACK &&
                    y < band.bottom - SEPARATOR_EDGE_SLACK,
            ),
        );
        if (interior) return null;
        const edge = separatorEdge(geometry, x, y);
        if (edge === null) return null;
        if (!edges.includes(edge)) edges.push(edge);
    }
    return edges;
}

export function rectCenter(rect) {
    if (!rect?.top_left || !Number.isFinite(rect.width) || !Number.isFinite(rect.height)) {
        throw new Error(`cellRect returned an unexpected shape: ${JSON.stringify(rect)}`);
    }
    return {
        x: rect.top_left.x + rect.width / 2,
        y: rect.top_left.y + rect.height / 2,
    };
}

export function detailFor(error) {
    if (error instanceof Error) return error.stack || error.message;
    if (error && typeof error === "object") {
        return JSON.stringify(error, Object.getOwnPropertyNames(error), 2);
    }
    return String(error);
}

export function queryOptions(search) {
    const params = new URLSearchParams(search);
    const requested = params.get("workbook") || "sample";
    return {
        autorun: params.get("autorun") === "1",
        workbook: requested === "sample" || requested in DEMO_WORKBOOKS ? requested : "sample",
    };
}
/**
 * Decide whether the one-shot rAF scheduler must stay armed after a drain.
 * `result` is the wasm `RenderResult` enum object. A held attempt
 * (`RetryRequired`) needs another frame with no new host signal; every
 * other outcome lets the loop sleep until the next `schedulePaint()` poke.
 */
export function shouldRescheduleAfterDrain(lastOutcome, result) {
    return lastOutcome === result.RetryRequired;
}
