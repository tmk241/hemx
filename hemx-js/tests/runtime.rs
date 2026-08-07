#[test]
fn runtime_abi_version_is_explicit_and_stable() {
    assert_eq!(hemx_js::RUNTIME_ABI_VERSION, 1); // test
}

#[test]
fn runtime_exposes_debug_api_before_startup_side_effects() {
    let source = hemx_js::RUNTIME_JS;

    let api = source
        .find("window.hemx = Object.freeze")
        .expect("runtime exposes browser API");
    let start = source
        .find("if (document.readyState === \"loading\") document.addEventListener(\"DOMContentLoaded\", start)")
        .expect("runtime starts after declaration");
    assert!(api < start);
    assert!(source.contains("try {\n        bootstrapState(root);"));
    assert!(source.contains("emit(root, \"hemx:state-error\", String(error));"));
    assert!(source.contains("try {\n        bindRoot(root);"));
    assert!(source.contains("emit(root, \"hemx:bind-error\", String(error));"));
}

#[test]
fn runtime_reports_http_failures_without_applying_effects() {
    let source = hemx_js::RUNTIME_JS;

    assert!(source.contains("function httpError(response)"));
    assert!(source.contains("if (!response.ok) throw httpError(response)"));
    assert!(source.contains("showError(target, error)"));
    assert!(source.contains("showError(target, null)"));
    assert!(source.contains("Request failed (${error.status})"));
    assert!(source.contains("error.status = response.status"));
    assert!(source.contains(
        "emit(rootOf(target), \"hemx:error\", { message: String(error), status: error.status || null })"
    ));
}

#[test]
fn runtime_posts_urlencoded_forms_by_default() {
    let source = hemx_js::RUNTIME_JS;

    assert!(source.contains("new URLSearchParams()"));
    assert!(source.contains("form.getAttribute(\"method\")"));
    assert!(source.contains("multipart/form-data"));
    assert!(source.contains("const body = method === \"GET\" || method === \"HEAD\" ? undefined : requestBody(data, multipart)"));
    assert!(source.contains("application/x-www-form-urlencoded;charset=UTF-8"));
}

#[test]
fn runtime_turns_page_get_forms_into_url_state_navigation() {
    let source = hemx_js::RUNTIME_JS;

    assert!(source.contains("function pageFormHistoryMode(source, form)"));
    assert!(source
        .contains("const mode = method === \"GET\" && form && pageFormHistoryMode(source, form)"));
    assert!(source.contains("if (mode)"));
    assert!(source.contains(
        "await navigateUrl(pageRequestUrl(form, source), rootOf(form) || rootOf(el), mode)"
    ));
    assert!(source.contains("function successfulFormData(form, source)"));
    assert!(source.contains("new FormData(form, source)"));
    assert!(source.contains("data-hemx-nav"));
    assert!(source.contains("data-hemx-history"));
}

#[test]
fn runtime_preserves_multipart_file_upload_fallback_shape() {
    let source = hemx_js::RUNTIME_JS;

    assert!(source.contains(
        "multipart: form && String(form.enctype).toLowerCase() === \"multipart/form-data\""
    ));
    assert!(source.contains("if (typeof File !== \"undefined\" && value instanceof File) continue"));
    assert!(source.contains("return multipart ? data : urlEncoded(data)"));
    assert!(source.contains("if (body instanceof URLSearchParams) headers[\"Content-Type\"] = \"application/x-www-form-urlencoded;charset=UTF-8\""));
}

#[test]
fn runtime_uses_root_scoped_walks_not_dom_selector_apis() {
    let source = hemx_js::RUNTIME_JS;

    assert!(source.contains("function firstElement(scope, predicate)"));
    assert!(source.contains("function closestInRoot(start, root, predicate)"));
    assert!(source.contains("root.addEventListener(name, (event) =>"));
    assert!(!source.contains("querySelector"));
    assert!(!source.contains("querySelectorAll"));
    assert!(!source.contains(".closest("));
    assert!(!source.contains(".matches("));
    assert!(!source.contains("getElementsBy"));
    assert!(!source.contains("document.getElementById"));
}

#[test]
fn runtime_targets_generated_resources_not_response_selectors() {
    let source = hemx_js::RUNTIME_JS;

    assert!(source.contains("function targetFor(scope, ref)"));
    assert!(source.contains("return generatedTarget(scope, ref.resource.id)"));
    assert!(source.contains("function firstElement(scope, predicate)"));
    assert!(source.contains("function generatedResource(el, id)"));
    assert!(source.contains("return firstElement(scope, (el) => attrEquals(el, \"data-key\", key) && withinGeneratedResource(el, scope, id))"));
    assert!(
        source.contains("const nodes = fragmentNodes(op.payload, op.key, op.target.resource.id)")
    );
    assert!(source.contains("if (op.target.scope && op.target.scope.kind === \"key\" && op.payload.kind === \"html\") replacePayload(target, op.payload, op.target.scope.value, op.target.resource.id)"));
    assert!(source.contains("function replacePayload(target, payload, key, resourceId)"));
    assert!(source.contains("target.replaceWith(...nodes)"));
    assert!(source.contains("firstElement.setAttribute(\"data-sid\", resourceId)"));
    assert!(source.contains("const target = generatedTarget(scope, id)"));
    assert!(source.contains("if (!target) return missing(scope, op.target)"));
    assert!(!source.contains("data-hemx-target"));
    assert!(!source.contains("data-hemx-select"));
}

#[test]
fn runtime_interval_dispatch_avoids_duplicate_timers() {
    let source = hemx_js::RUNTIME_JS;

    assert!(source.contains("const everyTimers = new WeakMap()"));
    assert!(source.contains("data-hemx-interval"));
    assert!(source.contains(
        "eventName = el.hasAttribute(\"data-hemx-interval\") ? \"interval\" : \"every\""
    ));
    assert!(source.contains(
        "setInterval(() => document.contains(el) ? send(el, eventName) : stopPolling(el), ms)"
    ));
    assert!(source.contains("function stopPolling(el)"));
    assert!(source.contains("clearInterval(everyTimers.get(el))"));
    assert!(source.contains("everyTimers.delete(el)"));
}

#[test]
fn runtime_supports_tiny_delay_and_revealed_scheduling() {
    let source = hemx_js::RUNTIME_JS;

    assert!(source.contains("data-hemx-delay"));
    assert!(source.contains("setTimeout(() => send(el, eventName, source), delay)"));
    assert!(source.contains("const revealObservers = new WeakMap()"));
    assert!(source.contains("const revealed = new WeakSet()"));
    assert!(source.contains("data-hemx-revealed"));
    assert!(source.contains("data-hemx-revealed-ahead")); // test
    assert!(source.contains("typeof IntersectionObserver === \"undefined\""));
    assert!(source.contains("rootMargin"));
    assert!(source.contains("addEventListener(\"resize\", () => rebindRevealed(false))"));
    assert!(source.contains("schedule(entry.target, \"revealed\")"));
    assert!(source.contains("window.addEventListener(\"pageshow\", restoreRevealed)"));
    assert!(source.contains("revealed.delete(el)")); // test
    assert!(source.contains("record.addedNodes.forEach((node) =>"));
    assert!(source.contains("descendantRoots(node).forEach(bindRoot)"));
    assert!(source.contains("const owner = rootOf(node.parentElement)"));
    assert!(source.contains("bindPolling(owner)"));
    assert!(source.contains("bindRevealed(owner)"));
    assert!(source.contains(
        "for (const op of batch.ops) applyOp(scope, op);\n    bindPolling(scope);\n    bindRevealed(scope);"
    )); // test
}

#[test]
fn runtime_toggles_pending_conventions_around_requests() {
    let source = hemx_js::RUNTIME_JS;

    assert!(source.contains("function showPending(el, on)"));
    assert!(source.contains("el.getAttribute(\"data-hemx-pending-class\")"));
    assert!(source.contains("const pendingClassStates = new WeakMap()"));
    assert!(source.contains("function togglePendingClass(el, klass, on)"));
    assert!(source.contains("hadClass: el.classList.contains(klass)"));
    assert!(source.contains("if (state.hadClass) el.classList.add(state.className)"));
    assert!(source.contains("const indicatorStates = new WeakMap()"));
    assert!(source.contains("const busyStates = new WeakMap()"));
    assert!(source.contains("function toggleBusy(el, on)"));
    assert!(source.contains("el.setAttribute(\"aria-busy\", \"true\")"));
    assert!(source.contains("el.removeAttribute(\"aria-busy\")"));
    assert!(source.contains("toggleBusy(el, on)"));
    assert!(source.contains("function toggleIndicator(indicator, on)"));
    assert!(source.contains("data-hemx-error"));
    assert!(source.contains("function showError(el, error)"));
    assert!(source.contains("toggleIndicator(i, on)"));
    assert!(source.contains("indicator.hidden = state.hidden"));
    assert!(source.contains("el.hasAttribute(\"data-hemx-disable-while-pending\")"));
    assert!(source.contains("if (isDisableControl(el)) controls.push(el)"));
    assert!(source.contains(
        "forEachElement(el, (child) => { if (isDisableControl(child)) controls.push(child); })"
    ));
    assert!(source.contains("const disabledStates = new WeakMap()"));
    assert!(source.contains("function toggleDisabled(control, on)"));
    assert!(source
        .contains("else disabledStates.set(control, { count: 1, disabled: control.disabled })"));
    assert!(source.contains("control.disabled = state.disabled"));
    assert!(source.contains("controls.forEach((c) => toggleDisabled(c, on))"));
    assert!(source.contains("showPending(target, true)"));
    assert!(source.contains("showPending(target, false)"));
}

#[test]
fn runtime_confirms_before_handler_dispatch() {
    let source = hemx_js::RUNTIME_JS;

    assert!(source.contains("el.getAttribute(\"data-hemx-confirm\") && !confirm(el.getAttribute(\"data-hemx-confirm\"))"));
    assert!(source.contains("return;"));
}

#[test]
fn runtime_bounds_and_cancels_ordinary_requests() {
    let source = hemx_js::RUNTIME_JS;

    assert!(source.contains("const REQUEST_TIMEOUT_MS = 10_000"));
    assert!(source.contains("hemx request timed out after ${REQUEST_TIMEOUT_MS} ms"));
    assert!(source.contains("window.addEventListener(\"pagehide\""));
    assert!(source.contains("hemx request cancelled because page is hidden"));
    assert!(source.contains("activeRequests.add(abort)"));
    assert!(source.contains("activeRequests.delete(abort)"));
    assert!(source.contains("data-hemx-request-timeout-ms"));
}

#[test]
fn runtime_fetches_with_same_origin_credentials() {
    let source = hemx_js::RUNTIME_JS;

    assert_eq!(source.matches("credentials: \"same-origin\"").count(), 2);
}

#[test]
fn runtime_handles_get_forms_without_request_body() {
    let source = hemx_js::RUNTIME_JS;

    assert!(source.contains("function requestUrl(form, data, method)"));
    assert!(source.contains("method === \"GET\" || method === \"HEAD\" ? undefined : requestBody"));
    assert!(source.contains("if (method === \"GET\") url.search = urlEncoded(data).toString()"));
}

#[test]
fn runtime_clicking_submitter_schedules_form_submit() {
    let source = hemx_js::RUNTIME_JS;

    assert!(source.contains("function formOwner(el)"));
    assert!(source.contains("function formHandleId(form)"));
    assert!(source.contains("function elementById(scope, id)"));
    assert!(
        source.contains("return elementById(rootOf(el) || document, el.getAttribute(\"form\"))")
    );
    assert!(!source.contains("document.getElementById(el.getAttribute(\"form\"))"));
    assert!(source.contains(
        "const direct = closestInRoot(event.target, root, (el) => el.hasAttribute(HID))"
    ));
    assert!(source.contains("const submitter = closestInRoot(event.target, root, (el) =>"));
    assert!(source.contains("el.tagName === \"BUTTON\" && (!el.hasAttribute(\"type\") || el.getAttribute(\"type\") === \"submit\")"));
    assert!(source.contains("form.reportValidity && !form.reportValidity()"));
    assert!(source.contains("schedule(form, \"submit\", submitter)"));
    assert!(source.contains("function formDataFor(el, eventName, source = el)"));
    assert!(source.contains("data.append(source.name, source.value)"));
    assert!(source.contains("schedule(el, name, event.submitter || el)"));
}

#[test]
fn runtime_supports_drag_drop_params_without_user_js() {
    let source = hemx_js::RUNTIME_JS;

    assert!(source.contains("const dragKeys = new WeakMap()"));
    assert!(source.contains("dragstart"));
    assert!(source.contains("dragover"));
    assert!(source.contains("eventName === \"drop\""));
    assert!(source.contains("data.set(\"work_id\", dragKey)"));
}

#[test]
fn runtime_supports_queued_request_policy() {
    let source = hemx_js::RUNTIME_JS;

    assert!(source.contains("function normalizedPolicy(value)"));
    assert!(source.contains("value === \"latest\" || value === \"queue\" || value === \"drop\" || value === \"parallel\""));
    assert!(
        source.contains("const policy = normalizedPolicy(el.getAttribute(\"data-hemx-policy\"))")
    );
    assert!(source.contains("const queues = new WeakMap()"));
    assert!(source.contains("policy === \"queue\""));
    assert!(source.contains("const base = queues.get(target) || active.done"));
}

#[test]
fn runtime_latest_request_policy_releases_superseded_pending_state() {
    let source = hemx_js::RUNTIME_JS;

    assert!(source.contains("if (active && policy === \"latest\") {\n      active.abort.abort();\n      showPending(target, false);\n    }"));
    assert!(source.contains("if (pending.get(target)?.abort === abort)"));
}

#[test]
fn runtime_parallel_request_policy_releases_each_pending_state() {
    let source = hemx_js::RUNTIME_JS;

    assert!(source.contains(
        "else if (policy === \"parallel\") {\n        showPending(target, false);\n      }"
    ));
    assert!(source.contains("const pendingClassStates = new WeakMap()"));
    assert!(source.contains("const indicatorStates = new WeakMap()"));
    assert!(source.contains("const disabledStates = new WeakMap()"));
}

#[test]
fn runtime_exposes_page_swap_hooks() {
    let source = hemx_js::RUNTIME_JS;

    assert!(source.contains("data-hemx-nav"));
    assert!(source.contains("data-hemx-boost"));
    assert!(source.contains("history.pushState"));
    assert!(source.contains("snapshotPage(root)"));
    assert!(source.contains("restorePage(event.state, root)"));
    assert!(source.contains("scrollTo(state.scrollX, state.scrollY)")); // test
    assert!(source.contains("popstate"));
    assert!(source.contains("x-hemx-title"));
    assert!(source.contains("x-hemx-fingerprint"));
    assert!(source.contains("hemx:missing-content-slot"));
    assert!(source.contains("else location.href = href"));
}

#[test]
fn runtime_popstate_failed_partials_reload_instead_of_stale_ui() {
    let source = hemx_js::RUNTIME_JS;

    assert!(source.contains("if (mode === \"none\") location.reload();"));
    assert!(source.contains("if (root) navigateUrl(location.href, root, \"none\")"));
}

#[test]
fn runtime_preserves_native_navigation_escape_hatches() {
    let source = hemx_js::RUNTIME_JS;

    assert!(source.contains("function sameOriginNav(event, anchor)"));
    assert!(source.contains("!event.defaultPrevented"));
    assert!(source.contains("event.button === 0"));
    assert!(source.contains("!event.metaKey && !event.ctrlKey && !event.shiftKey && !event.altKey"));
    assert!(source.contains("anchor.origin === location.origin"));
    assert!(source.contains("!anchor.download"));
    assert!(source.contains("anchor.target !== \"_blank\""));
}

#[test]
fn runtime_refuses_partial_updates_on_fingerprint_mismatch() {
    let source = hemx_js::RUNTIME_JS;

    assert!(source.contains("function compatibleFingerprint(response, root)"));
    assert!(source.contains("const received = response.headers.get(\"x-hemx-fingerprint\")"));
    assert!(source.contains("const expected = root && root.getAttribute(FINGERPRINT)"));
    assert!(source.contains("if (!compatibleFingerprint(response, root))"));
    assert!(source.contains("location.reload()"));
    assert!(source.contains("if (expected && String(batch.fingerprint) !== expected)"));
}

#[test]
fn runtime_malformed_bootstrap_state_reports_and_continues() {
    let source = hemx_js::RUNTIME_JS;

    assert!(source.contains("function bootstrapState(root)"));
    assert!(source.contains("try {\n      const store = atomStore(root);"));
    assert!(source.contains("atomStores.delete(root)"));
    assert!(source.contains("emit(root, \"hemx:state-error\", String(error))"));
    assert!(source.contains("bindRoot(root)"));
    assert!(source.contains("bindSse(root)"));
}

#[test]
fn runtime_applies_sse_effect_batches_inside_roots() {
    let source = hemx_js::RUNTIME_JS;

    assert!(source.contains("const sseSources = new WeakMap()"));
    assert!(source.contains("const url = root.getAttribute(\"data-hemx-sse\")"));
    assert!(source.contains("const href = new URL(url, location.href)"));
    assert!(source.contains("if (href.origin !== location.origin)"));
    assert!(source.contains("emit(root, \"hemx:sse-error\", url)"));
    assert!(source.contains("new EventSource(href.href)"));
    assert!(source
        .contains("source.addEventListener(\"hemx\", (event) => applySseMessage(root, event))"));
    assert!(source
        .contains("source.addEventListener(\"message\", (event) => applySseMessage(root, event))"));
    assert!(source.contains("applyBatch(bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength), root)"));
    assert!(source.contains("emit(root, \"hemx:sse-error\", url)"));
}

#[test]
fn runtime_page_swaps_lowered_slot_ids() {
    let source = hemx_js::RUNTIME_JS;

    assert!(source.contains("function replaceLoweredSlots(scope, doc)"));
    assert!(source.contains("forEachElement(doc.body || doc, (source)"));
    assert!(source.contains("const target = generatedTarget(scope, id)"));
    assert!(source.contains("target.innerHTML = source.innerHTML"));
}

#[test]
fn runtime_keeps_form_field_targets_separate_from_error_targets() {
    let source = hemx_js::RUNTIME_JS;

    assert!(source.contains("function fieldTarget(scope, id, field)"));
    assert!(
        source.contains("attrEquals(el, \"name\", field) && withinGeneratedForm(el, scope, id)")
    );
    assert!(source.contains("function formErrorTarget(scope, id, field)"));
    assert!(source.contains(
        "attrEquals(el, \"data-hemx-error-for\", field) && withinGeneratedForm(el, scope, id)"
    ));
}

#[test]
fn runtime_preflights_batches_before_applying_ops() {
    let source = hemx_js::RUNTIME_JS;

    assert!(source.contains(
        "const missingTarget = batch.ops.map((op) => canApplyOp(scope, op)).find(Boolean)"
    ));
    assert!(source.contains("function canApplyOp(scope, op)"));
    assert!(source.contains("for (const op of batch.ops) applyOp(scope, op)"));
}

#[test]
fn runtime_ships_typescript_definitions() {
    let source = hemx_js::RUNTIME_D_TS;

    assert!(source.contains("export interface EffectBatch"));
    assert!(source.contains("fingerprint: bigint"));
    assert!(source.contains("export type Effect ="));
    assert!(source.contains("decodeBatch(buffer: ArrayBuffer): EffectBatch"));
    assert!(source.contains("interface Window"));
}

#[test]
fn runtime_clears_error_for_elements_on_form_reset() {
    let source = hemx_js::RUNTIME_JS;

    assert!(source.contains(r#"if (name === "hemx:form-reset")"#));
    assert!(source.contains("form.reset();"));
    assert!(source.contains(r#"if (el.hasAttribute("data-hemx-error-for"))"#));
    assert!(source.contains("forEachElement(form, (el) => {"));
    assert!(source.contains("el.setCustomValidity(\"\")"));
    assert!(source.contains("el.textContent = \"\""));
}
