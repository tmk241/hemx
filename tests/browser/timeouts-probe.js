// Real fetch and browser timers; only observation wraps fetch, never its result.
async function observeTimeouts({ observe, waitFor, pause }) {
  const setup = (w, d) => {
    w.timeoutProbe = { errors: [], headersSeen: false };
    w.fetch = async (...args) => {
      const response = await w.nativeFetch(...args);
      w.timeoutProbe.headersSeen = true;
      return response;
    };
    d.querySelector("#owner").addEventListener("hemx:error", event => {
      w.timeoutProbe.errors.push(event.detail);
      d.querySelector("#outlet").textContent = "Zeit abgelaufen; Zustand prüfen";
    });
  };
  const markup = (id, kind, timeout, standalone = false, policy = "drop") => {
    const attribute = timeout === null ? "" : `data-hemx-timeout="${timeout}"`;
    const controls = `<input id="value" name="value" value="unsaved"><button id="trigger" data-hemx-timeout="1ms">Save</button><input id="file" type="file" name="file">`;
    return `<main id="owner" data-hemx-root data-hemx-timeout="1ms">
      ${standalone ? `<input id="value" value="unsaved"><button id="trigger" type="button" data-hemx-action="7" ${attribute}>Save</button>`
        : `<form id="form" action="/timeouts/${id}/${kind}" method="post" enctype="multipart/form-data" data-hemx-action="7" data-hemx-policy="${policy}" ${attribute} ${policy === "drop" ? "data-hemx-disable-while-pending" : ""}>${controls}</form>`}
      <p id="content" data-hemx-resource="1">initial</p><p id="outlet" data-hemx-error hidden></p>
      <p data-hemx-indicator hidden>Waiting</p></main>`;
  };
  const state = async (w, id) => (await w.nativeFetch(`/timeouts/${id}/state`)).json();
  async function started(w, id) {
    const deadline = performance.now() + 2000;
    while ((await state(w, id)).requests === 0) {
      if (performance.now() >= deadline) throw new Error("HTTP request did not reach server");
      await pause(5);
    }
  }
  const snapshot = async (w, d, id) => ({
    errors: [...w.timeoutProbe.errors], headersSeen: w.timeoutProbe.headersSeen,
    state: await state(w, id), content: d.querySelector("#content").textContent,
    busy: !!d.querySelector('[aria-busy="true"]'), disabled: !!d.querySelector(":disabled"),
    value: d.querySelector("#value").value, outlet: d.querySelector("#outlet").textContent,
    fileCount: d.querySelector("#file")?.files.length || 0,
  });
  for (const [id, kind, timeout] of [
    ["long", "long", "15s"], ["local", "slow", null], ["standalone", "slow", "2s"],
    ["queue", "queue", "2s"], ["bare", "quick", "1000"], ["ms", "quick", " 1000ms "],
    ["seconds", "quick", "1s"], ["whitespace", "quick", "\uFEFF1000ms\uFEFF"], ["max", "quick", "2147483647ms"], ["max-seconds", "quick", "2147483s"],
    ["short", "slow", "250ms"], ["body", "body", "250ms"], ["upload", "upload", "250ms"],
  ]) {
    const fails = ["short", "body", "upload"].includes(id);
    await observe(`timeouts-${id}`, markup(id, kind, timeout, id === "standalone", id === "queue" ? "queue" : "drop"), async (w, d) => {
      if (id === "standalone") w.history.replaceState(null, "", `/timeouts/${id}/${kind}`);
      if (id === "upload") {
        const files = new w.DataTransfer();
        files.items.add(new w.File([new w.Uint8Array(16 * 1024 * 1024)], "upload.bin"));
        d.querySelector("#file").files = files.files;
      }
      const start = performance.now();
      d.querySelector("#trigger").click();
      if (id === "queue") {
        await started(w, id);
        const form = d.querySelector("#form");
        form.setAttribute("data-hemx-timeout", "300ms");
        // A listener registered after the runtime observes actual admission.
        const admitted = new Promise(resolve => w.navigation.addEventListener("navigate", event => {
          if (event.defaultPrevented) resolve();
        }, { once: true }));
        d.querySelector("#trigger").click();
        await admitted;
        // Queued requests retain the captured policy, just like submitted data.
        form.setAttribute("data-hemx-timeout", "invalid-after-admission");
      }
      await waitFor(() => fails ? w.timeoutProbe.errors.length === 1
        : d.querySelector("#content").textContent === `complete ${id === "queue" ? 2 : 1}`,
        `${id} result`, id === "long" ? 13000 : 2500);
      await waitFor(() => !d.querySelector('[aria-busy="true"]'), `${id} cleanup`);
      await pause(40);
      return { ...await snapshot(w, d, id), elapsed: performance.now() - start };
    }, "0", setup);
  }
  await observe("timeouts-invalid", markup("invalid", "quick", "1s"), async (w, d) => {
    const rows = [];
    for (const value of ["", "0", "0ms", "0s", "-1", "1.5s", "none", "10MS", "1 s", "1e3", "2147483648", "2147484s", "99999999999999999999999s", "\u00851000ms", "1000ms\u0085"]) {
      d.querySelector("#form").setAttribute("data-hemx-timeout", value);
      w.timeoutProbe.errors = [];
      d.querySelector("#trigger").click();
      await waitFor(() => w.timeoutProbe.errors.length === 1, `invalid timeout ${value}`);
      rows.push({ value, ...await snapshot(w, d, "invalid") });
    }
    return rows;
  }, "0", setup);
  await observe("timeouts-invalid-latest", markup("invalid-latest", "slow", "2s", false, "latest"), async (w, d) => {
    d.querySelector("#trigger").click();
    await started(w, "invalid-latest");
    d.querySelector("#form").setAttribute("data-hemx-timeout", "0");
    d.querySelector("#trigger").click();
    await waitFor(() => w.timeoutProbe.errors.length === 1, "invalid replacement rejected");
    await waitFor(() => d.querySelector("#content").textContent === "complete 1", "original request survives");
    await waitFor(() => !d.querySelector('[aria-busy="true"]'), "original request cleanup");
    return snapshot(w, d, "invalid-latest");
  }, "0", setup);
}
