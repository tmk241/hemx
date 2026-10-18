(() => {
  const ROOT = "data-hemx-root";
  const ACTION = "data-hemx-action";
  const RESOURCE = "data-hemx-resource";
  const COLLECTION = "data-hemx-collection";
  const KEY = "data-hemx-key";
  const runtimeAbiVersion = 3;
  const EFFECT_BATCH_ABI_VERSION = 3;
  const BUILD = "data-hemx-build";
  const ISLAND = "data-hemx-island";
  const dom = nativeDOM();
  const { rootOf, forEachElement, forEachOwned, select, firstElement, attrEquals, elements, unique,
    targetFor, collectionTarget, nearestKey, fieldTargets } =
    createResources(dom, { ROOT, ISLAND, RESOURCE, COLLECTION, KEY, ACTION });
  const morphing = createMorpher(dom, { ROOT, ISLAND, RESOURCE, KEY }, rootOf, updatePendingAttributes);
  const pending = new WeakMap();
  const queues = new WeakMap();
  const timers = new WeakMap();
  const everyTimers = new WeakMap();
  const revealObservers = new WeakMap();
  const revealed = new WeakMap();
  const revealBindings = new WeakMap();
  const pendingClassStates = new WeakMap();
  const indicatorStates = new WeakMap();
  const busyStates = new WeakMap();
  const disabledStates = new WeakMap();
  const sseSources = new WeakMap();
  const boundRoots = new WeakSet();
  const dragKeys = new WeakMap();
  const activeRequests = new Set();
  const REQUEST_TIMEOUT_MS = 10_000;
  const forms = createForms(dom, rootOf,
    (form, source) => handlesEvent(form, "submit") && Boolean(
      formHandleId(form) || (ownsSubmitter(form, source) && handleId(source)) ||
      pageFormHistoryMode(source, form) || boostRoot(form)),
    (form, source, capture) => schedule(form, "submit", source, capture));
  const isSubmitter = forms.isSubmitter;

  window.addEventListener("pagehide", () => {
    for (const controller of activeRequests) {
      controller.abort(new DOMException("hemx request cancelled because page is hidden", "AbortError"));
    }
  });

  function roots() {
    return dom.query(document, `[${ROOT}]`);
  }

  function formOwner(el) {
    if (!el || el instanceof HTMLFormElement) return el;
    const owner = rootOf(el);
    if (!owner) return null;
    if (el instanceof HTMLInputElement || el instanceof HTMLButtonElement ||
        el instanceof HTMLSelectElement || el instanceof HTMLTextAreaElement) {
      return el.form && rootOf(el.form) === owner ? el.form : null;
    }
    return closestInRoot(el, owner, (node) => node instanceof HTMLFormElement);
  }

  function elementById(scope, id) {
    if (attrEquals(scope, "id", id)) return scope;
    return firstElement(scope, (el) => attrEquals(el, "id", id));
  }

  function formHandleId(form) {
    if (!form) return null;
    const holder = dom.has(form, ACTION) ? form : firstElement(form, (el) => dom.has(el, ACTION));
    const raw = holder && dom.attr(holder, ACTION);
    return raw && /^\d+$/.test(raw) ? raw : null;
  }

  function closestInRoot(start, root, predicate) {
    if (!root) return null;
    for (let node = start; node; node = dom.parentElement(node)) {
      if (dom.element(node) && predicate(node)) return node;
      if (node === root || dom.has(node, ISLAND) || dom.has(node, ROOT)) break;
    }
    return null;
  }

  function handleId(el) {
    const raw = el && dom.attr(el, ACTION);
    return raw && /^\d+$/.test(raw) ? raw : null;
  }

  function normalizedPolicy(value) {
    return value === "latest" || value === "queue" || value === "drop" || value === "parallel" ? value : null;
  }

  function requestPolicy(el, eventName) {
    const policy = normalizedPolicy(dom.attr(el, "data-hemx-policy"));
    if (policy) return policy;
    if (dom.has(el, "data-hemx-debounce") || eventName === "input") return "latest";
    if (el instanceof HTMLFormElement) return "drop";
    return "parallel";
  }

  function beginPending(el, root) {
    const klass = dom.attr(el, "data-hemx-pending-class");
    const indicators = elements(root, (node) => dom.has(node, "data-hemx-indicator"));
    const controls = dom.has(el, "data-hemx-disable-while-pending") ? elements(el, isDisableControl) : [];
    if (klass) togglePendingClass(el, klass, true);
    toggleBusy(el, true);
    indicators.forEach((node) => toggleIndicator(node, true));
    controls.forEach((node) => toggleDisabled(node, true));
    let active = true;
    return () => {
      if (!active) return;
      active = false;
      if (klass) togglePendingClass(el, klass, false);
      toggleBusy(el, false);
      indicators.forEach((node) => toggleIndicator(node, false));
      controls.forEach((node) => toggleDisabled(node, false));
    };
  }

  // Pending feedback is a temporary transport overlay. A server update changes
  // the state restored by the remaining requests, not just the visible overlay.
  function updatePendingAttributes(node, source) {
    const busy = busyStates.get(node);
    if (busy) {
      busy.value = dom.attr(source, "aria-busy");
      dom.setAttribute(node, "aria-busy", "true");
    }
    const classes = pendingClassStates.get(node);
    if (classes) for (const [name, state] of classes) {
      state.hadClass = dom.classList(source).contains(name);
      dom.classList(node).add(name);
    }
    const indicator = indicatorStates.get(node);
    if (indicator) {
      indicator.hidden = dom.attr(source, "hidden");
      dom.removeAttribute(node, "hidden");
    }
    const disabled = disabledStates.get(node);
    if (disabled) {
      disabled.disabled = source.disabled;
      node.disabled = true;
    }
  }

  function toggleBusy(el, on) {
    const state = busyStates.get(el);
    if (on) {
      if (state) state.count += 1;
      else busyStates.set(el, { count: 1, value: dom.attr(el, "aria-busy") });
      dom.setAttribute(el, "aria-busy", "true");
      return;
    }
    if (!state) return;
    state.count -= 1;
    if (state.count <= 0) {
      if (state.value === null) dom.removeAttribute(el, "aria-busy");
      else dom.setAttribute(el, "aria-busy", state.value);
      busyStates.delete(el);
    }
  }

  function togglePendingClass(el, klass, on) {
    let classes = pendingClassStates.get(el);
    const state = classes && classes.get(klass);
    if (on) {
      if (!classes) { classes = new Map(); pendingClassStates.set(el, classes); }
      if (state) state.count += 1;
      else classes.set(klass, { count: 1, hadClass: dom.classList(el).contains(klass) });
      dom.classList(el).add(klass);
      return;
    }
    if (!state) return;
    state.count -= 1;
    if (state.count <= 0) {
      if (state.hadClass) dom.classList(el).add(klass);
      else dom.classList(el).remove(klass);
      classes.delete(klass);
      if (!classes.size) pendingClassStates.delete(el);
    }
  }

  function toggleIndicator(indicator, on) {
    const state = indicatorStates.get(indicator);
    if (on) {
      if (state) state.count += 1;
      else indicatorStates.set(indicator, { count: 1, hidden: dom.attr(indicator, "hidden") });
      dom.removeAttribute(indicator, "hidden");
      return;
    }
    if (!state) return;
    state.count -= 1;
    if (state.count <= 0) {
      if (state.hidden === null) dom.removeAttribute(indicator, "hidden");
      else dom.setAttribute(indicator, "hidden", state.hidden);
      indicatorStates.delete(indicator);
    }
  }

  function toggleDisabled(control, on) {
    const state = disabledStates.get(control);
    if (on) {
      if (state) state.count += 1;
      else disabledStates.set(control, { count: 1, disabled: control.disabled });
      control.disabled = true;
      return;
    }
    if (!state) return;
    state.count -= 1;
    if (state.count <= 0) {
      control.disabled = state.disabled;
      disabledStates.delete(control);
    }
  }

  function ownsSubmitter(form, source) {
    const sourceOwner = rootOf(source);
    return isSubmitter(source) && source.form === form &&
      (sourceOwner === rootOf(form) || (!sourceOwner && !closestInRoot(source, document, (node) => dom.has(node, ISLAND))));
  }

  function formDataFor(el, eventName, source = el, capture) {
    const form = capture ? capture.form : formOwner(el);
    const data = capture ? capture.data : form ? successfulFormData(form, source) : new FormData();
    const handled = eventName === "submit" && ownsSubmitter(form, source) && handleId(source) ? source : el;
    const id = handleId(handled) || formHandleId(form);
    if (id) data.set("__h", id);
    // Request fields are explicit: form controls plus data-hemx-param-*, and the
    // reserved __drag key for drops. Ordinary data-* stays DOM metadata.

    const dragKey = eventName === "drop" && dragKeys.get(rootOf(el));
    if (dragKey) data.set("__drag", dragKey);
    const PARAM = "data-hemx-param-";
    for (const { name, value } of dom.attributes(handled)) {
      if (name.startsWith(PARAM) && name.length > PARAM.length) data.set(name.slice(PARAM.length).replace(/-/g, "_"), value);
    }
    return { form, data, submission: capture ? capture.submission : forms.submission(form, source) };
  }

  function urlEncoded(data) {
    const encoded = new URLSearchParams();
    for (const [name, value] of data.entries()) {
      const text = value instanceof File ? value.name : value;
      encoded.append(name.replace(/\r\n|\r|\n/g, "\r\n"), text.replace(/\r\n|\r|\n/g, "\r\n"));
    }
    return encoded;
  }

  function requestBody(data, multipart) {
    return multipart ? data : urlEncoded(data);
  }

  function requestUrl(action, data, method) {
    const url = new URL(action, location.href);
    if (method === "GET") url.search = urlEncoded(data).toString();
    return url.href;
  }

  function successfulFormData(form, source) {
    return isSubmitter(source) ? new FormData(form, source) : new FormData(form);
  }

  function pageFormHistoryMode(source, form) {
    return historyMode(source, null) || historyMode(form, null) || historyMode(boostRoot(form), null);
  }

  function boostRoot(el) {
    return el && closestInRoot(dom.parentElement(el), rootOf(el), (node) => dom.has(node, "data-hemx-boost"));
  }

  function historyMode(el, fallback) {
    if (!el) return fallback;
    const value = dom.attr(el, "data-hemx-history") || dom.attr(el, "data-hemx-nav") || "";
    if (value === "push" || value === "replace" || value === "none") return value;
    if (dom.has(el, "data-hemx-history") || dom.has(el, "data-hemx-nav")) return fallback || "push";
    return fallback;
  }

  // A rejected response is never read. Cancel its body so the stream and
  // connection are released now, not whenever the response is collected.
  function discard(response) {
    response.body?.cancel().catch(() => {});
  }

  // A response from another build means this page is stale, whatever its status
  // (a stale form submission is answered 409). Report once; never replay.

  function staleError() {
    const error = new Error("hemx surface fingerprint mismatch; reload this page");
    error.stale = true;
    return error;
  }

  function requireUsable(response, root) {
    if (!compatibleFingerprint(response, root)) {
      discard(response);
      throw staleError();
    }
    if (!response.ok) {
      discard(response);
      throw httpError(response);
    }
  }

  function httpError(response) {
    const error = new Error(`HTTP ${response.status}`);
    error.status = response.status;
    return error;
  }

  function showError(el, error, root = rootOf(el)) {
    const message = !error ? "" : error.stale ? "This page is out of date. Reload to continue."
      : error.status ? `Request failed (${error.status})` : "Request failed";
    select(root, "[data-hemx-error]").forEach((outlet) => {
      dom.setText(outlet, message);
      if (message) dom.removeAttribute(outlet, "hidden");
      else dom.setAttribute(outlet, "hidden", "");
    });
  }


  async function send(el, eventName, source = el, capture) {
    const owner = rootOf(el);
    if (!dom.isConnected(el) || dom.ownerDocument(el) !== document || !owner) return;
    try {
      if (dom.attr(el, "data-hemx-confirm") && !confirm(dom.attr(el, "data-hemx-confirm"))) return;
      const target = formOwner(el) || el;
      const policy = requestPolicy(target, eventName);
      const active = pending.get(target);
      const tail = queues.get(target);
      if ((active || tail) && policy === "drop") return;
      // Capture successful controls once, at submission, also for queued work.
      const { form, data, submission } = formDataFor(el, eventName, source, capture);
      const { action, method, enctype } = submission;
      const mode = method === "GET" && form && (pageFormHistoryMode(source, form) || "push");
      const body = method === "GET" || method === "HEAD" ? undefined : requestBody(data, enctype === "multipart/form-data");
      const headers = { "X-HEMX-Partial": "1", "Accept": "application/hemx, text/html" };
      if (body instanceof URLSearchParams) headers["Content-Type"] = "application/x-www-form-urlencoded;charset=UTF-8";
      if (active && policy === "latest") {
        active.abort.abort();
        active.release();
      }

      const execute = async () => {
        if (!dom.isConnected(el) || dom.ownerDocument(el) !== document || rootOf(el) !== owner) return;
        const abort = new AbortController();
        const timeout = setTimeout(
          () => abort.abort(new DOMException(`hemx request timed out after ${REQUEST_TIMEOUT_MS} ms`, "TimeoutError")),
          REQUEST_TIMEOUT_MS,
        );
        activeRequests.add(abort);
        let finish;
        const done = new Promise((resolve) => { finish = resolve; });
        const release = beginPending(target, owner);
        pending.set(target, { abort, done, release });
        showError(target, null, owner);
        try {
          if (mode) {
            await navigateUrl(requestUrl(action, data, method), owner, mode, release, abort.signal);
            return;
          }
          const response = await fetch(requestUrl(action, data, method), {
            method, body, headers, credentials: "same-origin", signal: abort.signal,
          });
          if ((pending.get(target)?.abort !== abort && policy === "latest") ||
              !dom.isConnected(owner) || dom.ownerDocument(owner) !== document || !dom.has(owner, ROOT)) {
            discard(response);
            return;
          }
          requireUsable(response, owner);
          if (!await applyResponse(response, owner, release)) throw new Error("hemx response has no applicable content");
        } finally {
          clearTimeout(timeout);
          activeRequests.delete(abort);
          if (pending.get(target)?.abort === abort) pending.delete(target);
          release();
          finish();
        }
      };
      if ((active || tail) && policy === "queue") {
        const base = tail || active.done;
        let queued;
        queued = base.then(execute, execute).finally(() => {
          if (queues.get(target) === queued) queues.delete(target);
        });
        queues.set(target, queued);
        await queued;
      } else await execute();
    } catch (error) {
      if (error.name !== "AbortError") {
        showError(el, error, owner);
        if (!error.hemxReported) emit(owner, "hemx:error", { message: String(error), status: error.status || null });
      }
    }
  }

  async function navigate(anchor, mode = "push") {
    const href = anchor.href;
    const root = rootOf(anchor);
    const release = beginPending(anchor, root);
    try {
      await navigateUrl(href, root, mode, release);
    } finally {
      release();
    }
  }

  async function navigateUrl(href, root, mode = "replace", finishPending, signal) {
    try {
      const response = await fetch(href, {
        headers: { "X-HEMX-Partial": "1", "Accept": "text/html" },
        credentials: "same-origin", signal,
      });
      requireUsable(response, root);
      const result = await applyResponse(response, root, finishPending);
      if (result === "navigating") return true;
      if (!result) {
        fallbackNavigation(href, mode);
        return true;
      }
      if (mode === "push") history.pushState({ hemx: true }, "", href);
      else if (mode === "replace") history.replaceState({ hemx: true }, "", href);
      return true;
    } catch (error) {
      if (error.name !== "AbortError") {
        showError(root, error, root);
        if (!error.hemxReported) emit(root, "hemx:error", { message: String(error), status: error.status || null });
      }
      return false;
    }
  }

  function fallbackNavigation(href, mode) {
    if (mode === "none") location.reload();
    else location.href = href;
  }

  async function readWireBody(response) {
    if (Number(response.headers.get("content-length")) > MAX_WIRE_BYTES) {
      await response.body?.cancel();
      throw new Error(`hemx batch exceeds ${MAX_WIRE_BYTES} bytes`);
    }
    if (!response.body) return new ArrayBuffer(0);
    const reader = response.body.getReader();
    const chunks = [];
    let length = 0;
    try {
      while (true) {
        const { value, done } = await reader.read();
        if (done) break;
        if (value.byteLength > MAX_WIRE_BYTES - length) {
          await reader.cancel();
          throw new Error(`hemx batch exceeds ${MAX_WIRE_BYTES} bytes`);
        }
        length += value.byteLength;
        chunks.push(value);
      }
    } finally { reader.releaseLock(); }
    const bytes = new Uint8Array(length);
    let offset = 0;
    for (const chunk of chunks) { bytes.set(chunk, offset); offset += chunk.byteLength; }
    return bytes.buffer;
  }

  async function applyResponse(response, root, finishPending) {
    if (response.status === 204) { finishPending?.(); return true; }
    if (response.redirected) {
      discard(response);
      finishPending?.();
      location.href = response.url;
      return "navigating";
    }
    if (!compatibleFingerprint(response, root)) {
      discard(response);
      throw staleError();
    }
    const type = response.headers.get("content-type") || "";
    if (type.includes("text/html")) {
      const html = await response.text();
      finishPending?.();
      return updateHtml(html, root, response.headers.get("x-hemx-title"));
    }
    if (type.includes("application/hemx")) {
      const bytes = await readWireBody(response);
      finishPending?.();
      // A batch that found its rows drifted is being healed by a refetch; the
      // request itself succeeded.
      if (!applyBatch(bytes, root) && drifted.get(root || document) !== "healing") {
        const error = new Error("hemx effect application failed");
        error.hemxReported = true;
        throw error;
      }
      return true;
    }
    discard(response);
    finishPending?.();
    return false;
  }

  function compatibleFingerprint(response, root) {
    const received = response.headers.get("x-hemx-fingerprint");
    const expected = root && dom.attr(root, BUILD);
    return !received || !expected || received === expected;
  }

  function applyBatch(buffer, root) {
    const scope = root || document;
    try {
      return applyDecoded(decodeBatch(buffer), root);
    } catch (error) {
      emit(scope, "hemx:error", { message: String(error), code: error.code || null });
      return false;
    }
  }

  // Decoding has already checked every declaration's structure ();
  // effects then apply in order and stop at the first failure.
  function applyDecoded(batch, root) {
    const scope = root || document;
    try {
      const expected = root && dom.attr(root, BUILD);
      if (expected && String(batch.fingerprint) !== expected) throw staleError();
      for (const op of batch.ops) {
        if (!applyOp(scope, op)) return false;
      }
      roots().forEach(bindEnhancedRoot);
      return true;
    } catch (error) {
      if (error.code === DRIFT) recoverDrift(scope, root, error);
      else emit(scope, "hemx:error", { message: String(error), code: error.code || null });
      return false;
    }
  }

  function isInputControl(el) {
    return el instanceof HTMLInputElement || el instanceof HTMLTextAreaElement || el instanceof HTMLSelectElement;
  }

  function isDisableControl(el) {
    return isInputControl(el) || el instanceof HTMLButtonElement;
  }


  function clearField(scope, ref) {
    const controls = fieldTargets(scope, ref);
    if (!controls.length) return missing(scope, ref);
    if (controls.some((control) => !isInputControl(control))) throw new Error("hemx clearing requires native form controls");
    if (controls.some((control) => control instanceof HTMLSelectElement &&
        Array.from(control.options).some((option) => rootOf(option) !== rootOf(control)))) {
      throw new Error("hemx cannot clear selections owned by another root or island");
    }
    for (const control of controls) {
      if (control instanceof HTMLSelectElement) {
        for (const option of control.options) option.selected = false;
        control.selectedIndex = -1;
      } else if (control instanceof HTMLInputElement && (control.type === "checkbox" || control.type === "radio")) {
        control.checked = false;
      } else control.value = "";
    }
    return true;
  }

  // A collection row is exactly one element of the current root, stamped with
  // its key. Insertion and replacement share this shape (, ).
  function rowElement(nodes, ref, key, [action, adjective]) {
    const elements = nodes.filter(dom.element);
    if (elements.length !== 1 || nodes.some((node) => !dom.element(node) &&
        dom.nodeType(node) !== 8 && (dom.nodeType(node) !== 3 || dom.textContent(node).trim() !== ""))) {
      throw new Error(`hemx keyed ${action} requires one element`);
    }
    const row = elements[0];
    if (dom.has(row, ROOT) || dom.has(row, ISLAND)) throw new Error(`hemx ${adjective} row must belong to the current root`);
    const declared = dom.attr(row, KEY);
    if (declared !== null && declared !== key) throw new Error(`hemx ${adjective} key conflicts with rendered key`);
    if (attrEquals(row, RESOURCE, ref.resource.id) && !dom.has(row, COLLECTION)) {
      throw new Error(`hemx ${adjective} row shadows its collection resource`);
    }
    dom.setAttribute(row, KEY, key);
    return row;
  }

  // Without moveBefore a row can still be reinserted, unless that would lose
  // browser state it holds (); then nothing changes.
  function stateError(key) {
    return new Error(`hemx row ${key} holds focus, a selection, a frame, media or an open dialog and cannot move without native moveBefore`);
  }

  // Indices of a longest strictly increasing subsequence: those rows already
  // stand in the declared relative order and need not move.
  function stableIndices(values) {
    const tails = [], previous = new Array(values.length);
    for (let i = 0; i < values.length; i++) {
      let low = 0, high = tails.length;
      while (low < high) {
        const middle = (low + high) >> 1;
        if (values[tails[middle]] < values[i]) low = middle + 1; else high = middle;
      }
      previous[i] = low > 0 ? tails[low - 1] : -1;
      tails[low] = i;
    }
    const stable = new Set();
    for (let i = tails.length ? tails[tails.length - 1] : -1; i !== -1; i = previous[i]) stable.add(i);
    return stable;
  }

  // A collection holds exactly the declared keys in the declared order. Every
  // check runs before the first DOM change; retained rows keep their identity
  // and state, and only rows outside the stable order move natively.

  function applyRows(scope, op) {
    const container = collectionTarget(scope, op.target);
    if (!container) return missing(scope, op.target);
    const live = new Map();
    for (const child of dom.children(container)) {
      const key = dom.attr(child, KEY);
      if (key === null) throw new Error("hemx collection children must all be keyed rows");
      if (live.has(key)) throw new Error(`hemx ambiguous row ${key}`);
      live.set(key, child);
    }
    const supplied = new Map(op.rows.map(([key, html]) =>
      [key, rowElement(morphing.parse(container, html), op.target, key, ["row", "declared"])]));
    for (const key of op.order) {
      if (!live.has(key) && !supplied.has(key)) throw new Error(`hemx row ${key} has no element`);
    }
    const currentIndex = new Map(Array.from(live.keys(), (key, index) => [key, index]));
    const retained = op.order.filter((key) => live.has(key));
    const stable = stableIndices(retained.map((key) => currentIndex.get(key)));
    const moved = new Set(retained.filter((_, index) => !stable.has(index)));
    for (const key of moved) {
      if (!dom.canRelocate(container, live.get(key)) ||
          (supplied.has(key) && !dom.canRelocate(container, supplied.get(key)))) throw stateError(key);
    }
    // Every reconciliation is checked before the first one runs ().
    for (const [key, row] of supplied) if (live.has(key)) morphing.checkElement(live.get(key), row);
    // Commit: changed rows reconcile in place, absent rows go, then placement. A
    // row whose tag changed is a new element: placement must use that one.
    for (const [key, row] of supplied) if (live.has(key)) {
      forgetSubtreeOrders(live.get(key));
      live.set(key, morphing.morphElement(live.get(key), row));
    }
    for (const [key, row] of live) if (!op.order.includes(key)) dom.remove(row);
    let reference = null;
    for (let i = op.order.length - 1; i >= 0; i--) {
      const key = op.order[i];
      const node = live.get(key) || supplied.get(key);
      if (!live.has(key)) dom.insertBefore(container, node, reference);
      else if (moved.has(key)) morphing.move(container, node, reference);
      reference = node;
    }
    orderHashes.delete(container);
    return true;
  }

  // The order hash names a collection's key order (): FNV-1a 64 over
  // each key's UTF-8 bytes followed by 0xFF, exactly as hemx_core::order_hash.
  // Computed in 32-bit halves; the prime is 2^40 + 0x1b3.
  const utf8 = new TextEncoder();

  function orderHash(keys) {
    let hi = 0xcbf29ce4, lo = 0x84222325;
    const mix = (byte) => {
      lo = (lo ^ byte) >>> 0;
      const low = lo * 0x1b3;
      hi = (hi * 0x1b3 + ((lo << 8) >>> 0) + Math.floor(low / 0x100000000)) >>> 0;
      lo = low >>> 0;
    };
    for (const key of keys) {
      for (const byte of utf8.encode(key)) mix(byte);
      mix(0xff);
    }
    return (BigInt(hi) << 32n) | BigInt(lo);
  }

  // A collection's current order hash, computed from its rows once and then
  // carried forward by each applied delta. Anything else that may change the
  // rows (a Patch around them, a full Rows declaration, a page swap) forgets it.
  let orderHashes = new WeakMap();
  function currentOrder(container) {
    if (orderHashes.has(container)) return orderHashes.get(container);
    const keys = [];
    const seen = new Set();
    for (const child of dom.children(container)) {
      const key = dom.attr(child, KEY);
      if (key === null) throw new Error("hemx collection children must all be keyed rows");
      if (seen.has(key)) throw new Error(`hemx ambiguous row ${key}`);
      seen.add(key);
      keys.push(key);
    }
    const hash = orderHash(keys);
    orderHashes.set(container, hash);
    return hash;
  }
  function forgetOrders(nodes) {
    for (const node of nodes) if (node) orderHashes.delete(node);
  }
  // Reconciliation can retain a nested collection element while changing its
  // children. Forget before changing it, not just the outer collection's hash.
  function forgetSubtreeOrders(node) {
    forgetOrders([node, ...dom.query(node, `[${COLLECTION}]`)]);
  }

  // A delta changes a collection from one named order to another ().
  // At its `to` order it is a duplicate and is skipped; anywhere else but
  // `from` the client missed something and nothing changes. Every step is
  // checked against the live rows and supplied HTML first ().

  function applyRowsDelta(scope, op) {
    const container = collectionTarget(scope, op.target);
    if (!container) return missing(scope, op.target);
    const state = currentOrder(container);
    if (state !== op.from) {
      // Skipping a duplicate applies nothing, so it proves nothing about the page.
      if (state === op.to) return true;
      const error = new Error("hemx rows are not in the state this change starts from");
      error.code = DRIFT;
      throw error;
    }
    // A few steps look rows up natively; a delta with many steps indexes
    // the rows once instead of searching them for every step.
    const found = new Map();
    const indexed = op.steps.length + op.rows.length > 16;
    if (indexed) {
      for (const child of dom.children(container)) {
        const key = dom.attr(child, KEY);
        if (found.has(key)) throw new Error(`hemx ambiguous row ${key}`);
        found.set(key, child);
      }
    }
    const liveRow = (key) => {
      if (indexed) return found.get(key) || null;
      if (!found.has(key)) {
        const rows = dom.query(container, `:scope > ${dom.attrSelector(KEY, key)}`);
        if (rows.length > 1) throw new Error(`hemx ambiguous row ${key}`);
        found.set(key, rows[0] || null);
      }
      return found.get(key);
    };
    const html = new Map(op.rows);
    const parsed = new Map();
    const parse = (key) => rowElement(morphing.parse(container, html.get(key)), op.target, key, ["row", "declared"]);
    for (const key of html.keys()) parsed.set(key, parse(key));
    // Preflight on a model: which keys are live after each step.
    const removed = new Set(), created = new Set(), everRemoved = new Set();
    const alive = (key) => created.has(key) || (!removed.has(key) && liveRow(key) !== null);
    const moving = new Set();
    for (const step of op.steps) {
      if (step.op === "remove") {
        if (!alive(step.key)) throw new Error(`hemx row ${step.key} is not there to remove`);
        removed.add(step.key);
        created.delete(step.key);
        everRemoved.add(step.key);
      } else {
        if (step.before !== null && (step.before === step.key || !alive(step.before))) {
          throw new Error(`hemx row ${step.key} has no place before ${step.before}`);
        }
        if (alive(step.key)) {
          if (!created.has(step.key)) moving.add(step.key);
        } else if (html.has(step.key)) {
          created.add(step.key);
        } else {
          throw new Error(`hemx row ${step.key} has no element`);
        }
      }
    }
    for (const key of moving) {
      if (!dom.canRelocate(container, liveRow(key)) ||
          (parsed.has(key) && !dom.canRelocate(container, parsed.get(key)))) throw stateError(key);
    }
    const reconciled = [...html.keys()].filter((key) => !everRemoved.has(key) && liveRow(key));
    for (const key of reconciled) morphing.checkElement(liveRow(key), parsed.get(key));
    // Commit: live rows that stay reconcile in place (a changed tag is a new
    // element, which the steps must see), then the steps run.
    for (const key of reconciled) {
      forgetSubtreeOrders(liveRow(key));
      found.set(key, morphing.morphElement(liveRow(key), parsed.get(key)));
    }
    const current = new Map();
    const node = (key) => current.has(key) ? current.get(key) : liveRow(key);
    const fresh = new Set();
    for (const step of op.steps) {
      if (step.op === "remove") {
        dom.remove(node(step.key));
        current.set(step.key, null);
        continue;
      }
      const reference = step.before === null ? null : node(step.before);
      let row = node(step.key);
      if (row && !fresh.has(row)) {
        morphing.move(container, row, reference);
        continue;
      }
      if (!row) {
        row = parsed.get(step.key);
        if (!row || dom.parentNode(row)) row = parse(step.key);
        fresh.add(row);
        current.set(step.key, row);
      }
      dom.insertBefore(container, row, reference);
    }
    orderHashes.set(container, op.to);
    drifted.delete(scope);
    return true;
  }

  // A client that missed an update heals by fetching its page again, once
  // (). If the next delta still finds another order before any delta
  // applied, refetching will not help: report it instead of looping. Missing
  // capabilities (moveBefore) are not drift and never refetch.
  const DRIFT = "HEMX_ROWS_DRIFT";
  const drifted = new WeakMap();

  function recoverDrift(scope, root, error) {
    const state = drifted.get(scope);
    if (state === "healing") return;
    if (state === "healed") {
      emit(scope, "hemx:error", { message: `${error.message}; reload to continue`, code: DRIFT });
      return;
    }
    drifted.set(scope, "healing");
    emit(scope, "hemx:resync", null);
    // A delta that applied while the refetch ran has already cleared the guard;
    // the refetch finishing must not put it back.
    navigateUrl(location.href, root, "none").finally(() => {
      if (drifted.get(scope) === "healing") drifted.set(scope, "healed");
    });
  }

  function applyOp(scope, op) {
    if (op.kind === "patch") {
      if (op.mode === "morph" && op.target.scope?.kind === "field") {
        if (op.html !== "") throw new Error("hemx form-field HTML patches only support clearing");
        return clearField(scope, op.target);
      }
      const target = targetFor(scope, op.target);
      if (!target) return missing(scope, op.target);
      // A Patch may change rows of collections inside it; replacing a row may
      // change its collection's order. Reconciling a row keeps its key.
      forgetSubtreeOrders(target);
      if (op.mode === "replace") forgetOrders([dom.parentNode(target)]);
      if (op.mode === "replace") {
        const parent = dom.parentNode(target);
        if (!parent) throw new Error("hemx replacement requires a parent");
        const nodes = morphing.parse(parent, op.html);
        dom.replaceWith(target, ...nodes);
      } else if (op.target.scope?.kind === "key" && !attrEquals(target, RESOURCE, op.target.resource.id)) {
        // A collection row is the keyed element itself: reconcile the rendered
        // row with it, as a Rows declaration would create it, never nest it.

        const nodes = morphing.parse(dom.parentNode(target), op.html);
        morphing.morphElement(target, rowElement(nodes, op.target, op.target.scope.value, ["replacement", "replacement"]));
      } else morphing.morph(target, op.html);
    } else if (op.kind === "rows") {
      return applyRows(scope, op);
    } else if (op.kind === "rowsDelta") {
      return applyRowsDelta(scope, op);
    } else if (op.kind === "focus") {
      const fields = op.target.scope?.kind === "field" ? fieldTargets(scope, op.target) : null;
      const target = fields ? fields.find((field) => field instanceof HTMLInputElement && field.type === "radio" && field.checked && !field.disabled) ||
        fields.find((field) => !field.disabled) || fields[0] : targetFor(scope, op.target);
      if (!target) return missing(scope, op.target);
      dom.focus(target, { preventScroll: op.preventScroll });
    } else if (op.kind === "scroll") {
      const target = targetFor(scope, op.target);
      if (!target) return missing(scope, op.target);
      dom.scrollIntoView(target, { block: op.block, behavior: op.behavior });
    } else if (op.kind === "visit") {
      navigateUrl(op.url, scope, op.history).catch((error) => emit(scope, "hemx:error", String(error)));
    } else if (op.kind === "dispatch") {
      emit(scope, op.event, op.payload);
    }
    return true;
  }

  function missing(root, target) {
    emit(root, "hemx:missing-target", target);
    return false;
  }

  function applyHtml(html, root, title) {
    try {
      const applied = updateHtml(html, root, title);
      if (!applied) emit(root, "hemx:missing-content-slot", null);
      return applied;
    } catch (error) {
      emit(root, "hemx:error", { message: String(error) });
      return false;
    }
  }

  function sourceScope(doc, scope) {
    // A fragment with an outer slot already identifies its update boundary.
    if (elements(doc.body, (node) => dom.has(node, RESOURCE) || namedSlot(node, "content")).length) return doc.body;
    const incoming = [];
    forEachElement(doc, (node) => { if (dom.has(node, ROOT)) incoming.push(node); });
    if (!incoming.length) return doc.body;
    const id = dom.attr(scope, "id");
    if (id) return unique(incoming.filter((node) => attrEquals(node, "id", id)), `HTML root ${id}`);
    const name = dom.attr(scope, ROOT);
    if (name === null) return null;
    const parentRoot = closestRootAncestor(scope);
    if (parentRoot && incoming.length > 1) throw new Error("hemx nested HTML root requires an unambiguous identity");
    const top = incoming.filter((node) => !closestRootAncestor(node));
    return unique(top.filter((node) => dom.attr(node, ROOT) === name), "HTML root");
  }

  function closestRootAncestor(node) {
    for (let parent = dom.parentElement(node); parent; parent = dom.parentElement(parent)) {
      if (dom.has(parent, ROOT)) return parent;
    }
    return null;
  }


  function updateHtml(html, root, title) {
    const scope = root || document;
    const doc = new DOMParser().parseFromString(html, "text/html");
    const source = sourceScope(doc, scope);
    if (!source) return false;
    const template = firstElement(source, (node) => node instanceof HTMLTemplateElement && dom.has(node, "data-hemx"));
    orderHashes = new WeakMap();
    const lowered = replaceLoweredSlots(scope, source);
    const named = !lowered && replaceSlot(scope, source, "content", template ? dom.innerHTML(template) : html);
    if (!lowered && !named) return false;
    if (!lowered) replaceSlot(scope, source, "nav");
    const nextTitle = title || doc.title;
    if (nextTitle) document.title = nextTitle;
    roots().forEach(bindEnhancedRoot);
    return true;
  }

  function replaceLoweredSlots(scope, sourceScope) {
    const plans = [];
    let missingTarget = false;
    forEachOwned(sourceScope, (source) => {
      const id = dom.attr(source, RESOURCE);
      if (id === null) return;
      if (source !== sourceScope) {
        for (let parent = dom.parentElement(source); parent; parent = dom.parentElement(parent)) {
          if (dom.has(parent, RESOURCE)) return;
          if (parent === sourceScope) break;
        }
      }
      if (!/^\d+$/.test(id) || Number(id) > 0xffffffff || String(Number(id)) !== id) throw new Error("hemx invalid resource marker in HTML response");
      const key = nearestKey(source, sourceScope);
      const ref = { resource: { kind: "slot", id: Number(id) }, scope: key === null ? null : { kind: "key", value: key } };
      const target = dom.has(source, COLLECTION) ? collectionTarget(scope, ref) : targetFor(scope, ref);
      if (!target) { missingTarget = true; return; }
      if (plans.some((plan) => dom.contains(plan.target, target) || dom.contains(target, plan.target))) {
        throw new Error("hemx overlapping HTML slot updates require an enclosing resource");
      }
      plans.push({ target, source });
    });
    if (missingTarget) return false;
    for (const plan of plans) morphing.morphElement(plan.target, plan.source);
    return plans.length > 0;
  }

  function namedSlot(el, name) {
    return attrEquals(el, "data-hemx-slot", name) || attrEquals(el, "data-slot", name);
  }

  function replaceSlot(scope, sourceScope, name, fallback) {
    const target = unique(elements(scope, (node) => namedSlot(node, name)), `named slot ${name}`);
    if (!target) return false;
    const source = unique(elements(sourceScope, (node) => namedSlot(node, name)), `source named slot ${name}`);
    if (source) morphing.morphElement(target, source);
    else if (fallback !== undefined) morphing.morph(target, fallback);
    else return false;
    return true;
  }

  function emit(root, name, detail) {
    dom.dispatch(root || document, new CustomEvent(name, { bubbles: true, detail }));
  }

  function defaultEvent(el) {
    if (dom.attr(el, "data-hemx-on")) return dom.attr(el, "data-hemx-on").trim().split(/\s+/)[0];
    if (el instanceof HTMLFormElement) return "submit";
    return "click";
  }

  function handlesEvent(el, name) {
    const declared = dom.attr(el, "data-hemx-on");
    return declared ? declared.trim().split(/\s+/).includes(name) : defaultEvent(el) === name;
  }

  function bindRoot(root) {
    if (boundRoots.has(root)) return;
    boundRoots.add(root);
    ["click", "submit", "input", "change", "keydown", "dragstart", "dragover", "drop"].forEach((name) => {
      if (name === "submit") return; // Native submission admission is owned by forms.js.
      dom.on(root, name, (event) => {
        if (!dom.isConnected(root) || dom.ownerDocument(root) !== document || event.defaultPrevented || rootOf(event.target) !== root) return;
        if (name === "dragstart") {
          const item = closestInRoot(event.target, root, (el) => dom.has(el, "data-hemx-key"));
          if (item) {
            dragKeys.set(root, dom.attr(item, "data-hemx-key"));
            if (event.dataTransfer) event.dataTransfer.setData("text/plain", dom.attr(item, "data-hemx-key"));
          }
          return;
        }
        if (name === "dragover") {
          const drop = closestInRoot(event.target, root, (el) => dom.has(el, ACTION) && dom.attr(el, "data-hemx-on") === "drop");
          if (drop) event.preventDefault();
          return;
        }
        const nav = closestInRoot(event.target, root, (el) =>
          (el instanceof HTMLAnchorElement && dom.has(el, "data-hemx-nav")) ||
          (el instanceof HTMLAnchorElement && dom.has(el, "href") && closestInRoot(dom.parentElement(el), root, (node) => dom.has(node, "data-hemx-boost")))
        );
        if (name === "click" && nav && sameOriginNav(event, nav)) {
          event.preventDefault();
          navigate(nav, historyMode(nav, "push"));
          return;
        }
        if (name === "click") {
          const submitter = closestInRoot(event.target, root, isSubmitter);
          if (submitter && submitter.form) return;
          const direct = closestInRoot(event.target, root, (el) => dom.has(el, ACTION));
          if (direct && handlesEvent(direct, "click")) {
            if (direct instanceof HTMLAnchorElement && dom.has(direct, "href")) event.preventDefault();
            schedule(direct, name);
            return;
          }
        }
        let el = closestInRoot(event.target, root, (node) =>
          dom.has(node, ACTION) ||
          (node instanceof HTMLFormElement && (pageFormHistoryMode(event.submitter || node, node) || boostRoot(node)))
        );
        if (!el || !handlesEvent(el, name)) return;
        if (name === "drop") event.preventDefault();
        schedule(el, name, event.submitter || el);
      });
    });
  }

  function schedule(el, eventName, source = el, capture) {
    const owner = rootOf(el);
    if (!owner || !dom.isConnected(el) || dom.ownerDocument(el) !== document) return;
    try {
      const debounce = duration(dom.attr(el, "data-hemx-debounce"));
      const throttle = duration(dom.attr(el, "data-hemx-throttle"));
      if (timers.has(el) && timers.get(el).owner !== owner) cancelScheduled(el);
      if (debounce) {
        cancelScheduled(el);
        const id = setTimeout(() => {
          timers.delete(el);
          if (rootOf(el) === owner) send(el, eventName, source, capture);
        }, debounce);
        timers.set(el, { id, owner });
      } else if (throttle) {
        if (timers.has(el)) return;
        const id = setTimeout(() => timers.delete(el), throttle);
        timers.set(el, { id, owner });
        send(el, eventName, source, capture);
      } else send(el, eventName, source, capture);
    } catch (error) {
      emit(owner, "hemx:error", { message: String(error) });
    }
  }

  function cancelScheduled(el) {
    clearTimeout(timers.get(el)?.id);
    timers.delete(el);
  }

  // Removed attributes and moved nodes are handled by the mutation observer;
  // scanning only current declarations keeps rebinding proportional to them.
  function bindPolling(root) {
    select(root, "[data-hemx-every],[data-hemx-interval]").forEach((el) => {
      const eventName = dom.has(el, "data-hemx-interval") ? "interval" : "every";
      const setting = dom.attr(el, `data-hemx-${eventName}`);
      const previous = everyTimers.get(el);
      if (previous && (previous.owner !== root || previous.setting !== setting || previous.eventName !== eventName)) stopPolling(el);
      if (setting === null || everyTimers.has(el)) return;
      const ms = duration(setting);
      if (!ms) return;
      const id = setInterval(() => {
        if (!dom.isConnected(el) || dom.ownerDocument(el) !== document || rootOf(el) !== root || dom.attr(el, `data-hemx-${eventName}`) !== setting) stopPolling(el);
        else schedule(el, eventName);
      }, ms);
      everyTimers.set(el, { id, owner: root, setting, eventName });
    });
  }

  function stopPolling(el) {
    clearInterval(everyTimers.get(el)?.id);
    everyTimers.delete(el);
  }

  function revealedRootMargin(el) {
    const ahead = Number(dom.attr(el, "data-hemx-revealed-ahead") || "0");
    const viewports = Number.isFinite(ahead) && ahead >= 0 ? ahead : 0;
    return `0px 0px ${viewports * window.innerHeight}px 0px`;
  }

  function stopRevealed(el) {
    const binding = revealBindings.get(el);
    if (binding) binding.observer.unobserve(el);
    revealBindings.delete(el);
  }

  function bindRevealed(root) {
    let observers = revealObservers.get(root);
    select(root, "[data-hemx-revealed]").forEach((el) => {
      if (revealed.get(el) === root) {
        stopRevealed(el);
        return;
      }
      if (typeof IntersectionObserver === "undefined") throw new Error("hemx revealed requires IntersectionObserver");
      const rootMargin = revealedRootMargin(el);
      const binding = revealBindings.get(el);
      if (binding && binding.root === root && binding.rootMargin === rootMargin) return;
      stopRevealed(el);
      if (!observers) {
        observers = new Map();
        revealObservers.set(root, observers);
      }
      let observer = observers.get(rootMargin);
      if (!observer) {
        observer = new IntersectionObserver((entries) => {
          entries.forEach((entry) => {
            const node = entry.target;
            if (revealBindings.get(node)?.observer !== observer) return;
            if (!dom.isConnected(node) || dom.ownerDocument(node) !== document || rootOf(node) !== root || !dom.has(node, "data-hemx-revealed")) {
              stopRevealed(node);
              return;
            }
            if (!entry.isIntersecting || revealed.get(node) === root) return;
            revealed.set(node, root);
            stopRevealed(node);
            schedule(node, "revealed");
          });
        }, { rootMargin });
        observers.set(rootMargin, observer);
      }
      revealBindings.set(el, { root, rootMargin, observer });
      observer.observe(el);
    });
  }

  function activeRoot(root) {
    return dom.isConnected(root) && dom.ownerDocument(root) === document && dom.has(root, ROOT);
  }

  function bindSse(root) {
    const url = dom.attr(root, "data-hemx-sse");
    const previous = sseSources.get(root);
    let href;
    try {
      if (url) {
        href = new URL(url, document.baseURI);
        if (href.origin !== location.origin || !/^https?:$/.test(href.protocol) || href.username || href.password) throw new Error("invalid stream URL");
        if (previous && previous.url === href.href) return;
      }
    } catch (_) {
      sseSources.delete(root);
      previous?.close();
      throw new Error("hemx SSE requires a same-origin HTTP(S) URL without embedded credentials");
    }
    sseSources.delete(root);
    previous?.close();
    if (!url) return;
    if (typeof EventSource === "undefined") throw new Error("hemx SSE requires EventSource");
    const source = new EventSource(href.href);
    const current = () => activeRoot(root) && sseSources.get(root) === source;
    source.addEventListener("hemx", (event) => { if (current()) applySseMessage(root, event); });
    source.addEventListener("message", (event) => { if (current()) applySseMessage(root, event); });
    source.addEventListener("error", () => { if (current()) emit(root, "hemx:sse-error", "SSE connection failed"); });
    sseSources.set(root, source);
  }

  // Drain once per frame, but preserve every operation and message boundary.
  // Even Morph A -> B -> A differs from just A for a dirty input. A failed
  // message stops its own effects, not the next independently received batch.

  const sseInbox = new WeakMap();
  function applySseMessage(root, event) {
    let inbox = sseInbox.get(root);
    if (!inbox) {
      inbox = [];
      sseInbox.set(root, inbox);
      const flush = () => {
        sseInbox.delete(root);
        if (!activeRoot(root)) return;
        for (const batch of inbox) applyDecoded(batch, root);
      };
      // Hidden documents get no animation frames; drain their queue on a timer.
      if (document.hidden || typeof requestAnimationFrame !== "function") setTimeout(flush, 100);
      else requestAnimationFrame(flush);
    }
    try {
      inbox.push(decodeBatch(base64UrlBytes(event.data).buffer));
    } catch (error) {
      emit(root, "hemx:error", { message: String(error), code: error.code || null });
    }
  }

  function duration(value) {
    if (value === null || value === undefined) return 0;
    const match = String(value).trim().match(/^(\d+)(ms|s)?$/);
    const ms = match ? Number(match[1]) * (match[2] === "s" ? 1000 : 1) : NaN;
    if (!Number.isSafeInteger(ms) || ms < 0 || ms > 0x7fffffff) throw new Error("hemx duration must be an integer from 0 to 2147483647 milliseconds");
    return ms;
  }

  function base64UrlBytes(encoded) {
    if (typeof encoded !== "string" || encoded.length > Math.ceil(MAX_WIRE_BYTES * 4 / 3) + 4) {
      throw new Error(`encoded hemx state exceeds ${MAX_WIRE_BYTES} bytes`);
    }
    const normalized = encoded.replace(/-/g, "+").replace(/_/g, "/");
    const padded = normalized + "=".repeat((4 - normalized.length % 4) % 4);
    return Uint8Array.from(atob(padded), (ch) => ch.charCodeAt(0));
  }

  const MAX_WIRE_BYTES = 1024 * 1024;
  const MAX_WIRE_ITEMS = 1024;
  const MAX_ROW_KEYS = 65536;
  const MAX_WIRE_FIELD_BYTES = 256 * 1024;

  function boundedLength(value, maximum, field) {
    if (!Number.isSafeInteger(value) || value < 0 || value > maximum) {
      throw new Error(`${field} length ${value} exceeds ${maximum}`);
    }
    return value;
  }

  function decoder(buffer) {
    const bytes = new Uint8Array(buffer);
    if (bytes.length > MAX_WIRE_BYTES) throw new Error(`hemx batch exceeds ${MAX_WIRE_BYTES} bytes`);
    let offset = 0;
    const need = (len) => {
      boundedLength(len, bytes.length - offset, "hemx batch field");
      const end = offset + len;
      if (end > bytes.length) throw new Error("truncated hemx batch");
      const slice = bytes.subarray(offset, end);
      offset = end;
      return slice;
    };
    const u8 = () => need(1)[0];
    const u32 = () => {
      const b = need(4);
      return (b[0] | (b[1] << 8) | (b[2] << 16) | (b[3] << 24)) >>> 0;
    };
    const u64 = () => {
      const lo = BigInt(u32());
      const hi = BigInt(u32());
      return lo | (hi << 32n);
    };
    const str = () => new TextDecoder("utf-8", { fatal: true, ignoreBOM: true }).decode(
      need(boundedLength(u32(), MAX_WIRE_FIELD_BYTES, "hemx string")),
    );
    const bytesField = () => need(boundedLength(u32(), MAX_WIRE_FIELD_BYTES, "hemx bytes"));
    const enumValue = (values, field) => {
      const discriminant = u8();
      if (discriminant >= values.length) throw new Error(`unknown ${field} ${discriminant}`);
      return values[discriminant];
    };
    const resource = () => {
      const kind = u8();
      if (kind === 0) return { kind: "slot", id: u32() };
      if (kind === 2) return { kind: "handle", id: u32() };
      if (kind === 3) return { kind: "form", id: u32() };
      throw new Error(`unknown hemx resource ${kind}`);
    };
    const scope = () => {
      const kind = u8();
      if (kind === 0) return null;
      if (kind === 1) return { kind: "key", value: str() };
      if (kind === 2) return { kind: "field", value: str() };
      throw new Error(`unknown hemx scope ${kind}`);
    };
    const ref = () => ({ resource: resource(), scope: scope() });
    // Same structural rules as the Rust codec (, ).
    const rowsDeclaration = () => {
      const target = ref();
      const order = [];
      const orderLength = boundedLength(u32(), Math.min(MAX_ROW_KEYS, Math.floor((bytes.length - offset) / 4)), "hemx Rows keys");
      for (let index = 0; index < orderLength; index += 1) order.push(str());
      const rowsLength = boundedLength(u32(), orderLength, "hemx Rows rows");
      const rows = [];
      for (let index = 0; index < rowsLength; index += 1) rows.push([str(), str()]);
      const ordered = new Set(order);
      const supplied = new Set();
      if (ordered.size !== order.length || rows.some(([key]) => !ordered.has(key) || supplied.size === supplied.add(key).size)) {
        throw new Error("hemx Rows declaration repeats a key or supplies a row outside its order");
      }
      return { kind: "rows", target, order, rows };
    };
    const rowsDelta = () => {
      const target = ref();
      const from = u64();
      const to = u64();
      const stepsLength = boundedLength(u32(), Math.min(MAX_ROW_KEYS, Math.floor((bytes.length - offset) / 5)), "hemx RowsDelta steps");
      const steps = [];
      for (let index = 0; index < stepsLength; index += 1) {
        const tag = u8();
        if (tag === 0) steps.push({ op: "remove", key: str() });
        else if (tag === 1) {
          const key = str();
          steps.push({ op: "place", key, before: enumValue([false, true], "hemx RowsDelta anchor") ? str() : null });
        } else throw new Error(`unknown hemx RowsDelta step ${tag}`);
      }
      const rowsLength = boundedLength(u32(), Math.min(MAX_ROW_KEYS, Math.floor((bytes.length - offset) / 8)), "hemx RowsDelta rows");
      const rows = [];
      for (let index = 0; index < rowsLength; index += 1) rows.push([str(), str()]);
      if (new Set(rows.map(([key]) => key)).size !== rows.length) {
        throw new Error("hemx row declaration repeats a key");
      }
      return { kind: "rowsDelta", target, from, to, steps, rows };
    };
    const effect = () => {
      const kind = u8();
      if (kind === 0) return { kind: "patch", target: ref(), mode: enumValue(["morph", "replace"], "hemx patch mode"), html: str() };
      if (kind === 1) return rowsDeclaration();
      if (kind === 2) return { kind: "focus", target: ref(), preventScroll: enumValue([false, true], "hemx boolean") };
      if (kind === 3) return { kind: "scroll", target: ref(), block: enumValue(["start", "center", "end", "nearest"], "hemx scroll block"), behavior: enumValue(["auto", "instant", "smooth"], "hemx scroll behavior") };
      if (kind === 4) return { kind: "visit", url: str(), history: enumValue(["push", "replace"], "hemx history mode") };
      if (kind === 5) return { kind: "dispatch", event: str(), payload: bytesField() };
      if (kind === 6) return rowsDelta();
      throw new Error(`unknown hemx effect ${kind}`);
    };
    const vec = (read) => {
      const length = boundedLength(u32(), MAX_WIRE_ITEMS, "hemx effect vector");
      const values = [];
      for (let index = 0; index < length; index += 1) values.push(read());
      return values;
    };
    return { u8, u32, u64, vec, effect, done: () => offset === bytes.length };
  }

  function decodeBatch(buffer) {
    const d = decoder(buffer);
    if (String.fromCharCode(d.u8(), d.u8(), d.u8(), d.u8()) !== "HEMX") throw new Error("bad hemx batch magic");
    const abiVersion = d.u32();
    if (abiVersion !== EFFECT_BATCH_ABI_VERSION) {
      const error = new Error(`unsupported hemx batch ABI version ${abiVersion}; expected ${EFFECT_BATCH_ABI_VERSION}`);
      error.code = "HEMX_UNSUPPORTED_ABI";
      throw error;
    }
    const batch = { abiVersion, fingerprint: d.u64(), ops: d.vec(d.effect) };
    if (!d.done()) throw new Error("trailing hemx batch bytes");
    return batch;
  }

  function sameOriginNav(event, anchor) {
    // Test the attribute: an empty `download` value still requests a download.
    return !event.defaultPrevented && event.button === 0 && !event.metaKey && !event.ctrlKey && !event.shiftKey && !event.altKey &&
      anchor.origin === location.origin && !dom.has(anchor, "download") && (!anchor.target || anchor.target === "_self");
  }

  function descendantRoots(node) {
    const found = [];
    forEachElement(node, (child) => { if (child !== node && dom.has(child, ROOT)) found.push(child); });
    return found;
  }

  function cleanupRemovedFragment(node) {
    forEachElement(node, (element) => {
      const connected = dom.isConnected(element) && dom.ownerDocument(element) === document;
      const owner = connected ? rootOf(element) : null;
      if (!connected || (timers.has(element) && timers.get(element).owner !== owner)) cancelScheduled(element);
      if (!connected || (everyTimers.has(element) && everyTimers.get(element).owner !== owner)) stopPolling(element);
      if (!connected || (revealBindings.has(element) && revealBindings.get(element).root !== owner)) stopRevealed(element);
      if (!connected || (revealed.has(element) && revealed.get(element) !== owner)) revealed.delete(element);
      if ((!connected || !dom.has(element, ROOT)) &&
          (dom.has(element, ROOT) || sseSources.has(element) || revealObservers.has(element))) cleanupRemovedRoot(element);
    });
  }

  function cleanupRemovedRoot(root) {
    const source = sseSources.get(root);
    sseSources.delete(root);
    source?.close();
    const observers = revealObservers.get(root);
    if (observers) observers.forEach((observer) => observer.disconnect());
    revealObservers.delete(root);
    forEachElement(root, (el) => {
      if (everyTimers.get(el)?.owner === root) stopPolling(el);
      if (revealBindings.get(el)?.root === root) stopRevealed(el);
    });
    // A connected move keeps its owner and listeners. Detached roots may later
    // reattach; the same guarded listener set remains valid without rebinding.
  }

  function rebindRevealed(resetDispatched) {
    roots().forEach((root) => {
      const observers = revealObservers.get(root);
      if (observers) observers.forEach((observer) => observer.disconnect());
      revealObservers.delete(root);
      forEachOwned(root, (el) => {
        stopRevealed(el);
        if (resetDispatched) revealed.delete(el);
      });
      bindAdapter(root, "revealed", () => bindRevealed(root));
    });
  }

  function restoreRevealed(event) {
    if (event.persisted) rebindRevealed(true);
  }

  function bindEnhancedRoot(root) {
    if (!activeRoot(root)) return;
    bindAdapter(root, "events", () => bindRoot(root));
    bindAdapter(root, "sse", () => bindSse(root));
    bindAdapter(root, "polling", () => bindPolling(root));
    bindAdapter(root, "revealed", () => bindRevealed(root));
  }

  function bindAdapter(root, name, bind) {
    try {
      bind();
    } catch (error) {
      showError(root, error);
      emit(root, `hemx:${name}-error`, String(error));
    }
  }

  function start() {
    forms.bind();
    roots().forEach(bindEnhancedRoot);
    new MutationObserver((records) => {
      const owners = new Set();
      const collect = (node) => {
        if (!dom.element(node)) return;
        cleanupRemovedFragment(node);
        if (dom.has(node, ROOT)) owners.add(node);
        descendantRoots(node).forEach((root) => owners.add(root));
        const owner = rootOf(node);
        if (owner) owners.add(owner);
      };
      for (const record of records) {
        if (record.type === "attributes") {
          const node = record.target;
          if (record.attributeName === "data-hemx-every" || record.attributeName === "data-hemx-interval") stopPolling(node);
          if (record.attributeName === "data-hemx-revealed" || record.attributeName === "data-hemx-revealed-ahead") stopRevealed(node);
          if (record.attributeName === "data-hemx-revealed" && !dom.has(node, "data-hemx-revealed")) revealed.delete(node);
          if (record.attributeName === "data-hemx-debounce" || record.attributeName === "data-hemx-throttle") cancelScheduled(node);
          collect(node);
        } else {
          record.removedNodes.forEach((node) => { if (dom.element(node)) cleanupRemovedFragment(node); });
          record.addedNodes.forEach(collect);
        }
      }
      owners.forEach(bindEnhancedRoot);
    }).observe(document.documentElement, {
      childList: true, subtree: true, attributes: true,
      attributeFilter: [ROOT, ISLAND, "data-hemx-sse", "data-hemx-every", "data-hemx-interval",
        "data-hemx-revealed", "data-hemx-revealed-ahead", "data-hemx-debounce", "data-hemx-throttle"],
    });
  }

  window.addEventListener("pageshow", restoreRevealed);
  window.addEventListener("resize", () => rebindRevealed(false));

  // The server is the truth for every history entry: refetch it, never restore
  // page HTML from history state (which would show stale server state).

  addEventListener("popstate", () => {
    const root = roots()[0];
    if (root) navigateUrl(location.href, root, "none").catch((error) => emit(root, "hemx:error", String(error)));
  });

  window.hemx = Object.freeze({
    runtimeAbiVersion,
    roots,
    rootOf,
    applyHtml,
    applyBatch,
    decodeBatch,
  });

  if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", start);
  else start();
})();
