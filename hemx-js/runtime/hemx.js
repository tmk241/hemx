(() => {
  const ROOT = "data-hemx-root";
  const HID = "data-hid";
  const SID = "data-sid";
  const runtimeAbiVersion = 1;
  const FINGERPRINT = "data-hemx-fp";
  const STATE = "data-hemx-st";
  const pending = new WeakMap();
  const queues = new WeakMap();
  const timers = new WeakMap();
  const everyTimers = new WeakMap();
  const revealObservers = new WeakMap();
  const revealed = new WeakSet();
  const pendingClassStates = new WeakMap();
  const indicatorStates = new WeakMap();
  const busyStates = new WeakMap();
  const disabledStates = new WeakMap();
  const sseSources = new WeakMap();
  const atomStores = new WeakMap();
  const dragKeys = new WeakMap();
  let currentOperationId = null;
  const clientHandlers = new Map();
  const clientRuns = new WeakMap();
  const activeRequests = new Set();
  const REQUEST_TIMEOUT_MS = 10_000;

  window.addEventListener("pagehide", () => {
    for (const controller of activeRequests) {
      controller.abort(new DOMException("hemx request cancelled because page is hidden", "AbortError"));
    }
  });

  function roots() {
    const found = [];
    forEachElement(document, (el) => { if (el.hasAttribute(ROOT)) found.push(el); });
    return found;
  }

  function rootOf(node) {
    for (let el = node; el && el !== document; el = el.parentElement) {
      if (el.hasAttribute && el.hasAttribute(ROOT)) return el;
    }
    return null;
  }

  function formOwner(el) {
    if (!el || el.tagName === "FORM") return el;
    if (el.getAttribute && el.getAttribute("form")) return elementById(rootOf(el) || document, el.getAttribute("form"));
    return closestInRoot(el, rootOf(el) || document, (node) => node.tagName === "FORM");
  }

  function elementById(scope, id) {
    if (attrEquals(scope, "id", id)) return scope;
    return firstElement(scope, (el) => attrEquals(el, "id", id));
  }

  function formHandleId(form) {
    if (!form) return null;
    const holder = form.hasAttribute(HID) ? form : firstElement(form, (el) => el.hasAttribute(HID));
    const raw = holder && holder.getAttribute(HID);
    return raw && /^\d+$/.test(raw) ? raw : null;
  }

  function closestInRoot(start, root, predicate) {
    for (let node = start; node && node !== root.parentNode; node = node.parentElement) {
      if (predicate(node)) return node;
      if (node === root) break;
    }
    return null;
  }

  function handleId(el) {
    const raw = el && el.getAttribute(HID);
    return raw && /^\d+$/.test(raw) ? raw : null;
  }

  function normalizedPolicy(value) {
    return value === "latest" || value === "queue" || value === "drop" || value === "parallel" ? value : null;
  }

  function requestPolicy(el, eventName) {
    const policy = normalizedPolicy(el.getAttribute("data-hemx-policy"));
    if (policy) return policy;
    if (el.hasAttribute("data-hemx-debounce") || eventName === "input") return "latest";
    if (el.tagName === "FORM") return "drop";
    return "parallel";
  }

  function showPending(el, on) {
    const klass = el.getAttribute("data-hemx-pending-class");
    if (klass) togglePendingClass(el, klass, on);
    toggleBusy(el, on);
    const root = rootOf(el) || document;
    forEachElement(root, (i) => { if (i.hasAttribute("data-hemx-indicator")) toggleIndicator(i, on); });
    if (el.hasAttribute("data-hemx-disable-while-pending")) {
      const controls = [];
      if (isDisableControl(el)) controls.push(el);
      forEachElement(el, (child) => { if (isDisableControl(child)) controls.push(child); });
      controls.forEach((c) => toggleDisabled(c, on));
    }
  }

  function toggleBusy(el, on) {
    const state = busyStates.get(el);
    if (on) {
      if (state) state.count += 1;
      else busyStates.set(el, { count: 1, value: el.getAttribute("aria-busy") });
      el.setAttribute("aria-busy", "true");
      return;
    }
    if (!state) return;
    state.count -= 1;
    if (state.count <= 0) {
      if (state.value === null) el.removeAttribute("aria-busy");
      else el.setAttribute("aria-busy", state.value);
      busyStates.delete(el);
    }
  }

  function togglePendingClass(el, klass, on) {
    const state = pendingClassStates.get(el);
    if (on) {
      if (state) state.count += 1;
      else pendingClassStates.set(el, { count: 1, className: klass, hadClass: el.classList.contains(klass) });
      el.classList.add(klass);
      return;
    }
    if (!state) return;
    state.count -= 1;
    if (state.count <= 0) {
      if (state.hadClass) el.classList.add(state.className);
      else el.classList.remove(state.className);
      pendingClassStates.delete(el);
    }
  }

  function toggleIndicator(indicator, on) {
    const state = indicatorStates.get(indicator);
    if (on) {
      if (state) state.count += 1;
      else indicatorStates.set(indicator, { count: 1, hidden: indicator.hidden });
      indicator.hidden = false;
      return;
    }
    if (!state) return;
    state.count -= 1;
    if (state.count <= 0) {
      indicator.hidden = state.hidden;
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

  function formDataFor(el, eventName, source = el) {
    const form = formOwner(el);
    const data = form ? new FormData(form) : new FormData();
    if (form && source && source !== form && source.name && !source.disabled) data.append(source.name, source.value);
    const id = handleId(el) || formHandleId(form);
    if (id && !data.has("__h")) data.set("__h", id);
    const dragKey = eventName === "drop" && dragKeys.get(rootOf(el));
    if (dragKey && !data.has("work_id")) data.set("work_id", dragKey);
    for (const { name, value } of Array.from(el.attributes || [])) {
      if (name.startsWith("data-") && !name.startsWith("data-hemx-") && name !== HID && name !== SID) {
        data.set(name.slice(5).replace(/-/g, "_"), value);
      }
    }
    return { form, data, multipart: form && String(form.enctype).toLowerCase() === "multipart/form-data" };
  }

  function urlEncoded(data) {
    const encoded = new URLSearchParams();
    for (const [name, value] of data.entries()) {
      if (typeof File !== "undefined" && value instanceof File) continue;
      encoded.append(name, value);
    }
    return encoded;
  }

  function requestBody(data, multipart) {
    return multipart ? data : urlEncoded(data);
  }

  function requestUrl(form, data, method) {
    const url = new URL((form && form.action) || location.href, location.href);
    if (method === "GET") url.search = urlEncoded(data).toString();
    return url.href;
  }

  function pageRequestUrl(form, source) {
    const url = new URL((form && form.action) || location.href, location.href);
    url.search = urlEncoded(successfulFormData(form, source)).toString();
    return url.href;
  }

  function successfulFormData(form, source) {
    try {
      return source && source !== form ? new FormData(form, source) : new FormData(form);
    } catch (_) {
      return new FormData(form);
    }
  }

  function pageFormHistoryMode(source, form) {
    return historyMode(source, null) || historyMode(form, null) || historyMode(boostRoot(form), null);
  }

  function boostRoot(el) {
    return el && closestInRoot(el.parentElement, rootOf(el), (node) => node.hasAttribute("data-hemx-boost"));
  }

  function historyMode(el, fallback) {
    if (!el) return fallback;
    const value = el.getAttribute("data-hemx-history") || el.getAttribute("data-hemx-nav") || "";
    if (value === "push" || value === "replace" || value === "none") return value;
    if (el.hasAttribute("data-hemx-history") || el.hasAttribute("data-hemx-nav")) return fallback || "push";
    return fallback;
  }

  function httpError(response) {
    const error = new Error(`HTTP ${response.status}`);
    error.status = response.status;
    return error;
  }

  function showError(el, error) {
    const root = rootOf(el) || document;
    const message = error ? (error.status ? `Request failed (${error.status})` : "Request failed") : "";
    forEachElement(root, (outlet) => {
      if (!outlet.hasAttribute("data-hemx-error")) return;
      outlet.textContent = message;
      if (message) outlet.removeAttribute("hidden");
      else outlet.setAttribute("hidden", "");
    });
  }

  async function runClient(el, event) {
    const name = el.getAttribute("data-hemx-client");
    const root = rootOf(el);
    const policy = el.getAttribute("data-hemx-client-policy") || "latest";
    const run = { root, generation: (clientRuns.get(root)?.generation || 0) + 1 };
    if (policy === "drop" && clientRuns.has(root)) return;
    clientRuns.set(root, run);
    const active = () => clientRuns.get(root) === run && root.isConnected;
    const handler = clientHandlers.get(name);
    showError(el, null);
    showPending(el, true);
    try {
      if (!handler) throw new Error(`unknown client-local hemx handler: ${name}`);
      const stateVersion = Number(root.getAttribute("data-hemx-client-state-version") || "1");
      const operationId = crypto.randomUUID();
      const wire = await handler(
        1,
        event.type,
        dragKeys.get(root) || el.getAttribute("data-card-id") || ("value" in el ? String(el.value) : undefined),
        "checked" in el ? Boolean(el.checked) : undefined,
        event.key || undefined,
        stateVersion,
        root.getAttribute(STATE) || "",
      );
      if (!(wire instanceof Uint8Array)) throw new Error(`client-local hemx handler ${name} returned an invalid effect batch`);
      if (!active()) return;
      currentOperationId = operationId;
      try {
        applyBatch(wire, root);
      } finally {
        currentOperationId = null;
      }
      if (name === "reorder_card") {
        const key = dragKeys.get(root) || el.getAttribute("data-card-id");
        const moved = key ? firstElement(root, (node) => node.getAttribute("data-key") === key) : null;
        const focus = moved && (firstElement(moved, (node) => node.tagName === "BUTTON") || moved);
        if (focus && typeof focus.focus === "function") focus.focus();
        if (matchMedia("(prefers-reduced-motion: reduce)").matches) root.setAttribute("data-hemx-reduced-motion", "");
        else root.removeAttribute("data-hemx-reduced-motion");
      }
    } catch (error) {
      if (!active()) return;
      const fallback = el.hasAttribute("data-hemx-client-fallback");
      showError(el, error);
      emit(root, "hemx:client-error", { handler: name, message: String(error), fallback });
      if (fallback) await send(el, event.type, el);
    } finally {
      if (clientRuns.get(root) === run) clientRuns.delete(root);
      if (el.isConnected) showPending(el, false);
    }
  }

  async function send(el, eventName, source = el) {
    if (el.getAttribute("data-hemx-confirm") && !confirm(el.getAttribute("data-hemx-confirm"))) return;
    const { form, data, multipart } = formDataFor(el, eventName, source);
    const target = form || el;
    const policy = requestPolicy(target, eventName);
    const active = pending.get(target);
    if (active && policy === "drop") return;
    if (active && policy === "latest") {
      active.abort.abort();
      showPending(target, false);
    }
    if (active && policy === "queue") {
      const base = queues.get(target) || active.done;
      let queued;
      const next = base.then(() => send(el, eventName, source));
      queued = next.catch(() => {}).finally(() => {
        if (queues.get(target) === queued) queues.delete(target);
      });
      queues.set(target, queued);
      return;
    }

    const abort = new AbortController();
    const timeout = setTimeout(
      () => abort.abort(new DOMException(`hemx request timed out after ${REQUEST_TIMEOUT_MS} ms`, "TimeoutError")),
      REQUEST_TIMEOUT_MS,
    );
    activeRequests.add(abort);
    let finish;
    const done = new Promise((resolve) => { finish = resolve; });
    const method = String((form && form.getAttribute("method")) || "POST").toUpperCase();
    const mode = method === "GET" && form && pageFormHistoryMode(source, form);
    const body = method === "GET" || method === "HEAD" ? undefined : requestBody(data, multipart);
    const headers = { "X-HEMX-Partial": "1", "Accept": "application/hemx, text/html" };
    if (body instanceof URLSearchParams) headers["Content-Type"] = "application/x-www-form-urlencoded;charset=UTF-8";
    pending.set(target, { abort, done });
    showError(target, null);
    showPending(target, true);
    try {
      if (mode) {
        await navigateUrl(pageRequestUrl(form, source), rootOf(form) || rootOf(el), mode);
        return;
      }
      savePage(roots()[0]);
      const response = await fetch(requestUrl(form, data, method), {
        method,
        body,
        headers,
        credentials: "same-origin",
        signal: abort.signal,
      });
      if (pending.get(target)?.abort !== abort && policy === "latest") return;
      if (!response.ok) throw httpError(response);
      await applyResponse(response, rootOf(target));
    } catch (error) {
      if (error.name !== "AbortError") {
        showError(target, error);
        emit(rootOf(target), "hemx:error", { message: String(error), status: error.status || null });
      }
    } finally {
      clearTimeout(timeout);
      activeRequests.delete(abort);
      if (pending.get(target)?.abort === abort) {
        pending.delete(target);
        showPending(target, false);
      } else if (policy === "parallel") {
        showPending(target, false);
      }
      finish();
    }
  }

  async function navigate(anchor, mode = "push") {
    const href = anchor.href;
    const root = rootOf(anchor);
    showPending(anchor, true);
    try {
      await navigateUrl(href, root, mode);
    } finally {
      showPending(anchor, false);
    }
  }

  function snapshotPage(root) {
    return root ? {
      hemx: true,
      pageHtml: root.innerHTML,
      pageTitle: document.title,
      scrollX: window.scrollX,
      scrollY: window.scrollY,
    } : null;
  }

  function savePage(root) {
    const snapshot = snapshotPage(root);
    if (snapshot) history.replaceState(snapshot, "", location.href);
  }

  function restorePage(state, root) {
    if (!root || !state || typeof state.pageHtml !== "string") return false;
    root.innerHTML = state.pageHtml;
    if (state.pageTitle) document.title = state.pageTitle;
    bindRoot(root);
    requestAnimationFrame(() => scrollTo(state.scrollX, state.scrollY));
    return true;
  }

  async function navigateUrl(href, root, mode = "replace") {
    if (mode === "push") savePage(root);
    const response = await fetch(href, {
      headers: { "X-HEMX-Partial": "1", "Accept": "text/html" },
      credentials: "same-origin",
    });
    if (!await applyResponse(response, root)) {
      if (mode === "none") location.reload();
      else location.href = href;
      return;
    }
    if (mode === "push") history.pushState({ hemx: true }, "", href);
    else if (mode === "replace") history.replaceState({ hemx: true }, "", href);
  }

  async function applyResponse(response, root) {
    if (response.redirected) {
      location.href = response.url;
      return false;
    }
    if (!compatibleFingerprint(response, root)) {
      location.reload();
      return false;
    }
    const type = response.headers.get("content-type") || "";
    if (type.includes("text/html")) return applyHtml(await response.text(), root, response.headers.get("x-hemx-title"));
    if (type.includes("application/hemx")) {
      applyBatch(await response.arrayBuffer(), root);
      return true;
    }
    return false;
  }

  function compatibleFingerprint(response, root) {
    const received = response.headers.get("x-hemx-fingerprint");
    const expected = root && root.getAttribute(FINGERPRINT);
    return !received || !expected || received === expected;
  }

  function applyBatch(buffer, root) {
    let batch;
    try {
      batch = decodeBatch(buffer);
    } catch (error) {
      if (error?.code === "HEMX_UNSUPPORTED_ABI") {
        location.reload();
        return;
      }
      throw error;
    }
    const expected = root && root.getAttribute(FINGERPRINT);
    if (expected && String(batch.fingerprint) !== expected) {
      location.reload();
      return;
    }
    const scope = root || document;
    const missingTarget = batch.ops.map((op) => canApplyOp(scope, op)).find(Boolean);
    if (missingTarget) {
      missing(scope, missingTarget);
      return;
    }
    for (const op of batch.ops) applyOp(scope, op);
    bindPolling(scope);
    bindRevealed(scope);
  }

  function canApplyOp(scope, op) {
    if (op.kind === "put" && isAtom(op.target)) return null;
    if (op.kind === "put" || op.kind === "focus") return targetFor(scope, op.target) ? null : op.target;
    if (op.kind === "insert" || op.kind === "prepend") return targetFor(scope, op.target) ? null : op.target;
    if (op.kind === "remove") return (op.key ? keyedTarget(scope, op.target.resource.id, op.key) : targetFor(scope, op.target)) ? null : op.target;
    if (op.kind === "move") return targetFor(scope, op.target) && keyedTarget(scope, op.target.resource.id, op.key) ? null : op.target;
    if (op.kind === "navigate" && op.scroll && op.scroll.kind === "element") return targetFor(scope, op.scroll.target) ? null : op.scroll.target;
    return null;
  }

  function applyOp(scope, op) {
    if (op.kind === "put") {
      if (isAtom(op.target)) {
        atomStore(scope).set(String(op.target.resource.id), op.payload.value);
        forEachElement(scope, (element) => {
          if (element.getAttribute("data-aid") === String(op.target.resource.id)) {
            putPayload(element, op.payload);
          }
        });
        return true;
      }
      const target = targetFor(scope, op.target);
      if (!target) return missing(scope, op.target);
      if (op.target.scope && op.target.scope.kind === "key" && op.payload.kind === "html") replacePayload(target, op.payload, op.target.scope.value, op.target.resource.id);
      else putPayload(target, op.payload);
    } else if (op.kind === "insert" || op.kind === "prepend") {
      const target = targetFor(scope, op.target);
      if (!target) return missing(scope, op.target);
      const nodes = fragmentNodes(op.payload, op.key, op.target.resource.id);
      target[op.kind === "prepend" ? "prepend" : "append"](...nodes);
    } else if (op.kind === "remove") {
      const target = op.key ? keyedTarget(scope, op.target.resource.id, op.key) : targetFor(scope, op.target);
      if (!target) return missing(scope, op.target);
      target.remove();
    } else if (op.kind === "move") {
      const target = targetFor(scope, op.target);
      const item = keyedTarget(scope, op.target.resource.id, op.key);
      if (!target || !item) return missing(scope, op.target);
      const before = op.before && keyedTarget(scope, op.target.resource.id, op.before);
      target.insertBefore(item, before || null);
    } else if (op.kind === "focus") {
      const target = targetFor(scope, op.target);
      if (target && target.focus) target.focus();
      else return missing(scope, op.target);
    } else if (op.kind === "navigate") {
      if (op.mode === "redirect") location.href = op.url;
      else {
        history[op.mode === "replace" ? "replaceState" : "pushState"]({ hemx: true }, "", op.url);
        if (op.scroll === "top") scrollTo(0, 0);
        else if (op.scroll && op.scroll.kind === "element") {
          const target = targetFor(scope, op.scroll.target);
          if (target) target.scrollIntoView();
        }
        if (op.title) document.title = op.title;
      }
    } else if (op.kind === "emit") {
      let payload = op.payload;
      if (currentOperationId && op.name === "hemx:sync-patch") {
        const event = JSON.parse(payload);
        const patch = event && Object.getPrototypeOf(event) === Object.prototype && "patch" in event
          ? event.patch
          : event;
        if (patch.idempotencyKey !== "$hemx-interaction" || patch.operationId !== "$hemx-interaction") {
          throw new Error("hemx sync patch is missing its interaction identity");
        }
        patch.idempotencyKey = currentOperationId;
        patch.operationId = currentOperationId;
        payload = JSON.stringify(event);
      }
      handleRuntimeEvent(scope, op.name, payload);
      emit(scope, op.name, payload);
    }
    return true;
  }

  function targetFor(scope, ref) {
    if (isAtom(ref)) return null;
    if (ref.scope && ref.scope.kind === "key") return keyedTarget(scope, ref.resource.id, ref.scope.value);
    if (ref.scope && ref.scope.kind === "field") return fieldTarget(scope, ref.resource.id, ref.scope.value);
    return generatedTarget(scope, ref.resource.id);
  }

  function firstElement(scope, predicate) {
    const stack = [];
    for (let node = scope && scope.firstElementChild; node; node = node.nextElementSibling) stack.push(node);
    while (stack.length) {
      const node = stack.shift();
      if (predicate(node)) return node;
      for (let child = node.firstElementChild; child; child = child.nextElementSibling) stack.push(child);
    }
    return null;
  }

  function attrEquals(el, name, value) {
    return el && el.getAttribute && el.getAttribute(name) === String(value);
  }

  function generatedResource(el, id) {
    return attrEquals(el, "data-sid", id) || attrEquals(el, "data-slot-id", id);
  }

  function generatedForm(el, id) {
    return attrEquals(el, "data-fid", id) || attrEquals(el, "data-form-id", id);
  }

  function generatedTarget(scope, id) {
    if (scope && generatedResource(scope, id)) return scope;
    return firstElement(scope, (el) => generatedResource(el, id));
  }

  function withinGeneratedResource(el, scope, id) {
    for (let node = el; node && node !== scope.parentNode; node = node.parentElement) {
      if (generatedResource(node, id)) return true;
      if (node === scope) break;
    }
    return false;
  }

  function withinGeneratedForm(el, scope, id) {
    for (let node = el; node && node !== scope.parentNode; node = node.parentElement) {
      if (generatedForm(node, id)) return true;
      if (node === scope) break;
    }
    return false;
  }

  function isInputControl(el) {
    return el && (el.tagName === "INPUT" || el.tagName === "TEXTAREA" || el.tagName === "SELECT");
  }

  function isDisableControl(el) {
    return isInputControl(el) || (el && el.tagName === "BUTTON");
  }

  function isAtom(ref) {
    return ref && ref.resource && ref.resource.kind === "atom";
  }

  function atomStore(root) {
    const owner = root && root.nodeType === 1 ? root : document.documentElement;
    let store = atomStores.get(owner);
    if (!store) {
      store = new Map();
      atomStores.set(owner, store);
    }
    return store;
  }

  function atomValue(root, id) {
    return atomStore(rootOf(root) || root || roots()[0]).get(String(id));
  }

  function keyedTarget(scope, id, key) {
    return firstElement(scope, (el) => attrEquals(el, "data-key", key) && withinGeneratedResource(el, scope, id));
  }

  function fieldTarget(scope, id, field) {
    return firstElement(scope, (el) => attrEquals(el, "name", field) && withinGeneratedForm(el, scope, id));
  }

  function formErrorTarget(scope, id, field) {
    return firstElement(scope, (el) => attrEquals(el, "data-hemx-error-for", field) && withinGeneratedForm(el, scope, id)) ||
      firstElement(scope, (el) => attrEquals(el, "name", field) && withinGeneratedForm(el, scope, id));
  }

  function putFormError(target, message) {
    if (isInputControl(target)) target.setCustomValidity(message);
    else target.textContent = message;
  }

  function putPayload(target, payload) {
    if (payload.kind === "html") target.innerHTML = payload.value;
    else if (isInputControl(target)) target.value = payload.value;
    else target.textContent = payload.value;
  }

  function replacePayload(target, payload, key, resourceId) {
    const nodes = fragmentNodes(payload, key, resourceId);
    if (nodes.length) target.replaceWith(...nodes);
    else target.innerHTML = "";
  }

  function fragmentNodes(payload, key, resourceId) {
    const template = document.createElement("template");
    if (payload.kind === "html") template.innerHTML = payload.value;
    else template.textContent = payload.value;
    const nodes = Array.from(template.content.childNodes);
    const firstElement = nodes.find((node) => node.nodeType === 1);
    if (firstElement && key != null && !firstElement.hasAttribute("data-key")) firstElement.setAttribute("data-key", key);
    if (firstElement && resourceId != null && !firstElement.hasAttribute("data-sid")) firstElement.setAttribute("data-sid", resourceId);
    return nodes;
  }

  function handleRuntimeEvent(scope, name, payload) {
    if (name === "hemx:form-reset") {
      const form = generatedFormTarget(scope, payload);
      if (form && form.reset) {
        form.reset();
        forEachElement(form, (el) => {
          if (el.hasAttribute("data-hemx-error-for")) {
            if (isInputControl(el)) el.setCustomValidity("");
            else el.textContent = "";
          }
        });
      }
    } else if (name === "hemx:form-error") {
      const [id, field, message] = String(payload).split("\u001f");
      const target = formErrorTarget(scope, id, field);
      if (target) putFormError(target, message || "");
    } else if (name === "hemx:form-disable-while-pending") {
      const form = generatedFormTarget(scope, payload);
      if (form) form.setAttribute("data-hemx-disable-while-pending", "");
    }
  }

  function missing(root, target) {
    emit(root, "hemx:missing-target", target);
    return false;
  }

  function cssEscape(value) {
    return String(value).replace(/\\/g, "\\\\").replace(/"/g, "\\\"");
  }

  function applyHtml(html, root, title) {
    const scope = root || document;
    const doc = new DOMParser().parseFromString(html, "text/html");
    const template = firstElement(doc, (el) => el.tagName === "TEMPLATE" && el.hasAttribute("data-hemx"));
    const lowered = replaceLoweredSlots(scope, doc);
    const named = replaceSlot(scope, doc, "content", lowered ? undefined : (template ? template.innerHTML : html));
    if (!lowered && !named) {
      emit(scope, "hemx:missing-content-slot", null);
      return false;
    }
    replaceSlot(scope, doc, "nav");
    const titleEl = firstElement(doc, (el) => el.tagName === "TITLE");
    const nextTitle = title || (titleEl && titleEl.textContent);
    if (nextTitle) document.title = nextTitle;
    return true;
  }

  function replaceLoweredSlots(scope, doc) {
    let changed = false;
    forEachElement(doc.body || doc, (source) => {
      const id = source.getAttribute("data-sid") || source.getAttribute("data-slot-id");
      if (!id) return;
      const target = generatedTarget(scope, id);
      if (target) {
        target.innerHTML = source.innerHTML;
        changed = true;
      }
    });
    return changed;
  }

  function forEachElement(scope, visit) {
    for (let node = scope && scope.firstElementChild; node; node = node.nextElementSibling) {
      visit(node);
      forEachElement(node, visit);
    }
  }

  function generatedFormTarget(scope, id) {
    if (scope && generatedForm(scope, id)) return scope;
    return firstElement(scope, (el) => generatedForm(el, id));
  }

  function namedSlot(el, name) {
    return attrEquals(el, "data-hemx-slot", name) || attrEquals(el, "data-slot", name);
  }

  function replaceSlot(scope, doc, name, fallback) {
    const target = firstElement(scope, (el) => namedSlot(el, name));
    if (!target) return false;
    const source = firstElement(doc, (el) => namedSlot(el, name));
    if (!source && fallback === undefined) return false;
    target.innerHTML = source ? source.innerHTML : fallback;
    return true;
  }

  function emit(root, name, detail) {
    (root || document).dispatchEvent(new CustomEvent(name, { bubbles: true, detail }));
  }

  function defaultEvent(el) {
    if (el.getAttribute("data-hemx-on")) return el.getAttribute("data-hemx-on").trim().split(/\s+/)[0];
    if (el.tagName === "FORM") return "submit";
    return "click";
  }

  function handlesEvent(el, name) {
    const declared = el.getAttribute("data-hemx-on");
    return declared ? declared.trim().split(/\s+/).includes(name) : defaultEvent(el) === name;
  }

  function bindRoot(root) {
    ["click", "submit", "input", "change", "keydown", "dragstart", "dragover", "drop"].forEach((name) => {
      root.addEventListener(name, (event) => {
        if (name === "keydown") {
          const direct = closestInRoot(event.target, root, (el) => el.hasAttribute("data-hemx-client"));
          if (direct && !["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown"].includes(event.key)) return;
        }
        if (name === "dragstart") {
          const item = closestInRoot(event.target, root, (el) => el.hasAttribute("data-key"));
          if (item) {
            dragKeys.set(root, item.getAttribute("data-key"));
            if (event.dataTransfer) event.dataTransfer.setData("text/plain", item.getAttribute("data-key"));
          }
          return;
        }
        if (name === "dragover") {
          const drop = closestInRoot(event.target, root, (el) => el.hasAttribute(HID) && el.getAttribute("data-hemx-on") === "drop");
          if (drop) event.preventDefault();
          return;
        }
        const nav = closestInRoot(event.target, root, (el) =>
          (el.tagName === "A" && el.hasAttribute("data-hemx-nav")) ||
          (el.tagName === "A" && el.hasAttribute("href") && closestInRoot(el.parentElement, root, (node) => node.hasAttribute("data-hemx-boost")))
        );
        if (name === "click" && nav && sameOriginNav(event, nav)) {
          event.preventDefault();
          navigate(nav, historyMode(nav, "push"));
          return;
        }
        if (name === "click") {
          const direct = closestInRoot(event.target, root, (el) => el.hasAttribute(HID));
          const active = direct || closestInRoot(event.target, root, (el) => el.hasAttribute("data-hemx-client"));
          if (active && handlesEvent(active, "click")) {
            event.preventDefault();
            if (active.hasAttribute("data-hemx-client")) {
              runClient(active, event).catch((error) => emit(root, "hemx:client-error", {
                handler: active.getAttribute("data-hemx-client"),
                message: String(error),
                fallback: false,
              }));
            } else {
              schedule(active, name);
            }
            return;
          }
        }
        if (name === "click") {
          const submitter = closestInRoot(event.target, root, (el) =>
            (el.tagName === "BUTTON" && (!el.hasAttribute("type") || el.getAttribute("type") === "submit")) ||
            (el.tagName === "INPUT" && el.getAttribute("type") === "submit")
          );
          const form = formOwner(submitter);
          if (form && root.contains(form) && (formHandleId(form) || pageFormHistoryMode(submitter, form))) {
            if (form.reportValidity && !form.reportValidity()) return;
            event.preventDefault();
            schedule(form, "submit", submitter);
            return;
          }
        }
        let el = closestInRoot(event.target, root, (node) =>
          node.hasAttribute(HID) || node.hasAttribute("data-hemx-client") ||
          (node.tagName === "FORM" && (pageFormHistoryMode(event.submitter || node, node) || boostRoot(node)))
        );
        if (name === "submit" && !el && event.target && event.target.tagName === "FORM" && (formHandleId(event.target) || pageFormHistoryMode(event.submitter || event.target, event.target))) el = event.target;
        if (!el || !handlesEvent(el, name)) return;
        event.preventDefault();
        if (el.hasAttribute("data-hemx-client")) {
          runClient(el, event).catch((error) => emit(root, "hemx:client-error", {
            handler: el.getAttribute("data-hemx-client"),
            message: String(error),
            fallback: false,
          }));
        } else {
          schedule(el, name, event.submitter || el);
        }
      });
    });
    bindPolling(root);
    bindRevealed(root);
  }

  function schedule(el, eventName, source = el) {
    const debounce = duration(el.getAttribute("data-hemx-debounce"));
    const delay = duration(el.getAttribute("data-hemx-delay"));
    const throttle = duration(el.getAttribute("data-hemx-throttle"));
    if (debounce) {
      clearTimeout(timers.get(el));
      timers.set(el, setTimeout(() => send(el, eventName, source), debounce));
    } else if (delay) {
      setTimeout(() => send(el, eventName, source), delay);
    } else if (throttle) {
      if (timers.get(el)) return;
      send(el, eventName, source).finally(() => setTimeout(() => timers.delete(el), throttle));
    } else {
      send(el, eventName, source);
    }
  }

  function bindPolling(root) {
    forEachElement(root, (el) => {
      if ((!el.hasAttribute("data-hemx-every") && !el.hasAttribute("data-hemx-interval")) || everyTimers.has(el)) return;
      const eventName = el.hasAttribute("data-hemx-interval") ? "interval" : "every";
      const ms = duration(el.getAttribute("data-hemx-interval") || el.getAttribute("data-hemx-every"));
      if (!ms) return;
      everyTimers.set(el, setInterval(() => document.contains(el) ? send(el, eventName) : stopPolling(el), ms));
    });
  }

  function stopPolling(el) {
    clearInterval(everyTimers.get(el));
    everyTimers.delete(el);
  }

  function revealedRootMargin(el) {
    const ahead = Number(el.getAttribute("data-hemx-revealed-ahead") || "0");
    const viewports = Number.isFinite(ahead) && ahead >= 0 ? ahead : 0;
    return `0px 0px ${viewports * window.innerHeight}px 0px`;
  }

  function bindRevealed(root) {
    let observers = revealObservers.get(root);
    forEachElement(root, (el) => {
      if (!el.hasAttribute("data-hemx-revealed") || revealed.has(el)) return;
      if (typeof IntersectionObserver === "undefined") {
        revealed.add(el);
        schedule(el, "revealed");
        return;
      }
      if (!observers) {
        observers = new Map();
        revealObservers.set(root, observers);
      }
      const rootMargin = revealedRootMargin(el);
      let observer = observers.get(rootMargin);
      if (!observer) {
        observer = new IntersectionObserver((entries) => {
          entries.forEach((entry) => {
            if (!entry.isIntersecting || revealed.has(entry.target)) return;
            revealed.add(entry.target);
            observer.unobserve(entry.target);
            schedule(entry.target, "revealed");
          });
        }, { rootMargin });
        observers.set(rootMargin, observer);
      }
      observer.observe(el);
    });
  }

  function bindSse(root) {
    const url = root.getAttribute("data-hemx-sse");
    if (!url || sseSources.has(root) || typeof EventSource === "undefined") return;
    const href = new URL(url, location.href);
    if (href.origin !== location.origin) {
      emit(root, "hemx:sse-error", url);
      return;
    }
    const source = new EventSource(href.href);
    source.addEventListener("hemx", (event) => applySseMessage(root, event));
    source.addEventListener("message", (event) => applySseMessage(root, event));
    source.addEventListener("error", () => emit(root, "hemx:sse-error", url));
    sseSources.set(root, source);
  }

  function applySseMessage(root, event) {
    try {
      const bytes = base64UrlBytes(event.data);
      applyBatch(bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength), root);
    } catch (error) {
      emit(root, "hemx:error", String(error));
    }
  }

  function duration(value) {
    if (!value) return 0;
    const match = String(value).trim().match(/^(\d+)(ms|s)?$/);
    if (!match) return 0;
    return Number(match[1]) * (match[2] === "s" ? 1000 : 1);
  }

  function bootstrapState(root) {
    const encoded = root.getAttribute(STATE);
    if (!encoded) return;
    try {
      const store = atomStore(root);
      for (const atom of decodeAtomState(encoded)) store.set(String(atom.id), atom.bytes);
    } catch (error) {
      atomStores.delete(root);
      emit(root, "hemx:state-error", String(error));
    }
  }

  function decodeAtomState(encoded) {
    const bytes = base64UrlBytes(encoded);
    const d = postcardDecoder(bytes);
    const atoms = d.vec(() => ({ id: d.varint(), bytes: d.bytes() }));
    if (!d.done()) throw new Error("trailing hemx state bytes");
    return atoms;
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
  const MAX_WIRE_FIELD_BYTES = 256 * 1024;

  function boundedLength(value, maximum, field) {
    if (!Number.isSafeInteger(value) || value < 0 || value > maximum) {
      throw new Error(`${field} length ${value} exceeds ${maximum}`);
    }
    return value;
  }

  function postcardDecoder(bytes) {
    if (bytes.length > MAX_WIRE_BYTES) throw new Error(`hemx state exceeds ${MAX_WIRE_BYTES} bytes`);
    let offset = 0;
    const need = (len) => {
      boundedLength(len, bytes.length - offset, "hemx state field");
      const end = offset + len;
      if (end > bytes.length) throw new Error("truncated hemx state");
      const slice = bytes.subarray(offset, end);
      offset = end;
      return slice;
    };
    const varint = () => {
      let value = 0;
      for (let index = 0; index < 5; index += 1) {
        const byte = need(1)[0];
        if (index === 4 && byte > 0x0f) throw new Error("oversized hemx state varint");
        value += (byte & 0x7f) * (2 ** (index * 7));
        if ((byte & 0x80) === 0) return value >>> 0;
      }
      throw new Error("oversized hemx state varint");
    };
    const bytesField = () => need(boundedLength(varint(), MAX_WIRE_FIELD_BYTES, "hemx state bytes"));
    const vec = (read) => {
      const length = boundedLength(varint(), MAX_WIRE_ITEMS, "hemx state vector");
      const values = [];
      for (let index = 0; index < length; index += 1) values.push(read());
      return values;
    };
    return { varint, bytes: bytesField, vec, done: () => offset === bytes.length };
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
    const str = () => new TextDecoder("utf-8", { fatal: true }).decode(
      need(boundedLength(u32(), MAX_WIRE_FIELD_BYTES, "hemx string")),
    );
    const enumValue = (values, field) => {
      const discriminant = u8();
      if (discriminant >= values.length) throw new Error(`unknown ${field} ${discriminant}`);
      return values[discriminant];
    };
    const option = (read) => {
      const discriminant = u8();
      if (discriminant === 0) return null;
      if (discriminant === 1) return read();
      throw new Error(`unknown hemx option ${discriminant}`);
    };
    const resource = () => ({ kind: enumValue(["slot", "atom", "handle", "form"], "hemx resource"), id: u32() });
    const scope = () => {
      const kind = u8();
      if (kind === 0) return null;
      if (kind === 1) return { kind: "key", value: str() };
      if (kind === 2) return { kind: "field", value: str() };
      throw new Error(`unknown hemx scope ${kind}`);
    };
    const ref = () => ({ resource: resource(), scope: scope() });
    const payload = () => ({ kind: enumValue(["text", "html"], "hemx payload"), value: str() });
    const scroll = () => {
      const kind = u8();
      if (kind === 0) return "preserve";
      if (kind === 1) return "top";
      if (kind === 2) return { kind: "element", target: ref() };
      throw new Error(`unknown hemx scroll behavior ${kind}`);
    };
    const effect = () => {
      const kind = u8();
      if (kind === 0) return { kind: "put", target: ref(), payload: payload() };
      if (kind === 1) return { kind: "insert", target: ref(), key: str(), payload: payload() };
      if (kind === 2) return { kind: "prepend", target: ref(), key: str(), payload: payload() };
      if (kind === 3) return { kind: "remove", target: ref(), key: option(str) };
      if (kind === 4) return { kind: "move", target: ref(), key: str(), before: option(str) };
      if (kind === 5) return { kind: "focus", target: ref() };
      if (kind === 6) return { kind: "navigate", url: str(), mode: enumValue(["push", "replace", "redirect"], "hemx navigation mode"), scroll: scroll(), title: option(str) };
      if (kind === 7) return { kind: "emit", name: str(), payload: str() };
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
    if (abiVersion !== runtimeAbiVersion) {
      const error = new Error(`unsupported hemx batch ABI version ${abiVersion}; expected ${runtimeAbiVersion}`);
      error.code = "HEMX_UNSUPPORTED_ABI";
      throw error;
    }
    const batch = { abiVersion, fingerprint: d.u64(), ops: d.vec(d.effect) };
    if (!d.done()) throw new Error("trailing hemx batch bytes");
    return batch;
  }

  function sameOriginNav(event, anchor) {
    return !event.defaultPrevented && event.button === 0 && !event.metaKey && !event.ctrlKey && !event.shiftKey && !event.altKey &&
      anchor.origin === location.origin && !anchor.download && anchor.target !== "_blank";
  }

  function descendantRoots(node) {
    const roots = [];
    for (const child of node.children || []) {
      if (child.hasAttribute(ROOT)) roots.push(child);
      roots.push(...descendantRoots(child));
    }
    return roots;
  }

  function stopDescendantPolling(node) {
    for (const child of node.children || []) {
      if (child.hasAttribute("data-hemx-every") || child.hasAttribute("data-hemx-interval")) {
        stopPolling(child);
      }
      stopDescendantPolling(child);
    }
  }

  function cleanupRemovedRoot(root) {
    clientRuns.delete(root);
    const source = sseSources.get(root);
    if (source) source.close();
    sseSources.delete(root);
    const observers = revealObservers.get(root);
    if (observers) observers.forEach((observer) => observer.disconnect());
    revealObservers.delete(root);
    stopDescendantPolling(root);
  }

  function rebindRevealed(resetDispatched) {
    roots().forEach((root) => {
      const observers = revealObservers.get(root);
      if (observers) observers.forEach((observer) => observer.disconnect());
      revealObservers.delete(root);
      if (resetDispatched) {
        forEachElement(root, (el) => {
          if (attrEquals(el, "data-hemx-on", "revealed") || el.hasAttribute("data-hemx-revealed")) revealed.delete(el);
        });
      }
      bindRevealed(root);
    });
  }

  function restoreRevealed(event) {
    if (event.persisted) rebindRevealed(true);
  }

  function start() {
    roots().forEach((root) => {
      root.setAttribute("data-hemx-request-timeout-ms", String(REQUEST_TIMEOUT_MS));
      try {
        bootstrapState(root);
      } catch (error) {
        emit(root, "hemx:state-error", String(error));
      }
      try {
        bindRoot(root);
      } catch (error) {
        emit(root, "hemx:bind-error", String(error));
      }
      try {
        bindSse(root);
      } catch (error) {
        emit(root, "hemx:sse-error", String(error));
      }
    });
    new MutationObserver((records) => {
      records.forEach((record) => {
        record.removedNodes.forEach((node) => {
          if (!(node instanceof Element)) return;
          if (node.hasAttribute(ROOT)) cleanupRemovedRoot(node);
          descendantRoots(node).forEach(cleanupRemovedRoot);
        });
        record.addedNodes.forEach((node) => {
          if (!(node instanceof Element)) return;
          if (node.hasAttribute(ROOT)) bindRoot(node);
          descendantRoots(node).forEach(bindRoot);
          const owner = rootOf(node.parentElement);
          if (owner) {
            bindPolling(owner);
            bindRevealed(owner);
          }
        });
      });
    }).observe(document.documentElement, { childList: true, subtree: true });
    try {
      history.replaceState(history.state || { hemx: true }, "", location.href);
    } catch (error) {
      const root = roots()[0];
      if (root) emit(root, "hemx:history-error", String(error));
    }
  }

  window.addEventListener("pageshow", restoreRevealed);
  window.addEventListener("resize", () => rebindRevealed(false));

  addEventListener("popstate", (event) => {
    const root = roots()[0];
    if (root && !restorePage(event.state, root)) {
      if (root) navigateUrl(location.href, root, "none").catch((error) => emit(root, "hemx:error", String(error)));
    }
  });

  window.hemx = Object.freeze({
    runtimeAbiVersion,
    roots,
    rootOf,
    applyHtml,
    applyBatch,
    decodeBatch,
    atomValue,
    decodeAtomState,
    registerClientHandler(name, handler) {
      if (!name || typeof handler !== "function") throw new Error("client handler registration requires a name and function");
      const previous = clientHandlers.get(name);
      clientHandlers.set(name, handler);
      return previous;
    },
  });

  if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", start);
  else start();
})();
