// Native submission owns validation, successful controls and submit cancellation.
// This adapter only admits an already constructed, owned navigation as an action.
function createForms(dom, ownerOf, accepts, admit) {
  const submitted = new WeakMap();
  const prepared = new WeakMap();
  const nativeForm = (form, name) =>
    Object.getOwnPropertyDescriptor(HTMLFormElement.prototype, name).get.call(form);
  const isSubmitter = (node) =>
    (node instanceof HTMLButtonElement && node.type === "submit") ||
    (node instanceof HTMLInputElement && (node.type === "submit" || node.type === "image"));

  function submission(form, source) {
    if (!form) return { action: location.href, method: "POST", enctype: "application/x-www-form-urlencoded", target: "" };
    const value = (name, property) => isSubmitter(source) && dom.has(source, `form${name}`)
      ? source[property] : nativeForm(form, name);
    return {
      action: value("action", "formAction"),
      method: value("method", "formMethod").toUpperCase(),
      enctype: value("enctype", "formEnctype"),
      target: value("target", "formTarget"),
    };
  }

  function clear(form) {
    const record = prepared.get(form);
    if (!record) return;
    record.observer.disconnect();
    clearTimeout(record.expiry);
    prepared.delete(form);
  }

  function bind() {
    // Test the actual API surface used, not an unrelated precommit capability.
    if (!window.navigation || typeof NavigateEvent === "undefined" ||
        !("sourceElement" in NavigateEvent.prototype) || !("formData" in NavigateEvent.prototype)) return;

    dom.on(document, "submit", (event) => {
      const form = event.target;
      if (!(form instanceof HTMLFormElement) || !event.isTrusted) return;
      clear(form);
      submitted.delete(form);
      const owner = ownerOf(form);
      if (!owner) return;
      submitted.set(form, { event, source: event.submitter || form, owner });
    }, true);

    dom.on(document, "formdata", (event) => {
      const form = event.target;
      const pending = submitted.get(form);
      // FormData hooks can run during dispatch. Keep the submit record until
      // navigation; only that native boundary establishes final cancellation.
      if (!pending || pending.event.defaultPrevented || ownerOf(form) !== pending.owner) return;
      clear(form);
      const source = pending.source;
      if (source !== form && source.form !== form) return;
      if (!accepts(form, source)) return;
      const options = submission(form, source);
      if (!['GET', 'POST'].includes(options.method) ||
          (options.target && options.target !== '_self') ||
          !['application/x-www-form-urlencoded', 'multipart/form-data'].includes(options.enctype)) return;
      const action = new URL(options.action, location.href);
      if (action.origin !== location.origin || !['http:', 'https:'].includes(action.protocol)) return;

      // Navigation may be queued. If its routing/dispatch metadata changes in
      // between, leave the exact native submission alone instead of guessing.
      const record = { source, owner: pending.owner, options, changed: false };
      record.observer = new MutationObserver(() => { record.changed = true; });
      record.observer.observe(form, { attributes: true });
      if (source !== form) record.observer.observe(source, { attributes: true });
      // A disconnected form or canceled planned navigation may never navigate.
      // Bound this transient observation; it is not an application-state store.
      record.expiry = setTimeout(() => { clear(form); submitted.delete(form); }, 10000);
      prepared.set(form, record);
    }, true);

    dom.on(window.navigation, "navigate", (event) => {
      const source = event.sourceElement;
      const form = source instanceof HTMLFormElement ? source : isSubmitter(source) ? source.form : null;
      if (!form) return;
      const pending = submitted.get(form);
      submitted.delete(form);
      const record = prepared.get(form);
      if (!record) return;
      const changed = record.changed || record.observer.takeRecords().length !== 0;
      clear(form);
      if (!pending || pending.event.defaultPrevented || changed || event.defaultPrevented || !event.cancelable || !event.canIntercept ||
          event.hashChange || event.downloadRequest !== null ||
          !dom.isConnected(form) || ownerOf(form) !== record.owner ||
          (record.source !== form && record.source.form !== form)) return;
      const destination = new URL(event.destination.url);
      const action = new URL(record.options.action, location.href);
      if (destination.origin !== location.origin || destination.origin !== action.origin ||
          destination.pathname !== action.pathname || destination.hash !== action.hash) return;
      const method = event.formData === null ? "GET" : "POST";
      if (method !== record.options.method || (method === "POST" && destination.search !== action.search)) return;

      // The browser has already serialized exactly once, including formdata
      // hooks and image coordinates. Copy entries, never serialize the DOM again.
      const data = new FormData();
      const entries = method === "GET" ? destination.searchParams : event.formData;
      for (const [name, value] of entries) data.append(name, value);
      const capture = { form, data, submission: { ...record.options, action: destination.href } };
      event.preventDefault();
      admit(form, record.source, capture);
    });
  }

  return { bind, submission, isSubmitter };
}
