// Runtime request and navigation behavior that is not specific to forms or
// effects. Native dialogs cannot be answered by page script, so the confirm
// case replaces window.confirm with a recorded answer sequence.
async function observeRuntimeCases({ observe, root, waitFor, pause }) {
  await observe("runtime-confirm", root('<button id="act" type="button" data-hemx-action="5" data-hemx-confirm="Delete?">Delete</button>'),
    async (w, d) => {
      const button = d.querySelector("#act");
      button.click(); // declined
      button.click(); // accepted
      await waitFor(() => w.observed.requests.length === 1, "confirmed request");
      await waitFor(() => !button.hasAttribute("aria-busy"), "confirmed request completion");
      return { prompts: w.prompts, requests: w.observed.requests.length, credentials: w.observed.credentials, diagnostics: w.observed.diagnostics };
    }, "0", w => {
      const answers = [false, true];
      w.prompts = [];
      w.confirm = message => { w.prompts.push(message); return answers.shift(); };
    });

  await observe("runtime-pagehide", root('<button id="act" type="button" data-hemx-action="5">Load</button>'),
    async (w, d) => {
      const button = d.querySelector("#act");
      button.click();
      await waitFor(() => w.pendingSignal, "request start");
      const busy = button.getAttribute("aria-busy");
      // Synthetic lifecycle event: the probe tests the runtime's handler, not
      // the browser's decision to unload this document.
      w.dispatchEvent(new w.PageTransitionEvent("pagehide", { persisted: false }));
      await waitFor(() => w.pendingSignal.aborted && !button.hasAttribute("aria-busy"), "page-hide abort");
      await pause();
      return { busy, reason: w.pendingSignal.reason.name, diagnostics: w.observed.diagnostics };
    }, "0", w => {
      w.fetch = (url, options) => new Promise((_, reject) => {
        w.pendingSignal = options.signal;
        options.signal.addEventListener("abort", () => reject(options.signal.reason));
      });
    });

  await observe("runtime-drag-drop", root('<ul><li id="item" data-hemx-key="7" draggable="true">Item</li></ul><div id="zone" data-hemx-action="9" data-hemx-on="drop" data-tooltip="Drop here" data-hemx-param-list="inbox">Drop</div>'),
    async (w, d) => {
      // Synthetic drag events: the probe checks the runtime's key transfer, not
      // the browser's pointer gesture recognition.
      const transfer = new w.DataTransfer();
      const fire = (id, type) => {
        const event = new w.DragEvent(type, { bubbles: true, cancelable: true, dataTransfer: transfer });
        d.querySelector(`#${id}`).dispatchEvent(event);
        return event.defaultPrevented;
      };
      fire("item", "dragstart");
      const overAccepted = fire("zone", "dragover");
      fire("zone", "drop");
      await waitFor(() => w.observed.requests.length === 1, "drop request");
      return { overAccepted, text: transfer.getData("text/plain"), entries: w.observed.requests[0].entries };
    });

  const pageOne = '<nav data-hemx-slot="nav">one</nav><a id="go" href="/page-two" data-hemx-nav>Go</a><section data-hemx-slot="content"><p id="content">page one</p></section>';
  await observe("runtime-page-swap", root(pageOne), async (w, d) => {
    const history = w.history.length;
    d.querySelector("#go").click();
    await waitFor(() => w.location.pathname === "/page-two", "partial navigation");
    const afterNavigation = {
      path: w.location.pathname, historyDelta: w.history.length - history, title: d.title,
      content: d.querySelector("#content").textContent, nav: d.querySelector("nav").textContent,
    };
    // No page HTML may be stored in history.
    const storedState = JSON.stringify(w.history.state);
    w.history.back();
    // The server's current page one differs from what was first rendered, so
    // only a refetch, not a stored snapshot, can produce this text.
    await waitFor(() => w.location.pathname === "/frame" && d.querySelector("#content")?.textContent === "page one, refetched", "history refetch");
    return {
      afterNavigation, storedState,
      afterBack: { title: d.title, content: d.querySelector("#content").textContent, nav: d.querySelector("nav").textContent },
      requests: w.pageRequests, diagnostics: w.observed.diagnostics,
    };
  }, "0", w => {
    w.pageRequests = [];
    const page = (nav, content, title) => new Response(
      `<!doctype html><main data-hemx-root><nav data-hemx-slot="nav">${nav}</nav><section data-hemx-slot="content"><p id="content">${content}</p></section></main>`,
      { headers: { "Content-Type": "text/html", "X-Hemx-Title": title } });
    w.fetch = async (url, options) => {
      w.pageRequests.push({ url: String(url), headers: options.headers, credentials: options.credentials });
      return new URL(url, w.location.href).pathname === "/page-two"
        ? page("two", "page two", "Page two") : page("one", "page one, refetched", "Page one");
    };
  });

  const escapes = ["ctrl", "blank", "download", "external", "prevented", "plain"];
  await observe("runtime-native-navigation", root(
    '<a id="ctrl" href="/x" data-hemx-nav>c</a><a id="blank" href="/x" target="_blank" data-hemx-nav>b</a>' +
    '<a id="download" href="/x" download data-hemx-nav>d</a><a id="external" href="https://example.invalid/x" data-hemx-nav>e</a>' +
    '<a id="prevented" href="/x" data-hemx-nav>p</a><a id="plain" href="/plain" data-hemx-nav>n</a>'), async (w, d) => {
    const handled = {};
    // Runs after the root listener; records its decision, then stops any
    // native navigation so the fixture document remains under test.
    w.addEventListener("click", event => { handled[event.target.id] = event.defaultPrevented; event.preventDefault(); });
    d.querySelector("#prevented").addEventListener("click", event => event.preventDefault());
    for (const id of escapes) {
      d.querySelector(`#${id}`).dispatchEvent(new w.MouseEvent("click", { bubbles: true, cancelable: true, ctrlKey: id === "ctrl" }));
    }
    // Report the decisions even if the positive control fails; Rust owns the verdict.
    await waitFor(() => w.observed.requests.length === 1, "plain partial navigation").catch(() => {});
    return { handled, requests: w.observed.requests.map(request => request.url), diagnostics: w.observed.diagnostics };
  });

  // A stale page is told apart from an application conflict by the server's
  // build header on the 409. Either way the request is reported once, never replayed.
  const outlet = '<p id="error" data-hemx-error hidden></p>';
  for (const [name, markup, trigger, headers] of [
    ["integrity-stale-action", `<button id="act" type="button" data-hemx-action="5">Go</button>${outlet}`, d => d.querySelector("#act").click(), { "x-hemx-fingerprint": "99" }],
    ["integrity-stale-form", `<form id="f" method="post" action="/mutate" data-hemx-action="7"><input name="title" value="x"><button>Go</button></form>${outlet}`, d => d.querySelector("#f button").click(), { "x-hemx-fingerprint": "99" }],
    ["integrity-app-conflict", `<button id="act" type="button" data-hemx-action="5">Go</button>${outlet}`, d => d.querySelector("#act").click(), {}],
  ]) {
    await observe(name, root(markup), async (w, d) => {
      const content = () => [...d.querySelector("main").children].filter(node => node.id !== "error").map(node => node.outerHTML).join("");
      const before = { content: content(), url: w.location.href, history: w.history.length };
      const errors = [];
      d.addEventListener("hemx:error", event => errors.push(String(event.detail && event.detail.message)));
      trigger(d);
      await waitFor(() => errors.length >= 1, "rejection diagnostic");
      await pause(150); // A replay would arrive in this window.
      const error = d.querySelector("#error");
      return {
        requests: w.rejectedRequests.length, errors,
        outlet: { text: error.textContent, hidden: error.hasAttribute("hidden") },
        unchanged: content() === before.content, sameUrl: w.location.href === before.url,
        sameHistory: w.history.length === before.history, busy: d.querySelectorAll("[aria-busy]").length,
      };
    }, "0", w => {
      w.rejectedRequests = [];
      w.fetch = async (url, options = {}) => {
        w.rejectedRequests.push(options.method || "GET");
        return new Response("conflict", { status: 409, headers });
      };
    });
  }

  await observe("runtime-popstate-fallback", root('<p id="content">initial</p>'), async (w, d) => {
    const requests = w.fallbackRequests; // The reloaded document has a new global.
    w.history.pushState(null, "", "/elsewhere");
    w.history.back();
    // The unsnapshotted entry cannot be restored and the response is not
    // applicable; the runtime must reload rather than show stale UI.
    await waitFor(() => w.document !== d && w.document.readyState === "complete", "document reload");
    return { reloaded: true, path: w.location.pathname, requests };
  }, "0", w => {
    w.fallbackRequests = [];
    w.fetch = async url => {
      w.fallbackRequests.push(String(url));
      return new Response("not a Hemx response", { headers: { "Content-Type": "text/plain" } });
    };
  });
}
