const DATABASE = "hemx-sync-v1";
const STORE = "patches";
const SCHEMA_VERSION = 1;
const EVENT = "hemx:sync-patch";
const root = document.querySelector("[data-hemx-root]");
let database;
let pumping = false;

function requestResult(request) {
  return new Promise((resolve, reject) => {
    request.addEventListener("success", () => resolve(request.result), { once: true });
    request.addEventListener("error", () => reject(request.error), { once: true });
  });
}

function transactionDone(transaction) {
  return new Promise((resolve, reject) => {
    transaction.addEventListener("complete", resolve, { once: true });
    transaction.addEventListener("abort", () => reject(transaction.error), { once: true });
    transaction.addEventListener("error", () => reject(transaction.error), { once: true });
  });
}

async function openDatabase() {
  const request = indexedDB.open(DATABASE, 1);
  request.addEventListener("upgradeneeded", () => {
    if (!request.result.objectStoreNames.contains(STORE)) {
      request.result.createObjectStore(STORE, { keyPath: "idempotencyKey" });
    }
  });
  return requestResult(request);
}

function validIdentifier(value) {
  return typeof value === "string" && value.length > 0 && value.length <= 128 && /^[A-Za-z0-9:_.-]+$/.test(value);
}

function validKey(value) {
  return typeof value === "string"
    && value.length > 0
    && value.length <= 64
    && /^[A-Za-z][A-Za-z0-9_-]*$/.test(value)
    && !["schemaVersion", "idempotencyKey", "operationId", "key", "value"].includes(value);
}

function validatePatch(patch) {
  if (!patch || Object.getPrototypeOf(patch) !== Object.prototype) throw new Error("patch must be an object");
  const keys = Object.keys(patch).sort();
  const expected = ["idempotencyKey", "key", "operationId", "schemaVersion", "value"];
  if (keys.length !== expected.length || keys.some((key, index) => key !== expected[index])) {
    throw new Error("patch fields do not match schema");
  }
  if (patch.schemaVersion !== SCHEMA_VERSION) throw new Error(`unsupported patch schema version ${patch.schemaVersion}`);
  if (!validIdentifier(patch.idempotencyKey)) throw new Error("invalid idempotencyKey");
  if (!validIdentifier(patch.operationId)) throw new Error("invalid operationId");
  if (!validKey(patch.key)) throw new Error("invalid patch key");
  if (!["string", "number", "boolean"].includes(typeof patch.value)
      || (typeof patch.value === "number" && !Number.isSafeInteger(patch.value))
      || (typeof patch.value === "string" && patch.value.length > 4096)) {
    throw new Error("invalid patch value");
  }
  return patch;
}

function validateProjection(projection) {
  if (!Array.isArray(projection)
      || projection.length > 1024 * 1024
      || projection.some((byte) => !Number.isInteger(byte) || byte < 0 || byte > 255)) {
    throw new Error("invalid durable projection");
  }
  return projection;
}

function normalizeEvent(payload) {
  if (payload && Object.getPrototypeOf(payload) === Object.prototype && "patch" in payload) {
    const keys = Object.keys(payload).sort();
    if (keys.length !== 2 || keys[0] !== "patch" || keys[1] !== "projection") {
      throw new Error("durable patch fields do not match schema");
    }
    const patch = validatePatch(payload.patch);
    return { idempotencyKey: patch.idempotencyKey, patch, projection: validateProjection(payload.projection) };
  }
  const patch = validatePatch(payload);
  return { idempotencyKey: patch.idempotencyKey, patch, projection: null };
}

async function allPatches() {
  const transaction = database.transaction(STORE, "readonly");
  const done = transactionDone(transaction);
  const patches = await requestResult(transaction.objectStore(STORE).getAll());
  await done;
  return patches.sort((left, right) => left.queuedAt - right.queuedAt || left.idempotencyKey.localeCompare(right.idempotencyKey));
}

function normalizeStoredRecord(stored) {
  if (stored.patch) {
    return {
      patch: validatePatch(stored.patch),
      projection: stored.projection === null ? null : validateProjection(stored.projection),
    };
  }
  const { queuedAt: _queuedAt, ...legacyPatch } = stored;
  return { patch: validatePatch(legacyPatch), projection: null };
}

async function persist(record) {
  const transaction = database.transaction(STORE, "readwrite");
  const done = transactionDone(transaction);
  transaction.objectStore(STORE).add({ ...record, queuedAt: Date.now() });
  await done;
  root?.setAttribute("data-hemx-sync-pending", String((await allPatches()).length));
}

async function remove(idempotencyKey) {
  const transaction = database.transaction(STORE, "readwrite");
  const done = transactionDone(transaction);
  transaction.objectStore(STORE).delete(idempotencyKey);
  await done;
}

async function pump() {
  if (pumping || !navigator.onLine) return;
  pumping = true;
  try {
    for (const stored of await allPatches()) {
      const { patch } = normalizeStoredRecord(stored);
      const endpoint = root?.getAttribute("data-sync-endpoint") || "/sync/patches";
      const response = await fetch(endpoint, {
        method: "POST",
        credentials: "same-origin",
        headers: { "content-type": "application/json" },
        body: JSON.stringify(patch),
      });
      if (!response.ok) {
        root?.setAttribute("data-hemx-sync-error", `upload-${response.status}`);
        return;
      }
      const acknowledgement = await response.json();
      if (acknowledgement.idempotencyKey !== patch.idempotencyKey
          || acknowledgement.operationId !== patch.operationId) {
        root?.setAttribute("data-hemx-sync-error", "acknowledgement-mismatch");
        return;
      }
      await remove(patch.idempotencyKey);
      root?.setAttribute("data-hemx-sync-ack", acknowledgement.idempotencyKey);
    }
    root?.setAttribute("data-hemx-sync-pending", String((await allPatches()).length));
  } catch {
    root?.setAttribute("data-hemx-sync-error", "offline");
  } finally {
    pumping = false;
  }
}

async function start() {
  if (!root) return;
  database = await openDatabase();
  const pending = await allPatches();
  for (const stored of pending) {
    const { projection } = normalizeStoredRecord(stored);
    if (projection) {
      window.hemx?.applyBatch(Uint8Array.from(projection).buffer, root);
    }
  }
  document.addEventListener(EVENT, async (event) => {
    try {
      const record = normalizeEvent(JSON.parse(event.detail));
      await persist(record);
      await pump();
    } catch (error) {
      root.setAttribute("data-hemx-sync-error", error instanceof Error ? error.message : String(error));
    }
  });
  window.addEventListener("online", () => pump());
  root.setAttribute("data-hemx-sync-pending", String(pending.length));
  root.setAttribute("data-hemx-sync-ready", "");
  await pump();
}

start().catch((error) => root?.setAttribute("data-hemx-sync-error", error instanceof Error ? error.message : String(error)));
