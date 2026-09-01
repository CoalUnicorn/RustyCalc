import assert from "node:assert/strict";
import test from "node:test";

import {
    autofitPlanFor,
    columnLabel,
    denseRowMajor,
    installDenseRangeMethods,
    queryOptions,
    rectCenter,
} from "./harness-core.js";

test("autofit plans preserve the demo workbook used ranges", () => {
    assert.deepEqual(autofitPlanFor("forensics", 0), {
        firstRow: 1,
        lastRow: 30,
        firstColumn: 1,
        lastColumn: 5,
    });
    assert.equal(autofitPlanFor("forensics", 1), null);
    assert.deepEqual(autofitPlanFor("sample", 1), {
        firstRow: 1,
        lastRow: 2,
        firstColumn: 1,
        lastColumn: 2,
    });
});

test("columnLabel uses spreadsheet-style 1-based names", () => {
    assert.equal(columnLabel(1), "A");
    assert.equal(columnLabel(26), "Z");
    assert.equal(columnLabel(27), "AA");
    assert.throws(() => columnLabel(0), /1-based/);
});

test("denseRowMajor preserves inclusive row-major order", () => {
    assert.deepEqual(
        denseRowMajor(2, 3, 3, 5, (row, column) => `${row}:${column}`),
        ["2:3", "2:4", "2:5", "3:3", "3:4", "3:5"],
    );
});

test("dense range methods count one crossing and bypass scalar wrappers", () => {
    const model = {
        getCellStyle: (_sheet, row, column) => ({ row, column }),
        getCellType: (_sheet, row, column) => row + column,
        getFormattedCellValue: (_sheet, row, column) => `${row},${column}`,
    };
    const bridge = installDenseRangeMethods(model);

    assert.deepEqual(model.getCellStylesIn(0, 1, 1, 2, 2), [
        { row: 1, column: 1 },
        { row: 1, column: 2 },
        { row: 2, column: 1 },
        { row: 2, column: 2 },
    ]);
    assert.equal(bridge.counts.getCellStylesIn, 1);
    assert.equal(bridge.counts.getCellStyle, 0);

    model.getCellStyle(0, 1, 1);
    assert.equal(bridge.counts.getCellStyle, 1);
    bridge.reset();
    assert.deepEqual(bridge.snapshot(), {
        getCellStyle: 0,
        getCellType: 0,
        getFormattedCellValue: 0,
        getCellStylesIn: 0,
        getFormattedCellValuesIn: 0,
        getCellTypesIn: 0,
    });
});

test("rectCenter follows the serialized PixelRect shape", () => {
    assert.deepEqual(
        rectCenter({ top_left: { x: 60, y: 28 }, width: 80, height: 20 }),
        { x: 100, y: 38 },
    );
    assert.throws(() => rectCenter({ x: 0, y: 0, w: 10, h: 10 }), /unexpected shape/);
});

test("queryOptions accepts known demos and rejects unknown workbook ids", () => {
    assert.deepEqual(queryOptions("?autorun=1&workbook=forensics"), {
        autorun: true,
        workbook: "forensics",
    });
    assert.deepEqual(queryOptions("?workbook=unknown"), {
        autorun: false,
        workbook: "sample",
    });
});
