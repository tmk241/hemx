// Admission-specific native contracts, separate from transport/fallback tests.
async function observeFormAdmission({observe, root, waitFor, pause}) {
  for (const name of ["navigate-listener", "manual-data", "manual-data-canceled", "multipart", "image-get", "no-navigation-api", "base-target"]) {
    const image = name === "image-get";
    // A <base target> sends a form without its own target elsewhere: native.
    const base = name === "base-target" ? '<base target="hemx-elsewhere">' : "";
    const markup = base + root(`<form action="/admission" method="${image ? "get" : "post"}" data-hemx-action="7"><input name="value" value="native"><input type="hidden" name="__h" value="7">${name === "multipart" ? '<input id="file" type="file" name="attachment">' : ''}${image ? '<input type="image" name="pick" data-hemx-action="8" alt="Send">' : '<button name="choice" value="save" data-hemx-action="8">Send</button>'}</form>`);
    await observe(`form-admission-${name}`, markup, async (w, d) => {
      const form = d.querySelector("form");
      const source = d.querySelector('button,input[type="image"]');
      const before = { url: w.location.href, history: w.history.length, state: JSON.stringify(w.history.state) };
      let dataEvents = 0, vetoes = 0, serialized = 0, navigations = 0;
      // An enhanced submission must never start a document navigation, which is
      // what a native submission does (automation such as Playwright waits
      // forever for a canceled one). A GET result's own pushState is a
      // same-document navigation and does not count.
      w.navigation?.addEventListener("navigate", (event) => { if (!event.destination.sameDocument) navigations++; });
      form.addEventListener("formdata", event => { dataEvents++; event.formData.append("hook", "once"); });
      // Count page-script serializations of the form (Hemx or the app); the
      // browser's own entry-list construction is not a page-script call.
      const NativeFormData = w.FormData;
      w.FormData = new Proxy(NativeFormData, { construct(target, args) {
        if (args[0]) serialized++;
        return Reflect.construct(target, args);
      } });
      if (name.startsWith("manual-data")) form.addEventListener("submit", () => { new w.FormData(form); });
      if (name === "manual-data-canceled") d.addEventListener("submit", event => { vetoes++; event.preventDefault(); });
      if (name === "navigate-listener") w.navigation.addEventListener("navigate", event => { vetoes++; event.preventDefault(); });
      if (name === "multipart") {
        source.setAttribute("formenctype", "multipart/form-data");
        const transfer = new w.DataTransfer();
        transfer.items.add(new w.File(["file-proof"], "proof.txt", {type: "text/plain"}));
        d.querySelector("#file").files = transfer.files;
      }
      source.click();
      if (name === "manual-data-canceled" || name === "base-target") await pause();
      else {
        await waitFor(() => w.observed.requests.length === 1, `admitted ${name}`);
        await waitFor(() => !form.hasAttribute("aria-busy"), `completed ${name}`);
        if (image) await waitFor(() => w.location.pathname === "/admission", "image GET history");
      }
      w.FormData = NativeFormData;
      return {dataEvents, vetoes, serialized, requests: w.observed.requests, before, url: w.location.href,
        history: w.history.length, state: JSON.stringify(w.history.state), diagnostics: w.observed.diagnostics,
        navigations, navigationApi: typeof w.navigation};
    }, "0", name === "no-navigation-api" ? w => {
      Object.defineProperty(w, "navigation", {value: undefined, configurable: true});
    } : undefined);
  }

  // A submit listener of form A that submits form B: each submission keeps its
  // own takeover, so both are enhanced and nothing navigates.
  const nested = root(`<form id="a" action="/nested-a" method="post" data-hemx-action="7"><input type="hidden" name="__h" value="7"><button name="which" value="a">A</button></form><form id="b" action="/nested-b" method="post" data-hemx-action="7"><input type="hidden" name="__h" value="7"><button name="which" value="b">B</button></form>`);
  await observe("form-admission-nested", nested, async (w, d) => {
    const before = w.location.href;
    let submittedB = false;
    d.querySelector("#a").addEventListener("submit", () => {
      if (submittedB) return;
      submittedB = true;
      d.querySelector("#b").requestSubmit();
    });
    d.querySelector("#a button").click();
    await waitFor(() => w.observed.requests.length === 2, "both nested submissions");
    await waitFor(() => !d.querySelector('[aria-busy="true"]'), "nested completion");
    return {urls: w.observed.requests.map(request => new URL(request.url).pathname).sort(), url: w.location.href, before};
  }, "0");
}
