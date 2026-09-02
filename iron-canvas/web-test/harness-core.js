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
    };
    const raw = {
        style: model.getCellStyle.bind(model),
        type: model.getCellType.bind(model),
        value: model.getFormattedCellValue.bind(model),
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
