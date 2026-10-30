// The factory gives browser tests an isolated database and localStorage namespace.
export function createStorage(databaseName = "rustycalc", legacy = globalThis.localStorage) {
    const metadataStore = "workbooks";
    const bytesStore = "workbook_bytes";
    const settingsStore = "settings";
    let database;
    let queue = Promise.resolve();
    let metadata = {};

    function serialized(operation) {
        const result = queue.then(operation);
        queue = result.catch(() => {});
        return result;
    }

    function open() {
        if (database) return database;
        database = new Promise((resolve, reject) => {
            let rejected = false;
            const fail = error => { rejected = true; database = undefined; reject(error); };
            const request = indexedDB.open(databaseName, 1);
            request.onupgradeneeded = () => {
                for (const name of [metadataStore, bytesStore, settingsStore]) {
                    request.result.createObjectStore(name);
                }
            };
            request.onerror = () => fail(request.error);
            request.onblocked = () => {
                fail(new Error("Close other RustyCalc tabs, then retry opening storage."));
            };
            request.onsuccess = () => {
                const db = request.result;
                if (rejected) { db.close(); return; }
                db.onversionchange = () => { db.close(); database = undefined; };
                resolve(db);
            };
        });
        return database;
    }

    function requestValue(request) {
        return new Promise((resolve, reject) => {
            request.onsuccess = () => resolve(request.result);
            request.onerror = () => reject(request.error);
        });
    }

    async function transaction(mode, operation) {
        const db = await open();
        const tx = db.transaction([metadataStore, bytesStore, settingsStore], mode);
        const completed = new Promise((resolve, reject) => {
            tx.oncomplete = resolve;
            tx.onabort = () => reject(tx.error ?? new Error("Storage transaction aborted."));
            tx.onerror = () => {}; // The abort event reports the transaction failure.
        });
        try {
            const result = await operation(
                tx.objectStore(metadataStore), tx.objectStore(bytesStore), tx.objectStore(settingsStore),
            );
            await completed;
            return result;
        } catch (error) {
            try { tx.abort(); } catch { /* It may already have aborted. */ }
            await completed.catch(() => {});
            throw error;
        }
    }

    async function migrate() {
        const migrated = await transaction("readonly", (_meta, _bytes, settings) =>
            requestValue(settings.get("localStorageMigrated")));
        if (migrated) return;
        const raw = legacy.getItem("models");
        const registry = raw === null ? {} : JSON.parse(raw);
        if (!registry || typeof registry !== "object" || Array.isArray(registry)) {
            throw new Error("The old workbook registry is invalid. Its data has been kept.");
        }
        const entries = Object.entries(registry).map(([id, meta]) => {
            if (!/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(id)) {
                throw new Error("The old workbook registry contains an invalid ID.");
            }
            const encoded = legacy.getItem(id);
            if (encoded === null) throw new Error(`Missing saved bytes for ${id}. The registry has been kept.`);
            const value = JSON.parse(encoded);
            if (typeof value !== "string") throw new Error(`Invalid saved bytes for ${id}.`);
            const binary = atob(value);
            return [id, meta, Uint8Array.from(binary, ch => ch.charCodeAt(0))];
        });
        await transaction("readwrite", async (meta, bytes, settings) => {
            // Another tab may have completed migration since the readonly check.
            if (await requestValue(settings.get("localStorageMigrated"))) return;
            for (const [id, value, data] of entries) {
                if (await requestValue(meta.get(id)) === undefined) {
                    meta.put(value, id);
                    bytes.put(data, id);
                }
            }
            settings.put(true, "localStorageMigrated");
        });
        // A cleanup failure cannot invalidate the committed migration marker.
        try {
            for (const [id] of entries) legacy.removeItem(id);
            legacy.removeItem("models");
        } catch (error) {
            console.warn("Legacy workbook cleanup failed; migration is committed.", error);
        }
    }

    return {
        initialize: () => serialized(async () => {
            await migrate();
            metadata = await transaction("readonly", async meta => {
                const keys = requestValue(meta.getAllKeys());
                const values = requestValue(meta.getAll());
                const [ids, records] = await Promise.all([keys, values]);
                return Object.fromEntries(ids.map((key, index) => [key, records[index]]));
            });
        }),
        registry: () => JSON.stringify(metadata),
        load: id => serialized(() => transaction("readonly", (_meta, bytes) => requestValue(bytes.get(id)))),
        save: (id, name, data, origin) => {
            // Copy the Wasm view before yielding. Its allocation can be reused after return.
            const snapshot = Uint8Array.from(data);
            return serialized(async () => {
                const value = await transaction("readwrite", async (meta, bytes) => {
                    const previous = await requestValue(meta.get(id));
                    const existing = origin === "existing" || origin === "rename";
                    if (existing && !previous) throw new Error("The workbook was deleted. Save it as a new workbook.");
                    if (!existing && previous) throw new Error("The workbook ID already exists.");
                    const next = {
                        name, group: previous?.group ?? null, modified: Date.now(),
                        shared_from_link: origin === "rename" ? false : (previous?.shared_from_link ?? origin === "shared"),
                    };
                    meta.put(next, id);
                    bytes.put(snapshot, id);
                    return next;
                });
                metadata[id] = value;
            });
        },
        update: (id, patchJson) => serialized(async () => {
            const patch = JSON.parse(patchJson);
            const value = await transaction("readwrite", async meta => {
                const previous = await requestValue(meta.get(id));
                if (!previous) throw new Error("The workbook no longer exists.");
                const next = { ...previous, ...patch };
                meta.put(next, id);
                return next;
            });
            metadata[id] = value;
        }),
        delete: id => serialized(async () => {
            await transaction("readwrite", (meta, bytes) => {
                meta.delete(id);
                bytes.delete(id);
            });
            delete metadata[id];
        }),
        close: async () => {
            await queue;
            if (database) (await database).close();
            database = undefined;
        },
    };
}

let storage;
const current = () => storage ??= createStorage();
export const initializeStorage = () => current().initialize();
export const storageRegistry = () => current().registry();
export const loadWorkbook = id => current().load(id);
export const saveWorkbook = (id, name, bytes, origin) => current().save(id, name, bytes, origin);
export const updateWorkbook = (id, patch) => current().update(id, patch);
export const deleteWorkbook = id => current().delete(id);
