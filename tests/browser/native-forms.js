// Real HTTP, same-origin iframe navigation. This deliberately restores fetch:
// protocol stubs in the other browser fixtures cannot prove native fallback.
async function observeNativeForms({observe, fixture, root, cases, waitFor, pause}) {
  const fingerprint = cases.generatedFingerprint;
  // fixture/root are part of the shared probe API; observe owns frame cleanup.
  void fixture;
  void root;

  async function reset(id) {
    const response = await fetch(`/native-forms/${id}/reset`, {method: "POST"});
    if (response.status !== 204) throw new Error(`reset ${id}: ${response.status}`);
  }

  async function state(id) {
    const response = await fetch(`/native-forms/${id}/state`);
    if (!response.ok) throw new Error(`state ${id}: ${response.status}`);
    return response.json();
  }

  function form(id, method, valid, override) {
    const named = ["action", "method", "enctype", "target", "submit", "requestSubmit"]
      .map(name => `<input name="${name}" value="shadow-${name}">`).join("");
    // GET override exercises the native submitter URL/method/name contract.
    // Enhanced POST override proves identical domain mutation on another route.
    const button = override
      ? `formaction="/native-forms/${id}/override" formmethod="${method}"`
      : "";
    const initialMethod = override && method === "get" ? "post" : method;
    return `<main data-hemx-root data-hemx-build="${fingerprint}">
      <form action="/native-forms/${id}/submit" method="${initialMethod}" data-hemx-action="7">
        <input type="hidden" name="__h" value="7">
        <label>Value<input name="value" value="${valid ? "a & é" : ""}"></label>
        ${named}<button name="choice" value="save" ${button}>Save</button>
      </form>${cases.textInitial}</main>`;
  }

  for (const mode of ["no-runtime", "missing-navigation", "enhanced"]) {
    const scenarios = mode === "enhanced"
      ? [["post", true, false], ["post", false, false], ["post", true, true]]
      : [["get", true, false], ["get", false, false], ["post", true, false],
         ["post", false, false], ["get", true, true]];
    for (const [method, valid, override] of scenarios) {
      const id = `${mode}-${method}-${valid ? "success" : "invalid"}${override ? "-override" : ""}`;
      const enhanced = mode === "enhanced";
      await reset(id);
      await observe(`native-http-${id}`, form(id, method, valid, override), async (w, d) => {
        const before = w.location.href;
        const oldHistory = w.history.length;
        const runtimeLoaded = !!w.hemx;
        const navigationAvailable = !!w.navigation;
        d.querySelector("button").click();
        if (enhanced) {
          await waitFor(() => d.querySelector("#nested")?.textContent === (valid ? "Saved" : "Value is required"), "real enhanced form result");
          await waitFor(() => !d.querySelector('[aria-busy="true"]'), "real enhanced form completion");
        } else {
          await waitFor(() => w.document !== d && !!w.document.querySelector("#native-status"), "native HTTP form navigation");
        }
        // Give unintended replays time to arrive before reading server counters.
        await pause(30);
        const current = w.document;
        const invalid = current.querySelector('[name="value"]');
        const description = invalid?.getAttribute("aria-describedby");
        return {
          server: await state(id), runtimeLoaded, navigationAvailable,
          navigated: current !== d, before, location: w.location.href,
          historyDelta: w.history.length - oldHistory,
          contentType: current.contentType,
          status: current.querySelector(enhanced ? "#nested" : "#native-status")?.textContent,
          validationAccessible: invalid?.getAttribute("aria-invalid") === "true" &&
            !!description && current.getElementById(description)?.textContent === "Value is required",
          diagnostics: enhanced ? w.observed.diagnostics : []
        };
      }, fingerprint, (w) => {
        if (typeof w.nativeFetch !== "function") throw new Error("probe must retain nativeFetch before installing its stub");
        w.fetch = w.nativeFetch;
        if (mode === "missing-navigation") {
          Object.defineProperty(w, "navigation", {value: undefined, configurable: true});
        }
      }, {runtime: mode !== "no-runtime"});
    }
  }
}
