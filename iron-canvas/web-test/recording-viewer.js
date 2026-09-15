// Pure .icr analysis helpers. Keep DOM and Canvas2D concerns in the HTML
// shell so these functions can be exercised independently in a browser or
// JavaScript test runner.

export function findFullRebuildAnchor(frames, target) {
    const last = frames.length - 1;
    if (last < 0) return null;
    for (let i = Math.min(target, last); i >= 0; i--) {
        const trace = frames[i]?.trace;
        if (trace?.strategy === "full_rebuild" && trace.committed_seq != null) {
            return i;
        }
    }
    return null;
}

export function opKind(op) {
    if (typeof op === "string") return op;
    if (!op || typeof op !== "object") return "invalid";
    const keys = Object.keys(op);
    return keys.length === 1 ? keys[0] : "invalid";
}

export function summarizeOps(ops) {
    const kinds = new Map();
    const groups = new Map();
    const stack = [];
    const add = (map, key) => map.set(key, (map.get(key) || 0) + 1);

    for (const op of ops || []) {
        const kind = opKind(op);
        add(kinds, kind);
        add(groups, stack.length ? stack.join("/") : "(root)");
        if (kind === "BeginGroup") {
            const className = op.BeginGroup?.class || "?";
            stack.push(className);
        } else if (kind === "EndGroup") {
            if (stack.length) stack.pop();
        }
    }

    return {
        total: (ops || []).length,
        kinds: Object.fromEntries([...kinds].sort((a, b) => a[0].localeCompare(b[0]))),
        groups: Object.fromEntries([...groups].sort((a, b) => a[0].localeCompare(b[0]))),
        unbalanced: stack.length,
    };
}

export function diffOps(before, after) {
    const a = before || [];
    const b = after || [];
    const limit = Math.max(a.length, b.length);
    let firstDifferent = null;
    let changed = 0;
    let added = 0;
    let removed = 0;
    for (let i = 0; i < limit; i++) {
        const left = i < a.length ? JSON.stringify(a[i]) : undefined;
        const right = i < b.length ? JSON.stringify(b[i]) : undefined;
        if (left === right) continue;
        if (firstDifferent === null) firstDifferent = i;
        if (left === undefined) added++;
        else if (right === undefined) removed++;
        else changed++;
    }

    const beforeSummary = summarizeOps(a);
    const afterSummary = summarizeOps(b);
    const kindDelta = delta(beforeSummary.kinds, afterSummary.kinds);
    const groupDelta = delta(beforeSummary.groups, afterSummary.groups);
    return {
        beforeCount: a.length,
        afterCount: b.length,
        firstDifferent,
        changed,
        added,
        removed,
        kindDelta,
        groupDelta,
        beforeFirst: firstDifferent === null ? null : a[firstDifferent] ?? null,
        afterFirst: firstDifferent === null ? null : b[firstDifferent] ?? null,
    };
}

function delta(before, after) {
    const keys = new Set([...Object.keys(before), ...Object.keys(after)]);
    return [...keys]
        .sort((a, b) => a.localeCompare(b))
        .map((key) => ({
            key,
            before: before[key] || 0,
            after: after[key] || 0,
        }))
        .filter((entry) => entry.before !== entry.after);
}
