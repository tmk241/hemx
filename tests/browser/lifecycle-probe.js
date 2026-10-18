// Observations only: Rust owns the expected outcomes. These wrappers delegate
// to native timers and IntersectionObserver; they expose registration/cleanup
// without substituting a clock, geometry, observer callback, DOM, or history.
async function observeLifecycleCases({ observe, root, apply, cases, waitFor, pause }) {
  function trackNatives(w) {
    const state = { intervals: new Map(), timeouts: new Map(), fired: [], observers: [], entries: [] };
    for (const [set, clear, map, repeat] of [
      ["setInterval", "clearInterval", state.intervals, true],
      ["setTimeout", "clearTimeout", state.timeouts, false]
    ]) {
      const nativeSet = w[set].bind(w), nativeClear = w[clear].bind(w);
      w[set] = (callback, ms, ...args) => {
        let id;
        id = nativeSet((...values) => {
          if (!repeat) map.delete(id);
          state.fired.push({ repeat, ms, id });
          callback(...values);
        }, ms, ...args);
        map.set(id, ms);
        return id;
      };
      w[clear] = id => { map.delete(id); nativeClear(id); };
    }
    const NativeObserver = w.IntersectionObserver;
    w.IntersectionObserver = class extends NativeObserver {
      constructor(callback, options) {
        super((entries, observer) => {
          state.entries.push(...entries.map(entry => ({ target: entry.target, intersecting: entry.isIntersecting })));
          callback(entries, observer);
        }, options);
        this.targets = new Set();
        this.observeCalls = 0;
        this.disconnectCalls = 0;
        state.observers.push(this);
      }
      observe(target) { this.targets.add(target); this.observeCalls++; return super.observe(target); }
      unobserve(target) { this.targets.delete(target); return super.unobserve(target); }
      disconnect() { this.targets.clear(); this.disconnectCalls++; return super.disconnect(); }
    };
    w.lifecycleNative = state;
  }

  // A separate native mutation observer supplies a completion barrier, not an
  // assumption that one arbitrary timeout means lifecycle work has finished.
  async function mutate(w, d, change) {
    let delivered = false;
    const observer = new w.MutationObserver(() => { delivered = true; });
    observer.observe(d.body, { subtree: true, childList: true, attributes: true });
    change();
    try { await waitFor(() => delivered, "native mutation delivery"); }
    finally { observer.disconnect(); }
  }
  const active = (map) => Array.from(map.values()).sort((a, b) => a - b);
  const requestsFor = (w, name) => w.observed.requests.filter(request => request.entries.some(([key, value]) => key === "probe" && value === name)).length;
  const status = d => d.querySelector("#status").textContent;
  const pending = d => {
    const target = d.querySelector("#work") || d.querySelector("#trigger");
    return { busy: target.getAttribute("aria-busy"), loading: target.classList.contains("loading"),
      disabled: d.querySelector("#field")?.disabled ?? null,
      hidden: d.querySelector("#indicator")?.getAttribute("hidden") ?? null };
  };
  function control(w) {
    const requests = [];
    w.fetch = (url, options = {}) => new Promise((resolve, reject) => {
      const request = { url: String(url), method: options.method, entries: Array.from(options.body?.entries() || []), signal: options.signal, resolve, reject };
      requests.push(request);
    });
    return {
      requests,
      reply(index, name = "empty", code = 200) {
        const bytes = name === "empty" ? cases.wire.empty : cases.lifecycle.wire[name];
        if (!bytes) throw new Error(`missing lifecycle wire: ${name}`);
        requests[index].resolve(new w.Response(new w.Uint8Array(bytes), { status: code, headers: { "Content-Type": "application/hemx" } }));
      },
      reject(index) { requests[index].reject(new w.TypeError("controlled network failure")); },
      bad(index) { requests[index].resolve(new w.Response(new w.Uint8Array([0]), { headers: { "Content-Type": "application/hemx" } })); },
      // Controlled-request fixtures have no scheduling attributes. Their only
      // runtime timeouts are the live request deadlines (runner waits live in
      // the parent window), so clearing one is observable finalization.
      running: () => w.lifecycleNative.timeouts.size,
      values: () => requests.map(request => request.entries.filter(([key]) => key === "value").map(([, value]) => value)),
      handles: () => requests.map(request => request.entries.filter(([key]) => key === "__h").map(([, value]) => value))
    };
  }
  const policyMarkup = policy => root(`<form id="work" method="post" data-hemx-policy="${policy}" data-hemx-pending-class="loading"><input id="field" name="value" value="A"><button id="trigger" type="button" data-hemx-action="7">Action</button></form><p id="status" data-hemx-resource="2">initial</p>`);

  for (const policy of ["latest", "queue", "drop", "parallel"]) {
    await observe(`lifecycle-policy-${policy}`, policyMarkup(policy), async (w, d) => {
      const transport = control(w), button = d.querySelector("button"), field = d.querySelector("input");
      let captures = 0;
      d.querySelector("form").addEventListener("formdata", () => captures++);
      button.click();
      field.value = "B"; button.click();
      if (policy === "queue") { field.value = "C"; button.click(); }
      field.value = "not-submitted";
      const admitted = transport.requests.length, capturedAtAdmission = captures;
      const aborted = transport.requests.map(request => request.signal.aborted);
      let intermediate;
      if (policy === "latest") {
        // Deliberately let an aborted transport resolve anyway. Admission must
        // release exactly once, and stale bytes must not reach the DOM.
        transport.reply(0, "stale");
        await waitFor(() => transport.running() === 1, "superseded request finalization");
        intermediate = { status: status(d), ...pending(d) };
        transport.reply(1, "fresh");
      } else if (policy === "queue") {
        transport.reply(0, "first");
        await waitFor(() => transport.requests.length === 2, "second queued request");
        intermediate = { status: status(d), requests: transport.requests.length };
        transport.reply(1, "second");
        await waitFor(() => transport.requests.length === 3, "third queued request");
        transport.reply(2, "third");
      } else if (policy === "drop") {
        transport.reply(0, "first");
        await waitFor(() => transport.running() === 0, "drop policy first completion");
        field.value = "C"; button.click();
        await waitFor(() => transport.requests.length === 2, "drop policy readmission");
        intermediate = { status: status(d), requests: transport.requests.length };
        transport.reply(1, "fresh");
      } else {
        transport.reply(1, "second");
        await waitFor(() => transport.running() === 1, "parallel second completion");
        intermediate = { status: status(d), ...pending(d) };
        transport.reply(0, "first");
      }
      await waitFor(() => transport.running() === 0, `${policy} finalization`);
      return { admitted, capturedAtAdmission, captures, aborted, intermediate, values: transport.values(), handles: transport.handles(), status: status(d), final: pending(d), diagnostics: w.observed.diagnostics };
    }, "0", trackNatives);
  }

  await observe("lifecycle-queue-removal", policyMarkup("queue"), async (w, d) => {
    const transport = control(w), form = d.querySelector("form"), button = d.querySelector("button");
    button.click(); button.click();
    await mutate(w, d, () => form.remove());
    transport.reply(0);
    await waitFor(() => transport.running() === 0 && form.getAttribute("aria-busy") === null, "detached active request release");
    return { requests: transport.requests.length, connected: form.isConnected, busy: form.getAttribute("aria-busy"), loading: form.classList.contains("loading"), diagnostics: w.observed.diagnostics };
  }, "0", trackNatives);

  function pendingMarkup(disabled = false, preexisting = false) {
    return root(`<section data-hemx-resource="1"><form id="work" method="post" data-hemx-policy="parallel" data-hemx-pending-class="loading" data-hemx-disable-while-pending class="${preexisting ? "base loading" : "base"}" aria-busy="false"><input id="field" name="value" value="server" ${disabled ? "disabled" : ""}></form><button id="trigger" type="button" form="work" data-hemx-action="7">Action</button><span id="indicator" data-hemx-indicator hidden="until-found">Working</span><output data-hemx-error hidden></output></section>`);
  }
  for (const baseline of ["preserved", "disabled", "enabled"]) {
    await observe(`lifecycle-pending-${baseline}`, pendingMarkup(baseline === "enabled", baseline === "preserved"), async (w, d) => {
      const transport = control(w), button = d.querySelector("button");
      const nodes = [d.querySelector("form"), d.querySelector("input"), d.querySelector("#indicator")];
      const initial = pending(d);
      button.click(); button.click();
      const overlapping = pending(d);
      transport.reply(0, baseline === "preserved" ? "empty" : `baseline-${baseline}`);
      await waitFor(() => transport.running() === 1, "first pending release and server update");
      const between = pending(d);
      transport.reply(1);
      await waitFor(() => transport.running() === 0, "last pending release");
      return { initial, overlapping, between, final: pending(d), same: nodes.map(node => d.getElementById(node.id) === node), requests: transport.requests.length, diagnostics: w.observed.diagnostics };
    }, "0", trackNatives);
  }

  for (const failure of ["http", "decode", "network", "removed"]) {
    await observe(`lifecycle-pending-error-${failure}`, pendingMarkup(), async (w, d) => {
      const transport = control(w), owner = d.querySelector("main"), form = d.querySelector("form"), indicator = d.querySelector("#indicator"), field = d.querySelector("input");
      const diagnostics = [];
      owner.addEventListener("hemx:error", event => diagnostics.push({ owned: event.target === owner, message: event.detail.message }));
      d.querySelector("button").click();
      const during = pending(d);
      if (failure === "removed") await mutate(w, d, () => owner.remove());
      if (failure === "http") transport.reply(0, "empty", 503);
      else if (failure === "decode") transport.bad(0);
      else transport.reject(0);
      await waitFor(() => transport.running() === 0 && diagnostics.length > 0, "failed request diagnostic and release");
      const outlet = owner.querySelector("output");
      return { during, final: { busy: form.getAttribute("aria-busy"), loading: form.classList.contains("loading"), disabled: field.disabled, hidden: indicator.getAttribute("hidden") }, diagnostics, outlet: { hidden: outlet.hidden, text: outlet.textContent }, connected: owner.isConnected };
    }, "0", trackNatives);
  }

  await observe("lifecycle-scheduling", root('<button id="debounce" type="button" data-hemx-action="7" data-hemx-param-probe="debounce" data-hemx-debounce="70">Debounce</button><button id="throttle" type="button" data-hemx-action="7" data-hemx-param-probe="throttle" data-hemx-throttle="90">Throttle</button>'), async (w, d) => {
    const state = w.lifecycleNative, debounce = d.querySelector("#debounce"), throttle = d.querySelector("#throttle"), owner = d.querySelector("main");
    debounce.click(); debounce.click();
    const debounced = { timers: active(state.timeouts), requests: requestsFor(w, "debounce") };
    // A still-connected root move must not cancel its outstanding schedule.
    const originalTimer = Array.from(state.timeouts.keys())[0];
    await mutate(w, d, () => d.body.append(owner));
    const retained = state.timeouts.has(originalTimer);
    await waitFor(() => requestsFor(w, "debounce") === 1, "debounced request after root move");
    debounce.click();
    await mutate(w, d, () => debounce.setAttribute("data-hemx-debounce", "75"));
    const configCancelled = !Array.from(state.timeouts.values()).includes(70);
    debounce.click();
    await waitFor(() => requestsFor(w, "debounce") === 2, "updated debounce timer");
    debounce.click();
    await mutate(w, d, () => debounce.remove());
    const debounceCancelled = !Array.from(state.timeouts.values()).includes(75);
    throttle.click(); throttle.click(); throttle.click();
    const throttled = requestsFor(w, "throttle");
    await waitFor(() => state.fired.some(item => !item.repeat && item.ms === 90), "throttle interval expiration");
    throttle.click();
    await waitFor(() => requestsFor(w, "throttle") === 2, "throttle readmission");
    await mutate(w, d, () => throttle.remove());
    const throttleCancelled = !Array.from(state.timeouts.values()).includes(90);
    await mutate(w, d, () => owner.append(throttle));
    throttle.click();
    await waitFor(() => requestsFor(w, "throttle") === 3, "reattached throttle admission");
    await mutate(w, d, () => owner.remove());
    return { debounced, retained, configCancelled, debounceCancelled, throttled, throttleCancelled, finalTimers: active(state.timeouts).filter(ms => ms === 70 || ms === 75 || ms === 90), debounceRequests: requestsFor(w, "debounce"), throttleRequests: requestsFor(w, "throttle"), bindings: w.observed.bindings, diagnostics: w.observed.diagnostics };
  }, "0", trackNatives);

  await observe("lifecycle-polling", root('<button id="poll" type="button" data-hemx-action="7" data-hemx-param-probe="poll" data-hemx-every="40">Poll</button>'), async (w, d) => {
    const state = w.lifecycleNative, poll = d.querySelector("#poll"), owner = d.querySelector("main");
    await waitFor(() => requestsFor(w, "poll") > 0, "native polling");
    const inserted = d.createElement("button");
    inserted.setAttribute("type", "button"); inserted.setAttribute("data-hemx-action", "7");
    inserted.setAttribute("data-hemx-param-probe", "inserted"); inserted.setAttribute("data-hemx-interval", "55");
    await mutate(w, d, () => owner.append(inserted));
    await waitFor(() => requestsFor(w, "inserted") > 0, "inserted polling adapter");
    const initial = active(state.intervals);
    await mutate(w, d, () => { owner.append(d.createElement("span")); owner.append(d.createElement("span")); });
    const rescanned = active(state.intervals);
    await mutate(w, d, () => poll.setAttribute("data-hemx-every", "65"));
    const changed = active(state.intervals);
    await mutate(w, d, () => poll.setAttribute("data-hemx-interval", "75"));
    const precedence = active(state.intervals);
    const beforeChangedTick = requestsFor(w, "poll");
    await waitFor(() => state.fired.some(item => item.repeat && item.ms === 75) && requestsFor(w, "poll") > beforeChangedTick, "reconfigured native polling dispatch");
    await mutate(w, d, () => { poll.removeAttribute("data-hemx-interval"); poll.removeAttribute("data-hemx-every"); inserted.remove(); });
    const removed = active(state.intervals);
    await mutate(w, d, () => owner.append(inserted));
    const before = requestsFor(w, "inserted");
    await waitFor(() => requestsFor(w, "inserted") > before, "reattached polling adapter");
    const reattached = active(state.intervals);
    await mutate(w, d, () => owner.remove());
    return { initial, rescanned, changed, precedence, removed, reattached, final: active(state.intervals), diagnostics: w.observed.diagnostics };
  }, "0", trackNatives);

  await observe("lifecycle-polling-ownership", root('<section id="gate"><button id="poll" type="button" data-hemx-action="7" data-hemx-every="40">Poll</button></section><div data-hemx-island="widget"><button type="button" data-hemx-action="7" data-hemx-every="45">Ignored</button><aside data-hemx-root data-hemx-build="0"><button type="button" data-hemx-action="7" data-hemx-interval="55">Nested</button></aside></div>') + '<main id="other" data-hemx-root data-hemx-build="0"></main>', async (w, d) => {
    const state = w.lifecycleNative, gate = d.querySelector("#gate"), poll = d.querySelector("#poll"), other = d.querySelector("#other"), owner = d.querySelector("main");
    const initial = active(state.intervals);
    await mutate(w, d, () => gate.setAttribute("data-hemx-island", "editor"));
    const island = active(state.intervals);
    await mutate(w, d, () => gate.removeAttribute("data-hemx-island"));
    const restored = active(state.intervals);
    await mutate(w, d, () => other.append(poll));
    const moved = active(state.intervals);
    await mutate(w, d, () => other.removeAttribute("data-hemx-root"));
    const unowned = active(state.intervals);
    await mutate(w, d, () => other.setAttribute("data-hemx-root", ""));
    const reclaimed = active(state.intervals);
    await mutate(w, d, () => owner.remove());
    const removedOwner = active(state.intervals);
    await mutate(w, d, () => other.remove());
    return { initial, island, restored, moved, unowned, reclaimed, removedOwner, final: active(state.intervals), diagnostics: w.observed.diagnostics };
  }, "0", trackNatives);

  await observe("lifecycle-revealed", root('<button id="reveal" type="button" style="position:absolute;left:0" data-hemx-action="7" data-hemx-param-probe="reveal" data-hemx-revealed>Reveal</button><div data-hemx-island="widget"><button id="ignored" type="button" data-hemx-action="7" data-hemx-param-probe="ignored" data-hemx-revealed>Ignored</button><aside data-hemx-root data-hemx-build="0"><button id="nested" type="button" data-hemx-action="7" data-hemx-param-probe="nested" data-hemx-revealed>Nested</button></aside></div>') + '<main id="other" data-hemx-root data-hemx-build="0"><section id="gate"></section></main>', async (w, d) => {
    const state = w.lifecycleNative, reveal = d.querySelector("#reveal"), owner = d.querySelector("main"), other = d.querySelector("#other"), gate = d.querySelector("#gate");
    const watched = node => state.observers.reduce((sum, observer) => sum + Number(observer.targets.has(node)), 0);
    await waitFor(() => state.entries.some(entry => entry.target === reveal && !entry.intersecting) && requestsFor(w, "nested") === 1, "native initial intersection delivery");
    const initial = { reveal: requestsFor(w, "reveal"), ignored: requestsFor(w, "ignored"), nested: requestsFor(w, "nested"), watching: watched(reveal), ignoredWatching: watched(d.querySelector("#ignored")) };
    const observes = state.observers.reduce((sum, observer) => sum + observer.observeCalls, 0);
    await mutate(w, d, () => owner.append(d.createElement("span")));
    const duplicateObservations = state.observers.reduce((sum, observer) => sum + observer.observeCalls, 0) - observes;
    await mutate(w, d, () => reveal.setAttribute("data-hemx-revealed-ahead", "1"));
    await waitFor(() => requestsFor(w, "reveal") === 1, "changed native reveal margin");
    const first = requestsFor(w, "reveal");
    let resized = false;
    w.addEventListener("resize", () => { resized = true; }, { once: true });
    w.frameElement.style.height = `${w.innerHeight + 40}px`;
    await waitFor(() => resized, "native iframe resize");
    const afterResize = { requests: requestsFor(w, "reveal"), watching: watched(reveal) };
    await mutate(w, d, () => reveal.removeAttribute("data-hemx-revealed"));
    await mutate(w, d, () => reveal.setAttribute("data-hemx-revealed", ""));
    await waitFor(() => requestsFor(w, "reveal") === 2, "re-enabled reveal");
    await mutate(w, d, () => gate.append(reveal));
    await waitFor(() => requestsFor(w, "reveal") === 3, "new root reveal ownership");
    await mutate(w, d, () => gate.setAttribute("data-hemx-island", "editor"));
    const islandWatching = watched(reveal);
    await mutate(w, d, () => gate.removeAttribute("data-hemx-island"));
    await waitFor(() => requestsFor(w, "reveal") === 4, "restored reveal ownership");
    const inserted = d.createElement("button");
    inserted.type = "button";
    for (const [name, value] of [["data-hemx-action", "7"], ["data-hemx-param-probe", "inserted"], ["data-hemx-revealed", ""]]) inserted.setAttribute(name, value);
    await mutate(w, d, () => other.append(inserted));
    await waitFor(() => requestsFor(w, "inserted") === 1, "inserted reveal adapter");
    // Leave a genuinely observed, nonintersecting node pending on removal.
    await mutate(w, d, () => { reveal.removeAttribute("data-hemx-revealed"); reveal.style.top = `${w.innerHeight * 10}px`; });
    const beforePending = state.entries.length;
    await mutate(w, d, () => reveal.setAttribute("data-hemx-revealed", ""));
    await waitFor(() => watched(reveal) === 1 && state.entries.slice(beforePending).some(entry => entry.target === reveal && !entry.intersecting), "pending native reveal binding");
    const disconnects = state.observers.reduce((sum, observer) => sum + observer.disconnectCalls, 0);
    await mutate(w, d, () => { owner.remove(); other.remove(); });
    return { initial, duplicateObservations, first, afterResize, islandWatching, reveals: requestsFor(w, "reveal"), inserted: requestsFor(w, "inserted"), finalWatching: state.observers.reduce((sum, observer) => sum + observer.targets.size, 0), disconnected: state.observers.reduce((sum, observer) => sum + observer.disconnectCalls, 0) > disconnects, diagnostics: w.observed.diagnostics };
  }, "0", (w, d) => {
    trackNatives(w);
    // Keep the target inside the iframe but below the top-level viewport.
    // rootMargin expands the implicit top-level root, not iframe clipping.
    // The nested control remains in the visible upper half of the frame.
    Object.assign(w.frameElement.style, { position: "fixed", top: `${window.innerHeight - 90}px`, left: "0", width: "400px", height: "180px" });
    d.querySelector("#reveal").style.top = "160px";
  });
}
