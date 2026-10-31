// Real fetch, timers and server ledger; observation never changes a request.
async function observeLedger({ observe, waitFor, pause }) {
  const setup = (w, d) => {
    w.ledgerProbe = { errors: [] };
    // Real network: undo the fixture's default fake fetch.
    w.fetch = (...args) => w.nativeFetch(...args);
    d.querySelector("#owner").addEventListener("hemx:error", event => {
      w.ledgerProbe.errors.push(event.detail);
      d.querySelector("#outlet").textContent = "Nicht bestätigt";
    });
  };
  const markup = (mode, id) => `<main id="owner" data-hemx-root>
      <form id="form" action="/ledger/${mode}/${id}" method="post" data-hemx-action="7" data-hemx-timeout="250ms" data-hemx-disable-while-pending>
        <input type="hidden" name="__h" value="7"><input type="hidden" name="__b" value="0">
        <input type="hidden" name="case" value="${id}"><input id="value" name="value" value="unsaved">
        <button id="trigger">Save</button>
      </form>
      <p id="content" data-hemx-resource="1">initial</p><p id="outlet" data-hemx-error hidden></p></main>`;
  const state = async (w, mode, id) => (await w.nativeFetch(`/ledger/${mode}/${id}/state`)).json();
  for (const [mode, id] of [["kept", "lost"], ["kept", "running"], ["plain", "none"]]) {
    await observe(`ledger-${id}`, markup(mode, id), async (w, d) => {
      // A first, quick submission lets the page learn whether the action keeps a ledger.
      d.querySelector("#trigger").click();
      await waitFor(() => d.querySelector("#content").textContent === "saved 1", `${id} first submission`);
      await waitFor(() => !d.querySelector('[aria-busy="true"]'), `${id} first cleanup`);
      d.querySelector("#trigger").click();
      const expected = { lost: () => d.querySelector("#content").textContent === "saved 2",
        running: () => w.ledgerProbe.errors.length === 2, none: () => w.ledgerProbe.errors.length === 1 }[id];
      await waitFor(expected, `${id} outcome`, 3000);
      await waitFor(() => !d.querySelector('[aria-busy="true"]'), `${id} cleanup`);
      // Let every server-side run and any (wrong) further resend finish.
      await pause(1200);
      return {
        errors: [...w.ledgerProbe.errors], state: await state(w, mode, id),
        content: d.querySelector("#content").textContent, outlet: d.querySelector("#outlet").textContent,
        busy: !!d.querySelector('[aria-busy="true"]'), disabled: !!d.querySelector(":disabled"),
        value: d.querySelector("#value").value,
      };
    }, "0", setup);
  }
}
