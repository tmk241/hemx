// Observations only: Rust owns the expected boundary outcomes.
async function observeWireCases({ observe, root, apply, cases, waitFor, pause }) {
  const boundary = cases.wireBoundary;
  if (!boundary) throw new Error("missing canonical wire boundary cases");
  // `apply` is the parent helper for named fixtures; these compact boundary
  // fixtures call the same public runtime entry point with producer bytes.
  function unpack(w, packed) {
    const bytes = new w.Uint8Array(packed.length);
    let offset = 0;
    for (const [byte, count] of packed.runs) {
      if (count <= 0 || offset + count > bytes.length) throw new Error("invalid packed wire run");
      bytes.fill(byte, offset, offset + count);
      offset += count;
    }
    if (offset !== bytes.length) throw new Error("incomplete packed wire");
    return bytes;
  }

  const content = '<section data-hemx-resource="1">before<input value="server"></section>';
  function watch(w, d) {
    const owner = d.querySelector("[data-hemx-root]");
    const target = d.querySelector("section");
    const input = target.querySelector("input");
    input.value = "unsent edit";
    input.focus();
    input.setSelectionRange(2, 5, "backward");
    const original = target.innerHTML;
    const diagnostics = [];
    const events = [];
    owner.addEventListener("hemx:error", e => diagnostics.push({
      message: e.detail?.message || String(e.detail),
      code: e.detail?.code ?? null,
      status: e.detail?.status ?? null,
      rootScoped: e.target === owner
    }));
    owner.addEventListener("w", e => events.push(e.detail.byteLength));
    let deliveredMutations = 0;
    const observer = new w.MutationObserver(records => { deliveredMutations += records.length; });
    observer.observe(target, { subtree: true, childList: true, characterData: true, attributes: true });
    return {
      owner,
      finish() {
        const mutations = deliveredMutations + observer.takeRecords().length;
        observer.disconnect();
        return {
          textLength: target.textContent.length,
          textPrefix: target.textContent.slice(0, 16),
          unchanged: target.innerHTML === original && target.querySelector("input") === input &&
            input.value === "unsent edit" && d.activeElement === input &&
            input.selectionStart === 2 && input.selectionEnd === 5 && input.selectionDirection === "backward",
          mutations,
          dispatches: events.length,
          payloadBytes: events.reduce((sum, length) => sum + length, 0),
          diagnostics,
          documentDiagnostics: w.observed.diagnostics.length
        };
      }
    };
  }

  for (const group of ["valid", "invalid"]) {
    for (const entry of boundary[group]) {
      await observe(`wire-${entry.name}`, root(content), (w, d) => {
        const probe = watch(w, d);
        const accepted = w.hemx.applyBatch(unpack(w, entry.wire).buffer, probe.owner);
        return { accepted, ...probe.finish() };
      });
    }
  }

  const valid = name => boundary.valid.find(entry => entry.name === name).wire;
  const invalid = name => boundary.invalid.find(entry => entry.name === name).wire;
  const streams = [
    { name: "small", wire: valid("small") },
    { name: "batch-exact", wire: valid("batch-exact"), declared: boundary.limits.batch },
    { name: "oversize", wire: valid("batch-exact"), extra: true },
    { name: "length-oversize", wire: valid("small"), declared: boundary.limits.batch + 1 },
    { name: "length-understated", wire: valid("batch-exact"), extra: true, declared: 1 },
    { name: "decode", wire: invalid("utf8") },
    { name: "wire-fingerprint", wire: invalid("fingerprint") },
    { name: "header-fingerprint", wire: valid("small"), fingerprint: "1" },
    { name: "http-error", wire: valid("small"), status: 503 },
    { name: "stream-error", wire: valid("small"), fail: true }
  ];
  for (const entry of streams) {
    // A non-form action uses the runtime's POST path without depending on form
    // navigation support. DOM, history, Response, and streams stay native.
    const markup = root(content + '<button type="button" data-hemx-action="7">Send</button>');
    let state;
    await observe(`wire-stream-${entry.name}`, markup, async (w, d) => {
      const probe = watch(w, d);
      const location = w.location.href;
      d.querySelector("button").click();
      await waitFor(() => state.requests > 0 && !d.querySelector("button").hasAttribute("aria-busy"),
        `wire response ${entry.name}`);
      // Allow diagnostic microtasks to expose accidental double reporting.
      await pause();
      return {
        ...probe.finish(), requests: state.requests, pulls: state.pulls, cancels: state.cancels,
        locked: state.stream.locked, bodyUsed: state.response.bodyUsed,
        locationUnchanged: w.location.href === location
      };
    }, "0", (w) => {
      state = { requests: 0, pulls: 0, cancels: 0 };
      const bytes = unpack(w, entry.wire);
      // Split ordinary batches inside their envelope. Oversize responses first
      // reach the exact cap, then deliver one extra byte in a separate chunk.
      const chunks = entry.extra ? [bytes, new w.Uint8Array([0])] : [bytes.subarray(0, 7), bytes.subarray(7)];
      let index = 0;
      state.stream = new w.ReadableStream({
        pull(controller) {
          state.pulls += 1;
          if (entry.fail) { controller.error(new w.Error("wire stream failure")); return; }
          if (index < chunks.length) controller.enqueue(chunks[index++]);
          else controller.close();
        },
        cancel() { state.cancels += 1; }
      }, { highWaterMark: 0 });
      const headers = { "Content-Type": "application/hemx" };
      if (entry.declared !== undefined) headers["Content-Length"] = String(entry.declared);
      if (entry.fingerprint !== undefined) headers["X-HEMX-Fingerprint"] = entry.fingerprint;
      state.response = new w.Response(state.stream, { status: entry.status || 200, headers });
      w.fetch = async () => { state.requests += 1; return state.response; };
    });
  }
}
