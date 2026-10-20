// Real HTTP failures and synchronous application-owned translations. No fetch
// replacement: the application error must pass through applyResponse and send.
async function observeErrors({ observe, cases, waitFor, pause }) {
  const translation = "Fehler <b>nicht wiederholen</b>";
  for (const [transport, kinds] of [
    ["post", ["batch", "decode", "http", "stale", "timeout"]],
    ["navigate", ["batch"]],
    ["public", ["batch", "decode", "abi", "stale"]],
    ["sse", ["batch"]],
  ]) {
    for (const kind of kinds) {
      const id = `${transport}-${kind}`;
      const url = `/errors/${id}/${transport === "sse" ? "sse" : kind}`;
      const trigger = transport === "navigate"
        ? `<a id="trigger" href="${url}" data-hemx-nav>Open</a>`
        : `<form action="${url}" method="post" data-hemx-action="7"><button id="trigger">Save</button></form>`;
      const markup = `<main id="owner" data-hemx-root ${transport === "sse" ? `data-hemx-sse="${url}"` : ""}>
        ${trigger}<p id="content" data-hemx-resource="1">initial</p>
        <p id="outlet" data-hemx-error hidden></p>
        <section id="nested" data-hemx-root><p data-hemx-error>nested</p></section>
        <div data-hemx-island="editor"><p data-hemx-error>island</p></div>
      </main><aside id="sibling" data-hemx-root><p data-hemx-error>sibling</p></aside>`;
      await observe(`errors-${id}`, markup, async (w, d) => {
        const owner = d.querySelector("#owner"), outlet = d.querySelector("#outlet");
        const history = w.history.length;
        let accepted, nextAccepted;
        if (transport === "public") {
          accepted = w.hemx.applyBatch(new w.Uint8Array(cases.errors[kind]).buffer, owner);
          nextAccepted = w.hemx.applyBatch(new w.Uint8Array(cases.errors.recovery).buffer, owner);
        } else if (transport !== "sse") d.querySelector("#trigger").click();
        // Exercise the real ten-second request deadline, not a substituted
        // clock or fetch rejection. Other cases keep their two-second bound.
        await waitFor(() => w.errorProbe.events.length === 1, "localized error event", kind === "timeout" ? 12000 : 2000);
        if (transport === "sse" || transport === "public") {
          await waitFor(() => w.errorProbe.completed, "independent batch after failure");
        }
        await waitFor(() => !d.querySelector('[aria-busy="true"]'), "failed request cleanup");
        // Drain the asynchronous response/catch chain and allow any accidental
        // replay to reach the real server before inspecting its counter.
        await pause(40);
        const response = transport === "public" ? null : await w.nativeFetch(`/errors/${id}/state`);
        return {
          ...w.errorProbe, accepted, nextAccepted,
          text: outlet.textContent, markupChildren: outlet.children.length,
          foreign: ["#nested [data-hemx-error]", "[data-hemx-island] [data-hemx-error]", "#sibling [data-hemx-error]"].map(s => d.querySelector(s).textContent),
          content: d.querySelector("#content").textContent,
          requests: response ? await response.json() : 0,
          sameDocument: w.document === d, historyDelta: w.history.length - history,
        };
      }, "0", (w, d) => {
        w.fetch = w.nativeFetch;
        w.errorProbe = { events: [], foreignEvents: 0, completed: false };
        d.addEventListener("hemx:error", event => {
          const owner = d.querySelector("#owner");
          // Applications must check target ownership: nested-root events bubble.
          if (event.target !== owner) { w.errorProbe.foreignEvents++; return; }
          const outlet = d.querySelector("#outlet");
          w.errorProbe.events.push({
            owner: event.target.id, code: event.detail.code, status: event.detail.status,
            message: event.detail.message, defaultText: outlet.textContent, hidden: outlet.hidden,
          });
          outlet.textContent = translation;
        });
        d.addEventListener("error-probe-done", () => { w.errorProbe.completed = true; });
      });
    }
  }
}
