// Admission-specific native contracts, separate from transport/fallback tests.
async function observeFormAdmission({observe, root, waitFor, pause}) {
  for (const name of ["late-navigate-veto", "manual-data", "manual-data-canceled", "multipart", "image-get", "no-precommit"]) {
    const image = name === "image-get";
    const markup = root(`<form action="/admission" method="${image ? "get" : "post"}" data-hemx-action="7"><input name="value" value="native"><input type="hidden" name="__h" value="7">${name === "multipart" ? '<input id="file" type="file" name="attachment">' : ''}${image ? '<input type="image" name="pick" data-hemx-action="8" alt="Send">' : '<button name="choice" value="save" data-hemx-action="8">Send</button>'}</form>`);
    await observe(`form-admission-${name}`, markup, async (w, d) => {
      const form = d.querySelector("form");
      const source = d.querySelector('button,input[type="image"]');
      const before = { url: w.location.href, history: w.history.length, state: JSON.stringify(w.history.state) };
      let dataEvents = 0, vetoes = 0;
      form.addEventListener("formdata", event => { dataEvents++; event.formData.append("hook", "once"); });
      if (name.startsWith("manual-data")) form.addEventListener("submit", () => { new w.FormData(form); });
      if (name === "manual-data-canceled") d.addEventListener("submit", event => { vetoes++; event.preventDefault(); });
      if (name === "late-navigate-veto") w.navigation.addEventListener("navigate", event => { vetoes++; event.preventDefault(); });
      if (name === "multipart") {
        source.setAttribute("formenctype", "multipart/form-data");
        const transfer = new w.DataTransfer();
        transfer.items.add(new w.File(["file-proof"], "proof.txt", {type: "text/plain"}));
        d.querySelector("#file").files = transfer.files;
      }
      source.click();
      if (name === "manual-data-canceled") await pause();
      else {
        await waitFor(() => w.observed.requests.length === 1, `admitted ${name}`);
        await waitFor(() => !form.hasAttribute("aria-busy"), `completed ${name}`);
        if (image) await waitFor(() => w.location.pathname === "/admission", "image GET history");
      }
      return {dataEvents, vetoes, requests: w.observed.requests, before, url: w.location.href,
        history: w.history.length, state: JSON.stringify(w.history.state), diagnostics: w.observed.diagnostics,
        precommit: typeof w.NavigationPrecommitController};
    }, "0", name === "no-precommit" ? w => {
      Object.defineProperty(w, "NavigationPrecommitController", {value: undefined, configurable: true});
    } : undefined);
  }
}
