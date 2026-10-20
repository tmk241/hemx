// Native EventSource delivery, observed without replacing events,
// frames, constructors, networking, or the runtime's lifecycle callbacks.
async function observeStreamCases({ observe, root, waitFor, pause }) {
  const nativeFetch = window.fetch.bind(window);
  async function control(id, suffix, method = "GET") {
    const response = await nativeFetch(`/streams/${id}/${suffix}`, { method });
    if (!response.ok) throw new Error(`stream control ${id}/${suffix}: HTTP ${response.status}`);
    return method === "GET" ? response.json() : undefined;
  }
  async function stateWhere(id, predicate, description) {
    const deadline = performance.now() + 2000;
    while (true) {
      const state = await control(id, "state");
      if (predicate(state)) return state;
      if (performance.now() >= deadline) throw new Error(`timed out waiting for ${description}: ${JSON.stringify(state)}`);
      // Poll the observable server state; elapsed time never implies success.
      await pause(5);
    }
  }
  const attr = transport => `data-hemx-${transport}`;
  const source = (transport, id) => `/streams/${id}/${transport}`;
  const markup = (transport, id, content = '<p id="status" data-hemx-resource="1">before</p>') =>
    root(content).replace("<main ", `<main id="owner" ${attr(transport)}="${source(transport, id)}" `);

  function track(w, d, transport, missing = false) {
    const entries = [], diagnostics = [], dispatches = [];
    const api = "EventSource";
    const Native = w[api];
    function instrument(instance) {
      const entry = { instance, url: instance.url, opens: 0, closes: 0, frames: [] };
      entries.push(entry);
      instance.addEventListener("open", () => entry.opens++);
      for (const type of transport === "sse" ? ["hemx", "message"] : ["message"]) {
        instance.addEventListener(type, event => {
          const binary = event.data instanceof w.ArrayBuffer;
          const bytes = transport === "sse"
            ? Array.from(w.atob(event.data.replace(/-/g, "+").replace(/_/g, "/")), ch => ch.charCodeAt(0))
            : binary ? Array.from(new w.Uint8Array(event.data)) : null;
          entry.frames.push({ type, trusted: event.isTrusted, binary, bytes });
        });
      }
      const close = instance.close;
      instance.close = function(...args) { entry.closes++; return Reflect.apply(close, this, args); };
    }
    w[api] = class extends Native {
      constructor(...args) { super(...args); instrument(this); }
    };
    if (missing) w[api] = undefined;
    for (const type of ["hemx:error", "hemx:sse-error"]) {
      d.addEventListener(type, event => diagnostics.push({ type, owner: event.target.id, message: String(event.detail?.message || event.detail) }));
    }
    d.addEventListener("stream-proof", event => dispatches.push({ owner: event.target.id, payload: Array.from(event.detail) }));
    w.streamProbe = { entries, diagnostics, dispatches };
  }
  const summary = w => ({
    connections: w.streamProbe.entries.map(entry => ({ url: entry.url, opens: entry.opens, closes: entry.closes, frames: entry.frames })),
    diagnostics: w.streamProbe.diagnostics, dispatches: w.streamProbe.dispatches
  });
  async function opened(w, id, count = 1) {
    await stateWhere(id, state => state.opened === count && state.active === 1, `${id} server connection ${count}`);
    await waitFor(() => w.streamProbe.entries.filter(entry => entry.opens === 1).length === count, `${id} native open ${count}`);
  }
  async function closeRoot(d, id) {
    d.querySelector("#owner")?.remove();
    return stateWhere(id, state => state.active === 0 && state.closed === state.opened, `${id} server cleanup`);
  }
  async function mutate(w, d, change) {
    let delivered = false;
    const observer = new w.MutationObserver(() => { delivered = true; });
    observer.observe(d.body, { attributes: true, childList: true, subtree: true });
    change();
    try { await waitFor(() => delivered, "native stream mutation delivery"); }
    finally { observer.disconnect(); }
  }

  for (const transport of ["sse"]) {
    const delivery = `${transport}-delivery`, nested = `${transport}-nested`;
    const content = `<div data-hemx-island="editor"><p id="island" data-hemx-resource="1">island</p><section id="nested" data-hemx-root data-hemx-build="0" ${attr(transport)}="${source(transport, nested)}"><p id="nested-status" data-hemx-resource="1">nested</p></section></div><p id="status" data-hemx-resource="1">before</p>`;
    await observe(`streams-${delivery}`, markup(transport, delivery, content), async (w, d) => {
      // Two simultaneous real connections; nested open is not part of outer's
      // once-only count. Wait for both the native and server observations.
      await stateWhere(delivery, s => s.active === 1, "outer stream");
      await stateWhere(nested, s => s.active === 1, "nested stream");
      await waitFor(() => w.streamProbe.entries.length === 2 && w.streamProbe.entries.every(e => e.opens === 1), "both native streams");
      const owner = d.querySelector("#owner"), nestedOwner = d.querySelector("#nested");
      const outerDispatches = [];
      owner.addEventListener("stream-proof", event => {
        if (event.target === owner) outerDispatches.push({ payload: Array.from(event.detail), text: d.querySelector("#status").textContent });
      });
      await control(delivery, "push/valid", "POST");
      await waitFor(() => d.querySelector("#status").textContent === "delivered", "owned stream patch");
      const first = { outer: d.querySelector("#status").textContent, island: d.querySelector("#island").textContent, nested: d.querySelector("#nested-status").textContent };
      await control(nested, "push/valid", "POST");
      await waitFor(() => d.querySelector("#nested-status").textContent === "delivered", "nested owned patch");
      if (transport === "sse") {
        await control(delivery, "push/default", "POST");
        await waitFor(() => d.querySelector("#status").textContent === "default", "default SSE message event");
      }
      const final = { outer: d.querySelector("#status").textContent, island: d.querySelector("#island").textContent, nested: d.querySelector("#nested-status").textContent };
      const ownership = { outer: w.hemx.rootOf(d.querySelector("#status")) === owner, island: w.hemx.rootOf(d.querySelector("#island")) === null, nested: w.hemx.rootOf(d.querySelector("#nested-status")) === nestedOwner };
      const closed = await closeRoot(d, delivery);
      const nestedClosed = await stateWhere(nested, s => s.active === 0 && s.closed === 1, "nested stream cleanup");
      return { first, final, ownership, outerDispatches, closed, nestedClosed, ...summary(w) };
    }, "0", (w, d) => track(w, d, transport));

    const lifecycle = `${transport}-lifecycle`;
    await observe(`streams-${lifecycle}`, markup(transport, lifecycle), async (w, d) => {
      await opened(w, lifecycle);
      const owner = d.querySelector("#owner"), stages = {};
      const holder = d.createElement("article"); d.body.append(holder);
      await mutate(w, d, () => holder.append(owner));
      stages.moved = await control(lifecycle, "state");
      await control(lifecycle, "push/valid", "POST");
      await waitFor(() => d.querySelector("#status").textContent === "delivered", "delivery after connected move");
      await mutate(w, d, () => owner.setAttribute(attr(transport), `${source(transport, lifecycle)}?changed=1`));
      await opened(w, lifecycle, 2);
      stages.changed = await stateWhere(lifecycle, s => s.closed === 1, "old source cleanup");
      await mutate(w, d, () => owner.removeAttribute(attr(transport)));
      stages.removed = await stateWhere(lifecycle, s => s.closed === 2 && s.active === 0, "removed source cleanup");
      await mutate(w, d, () => owner.setAttribute(attr(transport), source(transport, lifecycle)));
      await opened(w, lifecycle, 3);
      stages.readded = await control(lifecycle, "state");
      await mutate(w, d, () => owner.removeAttribute("data-hemx-root"));
      stages.unrooted = await stateWhere(lifecycle, s => s.closed === 3 && s.active === 0, "removed root marker cleanup");
      await mutate(w, d, () => owner.setAttribute("data-hemx-root", ""));
      await opened(w, lifecycle, 4);
      stages.rerooted = await control(lifecycle, "state");
      await mutate(w, d, () => owner.remove());
      stages.detached = await stateWhere(lifecycle, s => s.closed === 4 && s.active === 0, "detached root cleanup");
      await mutate(w, d, () => holder.append(owner));
      await opened(w, lifecycle, 5);
      await control(lifecycle, "push/recovery", "POST");
      await waitFor(() => d.querySelector("#status").textContent === "recovered", "delivery after root reinsertion");
      stages.reinserted = await control(lifecycle, "state");
      const text = d.querySelector("#status").textContent;
      stages.final = await closeRoot(d, lifecycle);
      return { stages, text, ...summary(w) };
    }, "0", (w, d) => track(w, d, transport));

    for (const rejected of ["malformed", "fingerprint"]) {
      const id = `${transport}-${rejected}`;
      await observe(`streams-${id}`, markup(transport, id), async (w, d) => {
        await opened(w, id);
        const owner = d.querySelector("#owner"), before = owner.innerHTML;
        const mutations = [];
        const observer = new w.MutationObserver(records => mutations.push(...records));
        observer.observe(owner, { childList: true, attributes: true, characterData: true, subtree: true });
        let rejection;
        try {
          await control(id, `push/${rejected}`, "POST");
          await waitFor(() => w.streamProbe.diagnostics.length >= 1, "native stream rejection diagnostic");
          mutations.push(...observer.takeRecords());
          rejection = { unchanged: before === owner.innerHTML, mutations: mutations.length, dispatches: w.streamProbe.dispatches.length, diagnostics: w.streamProbe.diagnostics.slice() };
        } finally { observer.disconnect(); }
        await control(id, "push/recovery", "POST");
        await waitFor(() => d.querySelector("#status").textContent === "recovered", "valid message after rejection");
        const text = d.querySelector("#status").textContent;
        const closed = await closeRoot(d, id);
        return { rejection, text, closed, ...summary(w) };
      }, "0", (w, d) => track(w, d, transport));
    }

    for (const invalid of ["missing-api", "origin", "protocol", "credentials"]) {
      const id = `${transport}-${invalid}`;
      await observe(`streams-${id}`, markup(transport, id), async (w, d) => {
        const type = transport === "sse" ? "hemx:sse-error" : "hemx:websocket-error";
        await waitFor(() => w.streamProbe.diagnostics.some(e => e.type === type), "explicit stream admission failure");
        const state = await control(id, "state");
        const text = d.querySelector("#status").textContent;
        await closeRoot(d, id);
        return { state, text, ...summary(w) };
      }, "0", (w, d) => {
        track(w, d, transport, invalid === "missing-api");
        const owner = d.querySelector("#owner");
        if (invalid === "origin") owner.setAttribute(attr(transport), `http://not-local.invalid/streams/${id}/${transport}`);
        if (invalid === "protocol") owner.setAttribute(attr(transport), `ftp://${w.location.host}/streams/${id}/${transport}`);
        if (invalid === "credentials") owner.setAttribute(attr(transport), `http://user:password@${w.location.host}/streams/${id}/${transport}`);
      });
    }
  }

  // 200 unfolded server events: the runtime must fold what arrives within one
  // frame and touch the DOM far fewer than 200 times.
  await observe("streams-sse-burst", root('<p id="status" data-hemx-resource="1">0</p>').replace("<main ", '<main data-hemx-sse="/streams/burst" '), async (w, d) => {
    const status = d.querySelector("#status");
    let callbacks = 0;
    new w.MutationObserver(() => callbacks++).observe(status, { childList: true, characterData: true, subtree: true });
    await waitFor(() => status.textContent === "200", "final burst value");
    await pause(50);
    return { text: status.textContent, callbacks, diagnostics: w.observed.diagnostics };
  });

  // A replacement then a morph, applied in order, end in the new element.
  await observe("streams-sse-patch-fold", root('<div data-hemx-resource="1">x</div>').replace("<main ", '<main data-hemx-sse="/streams/patch-fold" '), async (w, d) => {
    await waitFor(() => d.querySelector("[data-hemx-resource]")?.textContent === "done", "final patch");
    await pause(50);
    const element = d.querySelector("[data-hemx-resource]");
    return { tag: element.localName, text: element.textContent, diagnostics: w.observed.diagnostics };
  });

  // 151 unfolded row deltas: folded per frame into chained deltas, they must
  // reach the exact final order with few DOM updates.
  await observe("streams-sse-delta-burst", root('<ul data-hemx-resource="1" data-hemx-collection=""></ul>').replace("<main ", '<main data-hemx-sse="/streams/delta-burst" '), async (w, d) => {
    const list = d.querySelector("ul");
    let callbacks = 0;
    new w.MutationObserver(() => callbacks++).observe(list, { childList: true });
    await waitFor(() => list.firstElementChild?.getAttribute("data-hemx-key") === "k100", "final delta");
    await pause(50);
    return { keys: Array.from(list.children, (row) => row.getAttribute("data-hemx-key")), callbacks, diagnostics: w.observed.diagnostics };
  });
}
