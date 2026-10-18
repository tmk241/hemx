// Browser-side observations. Rust assertions own the expected outcomes.
(async () => {
  const results = {};
  const pause = (ms = 0) => new Promise(resolve => setTimeout(resolve, ms));
  async function waitFor(predicate, description) {
    const deadline = performance.now() + 2000;
    while (!predicate()) {
      if (performance.now() >= deadline) throw new Error(`timed out waiting for ${description}`);
      await pause(5);
    }
  }
  const root = content => `<main data-hemx-root data-hemx-build="0">${content}</main>`;
  let cases;

  async function fixture(markup, run, fingerprint = "0", beforeRuntime, options = {}) {
    const frame = document.createElement("iframe");
    // Use a real same-origin document: srcdoc would require replacing native
    // history APIs and could conceal GET navigation failures.
    try {
      await new Promise((resolve, reject) => {
        const deadline = setTimeout(() => reject(new Error("frame load timeout")), 5000);
        frame.onload = () => { clearTimeout(deadline); resolve(); };
        frame.src = "/frame";
        document.body.append(frame);
      });
      const w = frame.contentWindow;
      const d = frame.contentDocument;
      d.body.innerHTML = markup;
      // Expected contract identity is supplied by the Rust producer, as it is
      // by the transport in an application; IDs are not copied from a build.
      for (const owner of d.querySelectorAll("[data-hemx-root]")) w.Element.prototype.setAttribute.call(owner, "data-hemx-build", fingerprint);
      const reply = fingerprint === cases.generatedFingerprint ? cases.wire["generated-empty"] : cases.wire.empty;
      const setup = d.createElement("script");
      setup.textContent = `
        window.observed = { requests: [], credentials: [], errors: [], diagnostics: [], bindings: {} };
        window.nativeFetch = window.fetch.bind(window);
        window.addEventListener("error", e => observed.errors.push(e.message));
        window.addEventListener("unhandledrejection", e => observed.errors.push(String(e.reason)));
        document.addEventListener("hemx:error", e => observed.diagnostics.push(e.detail?.message || String(e.detail)));
        document.addEventListener("hemx:missing-target", e => observed.diagnostics.push("missing target"));
        const addListener = EventTarget.prototype.addEventListener;
        EventTarget.prototype.addEventListener = function(type, listener, options) {
          let key;
          if (this instanceof Element && Element.prototype.hasAttribute.call(this, "data-hemx-root")) {
            key = (Element.prototype.getAttribute.call(this, "id") || Object.getOwnPropertyDescriptor(Element.prototype, "tagName").get.call(this)) + ":" + type;
          }
          if (this === window && type === "submit") key = "window:submit";
          if (key) observed.bindings[key] = (observed.bindings[key] || 0) + 1;
          return addListener.call(this, type, listener, options);
        };
        window.fetch = async (url, options = {}) => {
          const entries = options.body ? Array.from(options.body.entries(), ([name, value]) =>
            [name, value instanceof File ? { name: value.name, size: value.size, type: value.type } : value]) : [];
          observed.requests.push({ url: String(url), method: options.method || "GET", entries, headers: options.headers });
          observed.credentials.push(options.credentials);
          return new Response(new Uint8Array(${JSON.stringify(reply)}), {
            headers: { "Content-Type": "application/hemx" }
          });
        };
      `;
      d.head.append(setup);
      if (beforeRuntime) await beforeRuntime(w, d);
      if (options.runtime !== false) await new Promise((resolve, reject) => {
        const script = d.createElement("script");
        const deadline = setTimeout(() => reject(new Error("runtime load timeout")), 5000);
        script.onload = () => { clearTimeout(deadline); resolve(); };
        script.onerror = () => { clearTimeout(deadline); reject(new Error("runtime load failed")); };
        script.src = "/hemx.js";
        d.head.append(script);
      });
      const observed = w.observed;
      const value = await run(w, d);
      if (observed.errors.length) throw new Error(observed.errors.join("; "));
      return value;
    } finally {
      frame.remove();
    }
  }

  function apply(w, d, name) {
    const wire = cases.wire[name];
    if (!wire) throw new Error(`missing Rust wire case: ${name}`);
    return w.hemx.applyBatch(new w.Uint8Array(wire).buffer, d.querySelector("[data-hemx-root]"));
  }

  async function observe(name, markup, run, fingerprint = "0", beforeRuntime, options = {}) {
    if (Object.hasOwn(results, name)) throw new Error(`duplicate browser case: ${name}`);
    document.getElementById("results").textContent = `Running ${name} (${Object.keys(results).length} completed)…`;
    try { results[name] = await fixture(markup, run, fingerprint, beforeRuntime, options); }
    catch (error) { results[name] = { probeError: String(error) }; }
  }

  try {
    const response = await fetch("/cases");
    if (!response.ok) throw new Error(`case producer returned ${response.status}`);
    cases = await response.json();

    await observe("runtime-entrypoint", root('<p id="entry">unchanged</p>'), (w, d) => ({
      abi: w.hemx.runtimeAbiVersion,
      keys: Object.keys(w.hemx).sort(),
      frozen: Object.isFrozen(w.hemx),
      roots: w.hemx.roots().length,
      owner: w.hemx.rootOf(d.querySelector("#entry")) === d.querySelector("main"),
      bindingReadiness: w.entryReadiness,
      before: w.entryBefore,
      after: {url: w.location.href, history: w.history.length, state: JSON.stringify(w.history.state), html: d.body.innerHTML},
      requests: w.observed.requests.length,
      storage: w.localStorage.length + w.sessionStorage.length,
      diagnostics: w.observed.diagnostics,
    }), "0", (w, d) => {
      w.entryBefore = {url: w.location.href, history: w.history.length, state: JSON.stringify(w.history.state), html: d.body.innerHTML};
      w.entryReadiness = [];
      const add = w.EventTarget.prototype.addEventListener;
      w.EventTarget.prototype.addEventListener = function(...args) {
        if (this instanceof w.Element && this.hasAttribute("data-hemx-root")) {
          w.entryReadiness.push(typeof w.hemx?.decodeBatch === "function");
        }
        return Reflect.apply(add, this, args);
      };
    });

    await observe("effect-focus-control", root('<div style="height:3000px"></div><input id="target" data-hemx-resource="1">'),
      (w, d) => ({ accepted: apply(w, d, "focus-control"), focused: d.activeElement.id, scrollY: w.scrollY, diagnostics: w.observed.diagnostics }));
    await observe("effect-focus-radio", root('<form data-hemx-resource="2"><input id="first" type="radio" name="choice" value="a"><input id="checked" type="radio" name="choice" value="b" checked><input id="disabled" type="radio" name="choice" value="c" disabled></form>'),
      (w, d) => ({ accepted: apply(w, d, "focus-radio"), focused: d.activeElement.id, diagnostics: w.observed.diagnostics }));
    await observe("effect-scroll", root('<input id="focus"><div style="height:2000px"></div><p id="target" data-hemx-resource="1">target</p><div style="height:2000px"></div>'),
      (w, d) => {
        d.querySelector("#focus").focus({ preventScroll: true });
        const accepted = apply(w, d, "scroll-native");
        // Allow sub-pixel scroll rounding; a missed scroll is thousands of pixels.
        const aligned = Math.abs(d.querySelector("#target").getBoundingClientRect().top) <= 1;
        return { accepted, aligned, focused: d.activeElement.id, diagnostics: w.observed.diagnostics };
      });
    await observe("effect-visit", root('<p data-hemx-resource="1">before</p>'), async (w, d) => {
      const history = w.history.length;
      const accepted = apply(w, d, "visit-push");
      await waitFor(() => w.location.pathname === "/effects-visit", "Visit history update");
      return { accepted, path: w.location.pathname, historyDelta: w.history.length - history, requests: w.observed.requests, credentials: w.observed.credentials, diagnostics: w.observed.diagnostics };
    });
    await observe("effect-batch-prefix", root('<p data-hemx-resource="1">before</p>'),
      (w, d) => ({ accepted: apply(w, d, "batch-prefix-failure"), text: d.querySelector("p").textContent, diagnostics: w.observed.diagnostics }));

    const abc = '<ul data-hemx-resource="1" data-hemx-collection=""><li id="a" data-hemx-key="a">A</li><li id="b" data-hemx-key="b">B</li><li id="c" data-hemx-key="c">C</li></ul>';
    for (const [name, markup] of [
      ["rows-reorder", abc], ["rows-declare", abc], ["rows-missing", abc],
      ["delta-apply", abc], ["delta-skip", abc], ["delta-drift", abc],
      ["delta-reverse", `<ul data-hemx-resource="1" data-hemx-collection="">${Array.from({ length: 20 }, (_, n) => `<li id="k${n}" data-hemx-key="k${n}">${n}</li>`).join("")}</ul>`],
      ["delta-unicode", '<ul data-hemx-resource="1" data-hemx-collection=""><li id="t" data-hemx-key="todo-7">T</li><li id="u" data-hemx-key="\u00e4">U</li></ul>'],
      ["rows-unkeyed-child", abc.replace('<li id="c" data-hemx-key="c">C</li>', '<li id="c">C</li>')],
    ]) {
      await observe(name, root(markup), (w, d) => {
        const list = d.querySelector("ul");
        const before = new Map(Array.from(list.children, node => [node.id, node]));
        const observer = new w.MutationObserver(() => {});
        observer.observe(list, { childList: true, subtree: true, attributes: true, characterData: true });
        const accepted = apply(w, d, name);
        const mutations = observer.takeRecords().length;
        observer.disconnect();
        return {
          accepted, mutations, diagnostics: w.observed.diagnostics.length,
          ids: Array.from(list.children, node => node.id),
          keys: Array.from(list.children, node => node.getAttribute("data-hemx-key")),
          text: list.textContent,
          retained: Array.from(list.children, node => before.get(node.id) === node),
        };
      });
    }

    // Deltas in sequence: each starts from the order the previous one produced.
    // After a Patch replaced the rows, the next delta starts from the new rows.
    for (const [name, steps] of [
      ["delta-sequence", ["delta-apply", "delta-step2"]],
      ["delta-invalidated", ["delta-apply", "patch-rows-x", "delta-from-x"]],
    ]) {
      await observe(name, root(abc), (w, d) => ({
        accepted: steps.map((step) => apply(w, d, step)),
        ids: Array.from(d.querySelector("ul").children, node => node.id),
        diagnostics: w.observed.diagnostics.length,
      }));
    }

    // A row whose tag changes is a new element; the row placed next to it must
    // still find it (, ).
    const lone = '<ul data-hemx-resource="1" data-hemx-collection=""><li id="b" data-hemx-key="b">B</li></ul>';
    for (const name of ["rows-retag", "delta-retag"]) {
      await observe(name, root(lone), (w, d) => {
        const accepted = apply(w, d, name);
        const list = d.querySelector("ul");
        return {
          accepted, diagnostics: w.observed.diagnostics.length,
          tags: Array.from(list.children, node => `${node.localName}#${node.id}`),
          text: list.textContent,
        };
      });
    }

    // One unusable row among valid ones: nothing may change at all ().
    for (const name of ["rows-preflight", "delta-preflight"]) {
      await observe(name, root(abc), (w, d) => {
        const list = d.querySelector("ul");
        const observer = new w.MutationObserver(() => {});
        observer.observe(list, { childList: true, subtree: true, attributes: true, characterData: true });
        const accepted = apply(w, d, name);
        const mutations = observer.takeRecords().length;
        observer.disconnect();
        return {
          accepted, mutations, diagnostics: w.observed.diagnostics.length,
          ids: Array.from(list.children, node => node.id), text: list.textContent,
        };
      });
    }

    // A leading U+FEFF belongs to the key; decoding must not drop it ().
    await observe("delta-bom", root('<ul data-hemx-resource="1" data-hemx-collection=""><li id="x" data-hemx-key="\ufeffx">X</li><li id="a" data-hemx-key="a">A</li></ul>'), (w, d) => {
      const accepted = apply(w, d, "delta-bom");
      return { accepted, diagnostics: w.observed.diagnostics.length, ids: Array.from(d.querySelector("ul").children, node => node.id) };
    });

    // Only a delta that applied clears the heal guard: not a skipped duplicate, and
    // not a refetch that finishes after one did ().
    const healedPage = (ids) => root(`<ul data-hemx-resource="1" data-hemx-collection="">${ids.map(id => `<li id="${id}" data-hemx-key="${id}">${id.toUpperCase()}</li>`).join("")}</ul>`);
    await observe("delta-heal-skip", root(abc), async (w, d) => {
      apply(w, d, "delta-drift");
      await pause(200);
      apply(w, d, "delta-skip"); // a duplicate at its result applies nothing
      apply(w, d, "delta-drift");
      await pause(200);
      return { requests: w.healRequests, diagnostics: w.observed.diagnostics.length };
    }, "0", (w) => {
      w.healRequests = [];
      w.fetch = async () => {
        w.healRequests.push("GET");
        return new w.Response(healedPage(["a", "b", "c"]), { headers: { "Content-Type": "text/html" } });
      };
    });
    await observe("delta-heal-applied", root(abc), async (w, d) => {
      apply(w, d, "delta-drift"); // the refetch starts and is held open
      const applied = apply(w, d, "delta-move-last"); // a delta applies meanwhile
      w.release();
      await pause(200);
      apply(w, d, "delta-drift");
      await pause(200);
      return { applied, requests: w.healRequests, diagnostics: w.observed.diagnostics.length };
    }, "0", (w) => {
      w.healRequests = [];
      const gate = new Promise((resolve) => { w.release = resolve; });
      w.fetch = async () => {
        w.healRequests.push("GET");
        await gate;
        return new w.Response(healedPage(["b", "c", "a"]), { headers: { "Content-Type": "text/html" } });
      };
    });

    // A drifted client refetches its page once (). The page either
    // brings the missed rows, or, if it still disagrees, the next drift is
    // reported instead of refetched again.
    for (const [name, healed] of [["delta-heal", true], ["delta-heal-repeat", false]]) {
      const page = healed
        ? root('<ul data-hemx-resource="1" data-hemx-collection=""><li id="x" data-hemx-key="x">X</li><li id="y" data-hemx-key="y">Y</li></ul>')
        : root(abc);
      await observe(name, root(abc), async (w, d) => {
        let resyncs = 0;
        d.addEventListener("hemx:resync", () => resyncs++);
        apply(w, d, "delta-drift");
        await pause(200); // The refetch resolves within the fixture.
        const afterHeal = healed ? apply(w, d, "delta-from-x") : (apply(w, d, "delta-drift"), null);
        await pause(50);
        return {
          ids: Array.from(d.querySelector("ul").children, node => node.id),
          requests: w.healRequests, resyncs, afterHeal,
          diagnostics: w.observed.diagnostics.length,
        };
      }, "0", (w) => {
        // Replace the harness reply before the runtime captures fetch.
        w.healRequests = [];
        w.fetch = async (url, options = {}) => {
          w.healRequests.push(options.method || "GET");
          return new w.Response(page, { headers: { "Content-Type": "text/html" } });
        };
      });
    }

    // Moving row a to the end, natively or (without moveBefore, as in Safari)
    // by reinsertion, which is allowed only when row a holds no browser state
    // that reinsertion would lose (). Both the delta path and the
    // morph path decide the same way.
    const rowA = {
      plain: '<li id="a" data-hemx-key="a"><input value="server"></li>',
      focused: '<li id="a" data-hemx-key="a"><input value="server"></li>',
      selected: '<li id="a" data-hemx-key="a">Selected text</li>',
      media: '<li id="a" data-hemx-key="a"><video></video></li>',
      selectednode: '<li id="a" data-hemx-key="a">Selected node</li>',
    };
    const rest = '<li id="b" data-hemx-key="b">B</li><li id="c" data-hemx-key="c">C</li>';
    for (const [name, wire, state, unavailable] of [
      ["delta-move-native", "delta-move-last", "plain", false],
      ["delta-move-reinserted", "delta-move-last", "plain", true],
      ["delta-move-focused", "delta-move-last", "focused", true],
      ["delta-move-selected", "delta-move-last", "selected", true],
      ["delta-move-media", "delta-move-last", "media", true],
      // Range.selectNode covers the row with both ends in its parent.
      ["delta-move-selectednode", "delta-move-last", "selectednode", true],
      ["morph-move-reinserted", "patch-rows-bca", "plain", true],
      ["morph-move-focused", "patch-rows-bca", "focused", true],
      ["morph-move-stateful", "patch-rows-ba", "focused", true],
    ]) {
      const others = wire === "patch-rows-ba" ? '<li id="b" data-hemx-key="b"><video></video></li>' : rest;
      await observe(name, root(`<ul data-hemx-resource="1" data-hemx-collection="">${rowA[state]}${others}</ul>`), (w, d) => {
        const list = d.querySelector("ul");
        const first = list.firstElementChild;
        const input = first.querySelector("input");
        if (input) input.value = "unsent edit";
        if (state === "focused") input.focus();
        if (state === "selected") w.getSelection().selectAllChildren(first);
        if (state === "selectednode") {
          const range = d.createRange();
          range.selectNode(first);
          w.getSelection().removeAllRanges();
          w.getSelection().addRange(range);
        }
        const observer = new w.MutationObserver(() => {});
        observer.observe(list, { childList: true, subtree: true, attributes: true, characterData: true });
        const accepted = apply(w, d, wire);
        const mutations = observer.takeRecords().length;
        observer.disconnect();
        return {
          accepted, mutations, diagnostics: w.observed.diagnostics.length,
          ids: Array.from(list.children, node => node.id),
          same: list.lastElementChild === first,
          value: input ? input.value : null,
          focused: input ? d.activeElement === input : null,
        };
      }, "0", unavailable ? (w) => {
        for (const prototype of [w.Element.prototype, w.DocumentFragment.prototype, w.Document.prototype]) {
          Object.defineProperty(prototype, "moveBefore", { value: undefined, configurable: true });
        }
      } : undefined);
    }

    for (const nested of [false, true]) {
      const island = '<div data-hemx-island="editor"><input value="server"></div>';
      await observe(`island-${nested ? "nested" : "top"}`, root(`<section data-hemx-resource="1">${nested ? `<article>${island}</article>` : island}</section>`),
        (w, d) => {
          const old = d.querySelector("[data-hemx-island]");
          old.firstChild.value = "client edit";
          apply(w, d, `island-${nested ? "nested" : "top"}`);
          return { same: d.querySelector("[data-hemx-island]") === old, connected: old.isConnected, value: old.firstChild.value };
        });
    }

    for (const mode of ["unchanged", "changed", "replace"]) {
      await observe(`input-${mode}`, root('<section data-hemx-resource="1"><input id="edit" value="server"></section>'),
        (w, d) => {
          const old = d.querySelector("input");
          old.value = "unsent edit";
          old.focus();
          old.setSelectionRange(2, 5, "backward");
          const beforeFocused = d.activeElement === old;
          apply(w, d, `input-${mode}`);
          const current = d.querySelector("input");
          return {
            beforeFocused, same: old === current, focused: d.activeElement === current,
            value: current.value, defaultValue: current.defaultValue,
            selection: [current.selectionStart, current.selectionEnd, current.selectionDirection],
            invalid: current.getAttribute("aria-invalid")
          };
        });
    }

    for (const unavailable of [false, true]) {
      await observe(`move-focus-${unavailable ? "unavailable" : "native"}`, root('<ul data-hemx-resource="1" data-hemx-collection=""><li data-hemx-key="a"><input value="server"></li><li data-hemx-key="b">B</li><li data-hemx-key="c">C</li></ul>'),
        (w, d) => {
          const input = d.querySelector("input");
          input.value = "unsent edit";
          input.focus();
          input.setSelectionRange(2, 5, "backward");
          const beforeFocused = d.activeElement === input;
          const section = d.querySelector("ul");
          const observer = new w.MutationObserver(() => {});
          observer.observe(section, { childList: true, subtree: true, attributes: true, characterData: true });
          apply(w, d, "rows-move-last");
          const mutations = observer.takeRecords().length;
          observer.disconnect();
          return {
            beforeFocused, same: d.querySelector("input") === input, value: input.value,
            focused: d.activeElement === input,
            selection: [input.selectionStart, input.selectionEnd, input.selectionDirection],
            moved: section.lastElementChild.contains(input), mutations,
            capability: typeof w.Element.prototype.moveBefore,
            diagnostics: w.observed.diagnostics
          };
        }, "0", unavailable ? (w) => {
          // Remove the capability before the shipped runtime captures natives.
          // This verifies fail-closed behavior, not a simulated movement fallback.
          for (const prototype of [w.Element.prototype, w.DocumentFragment.prototype, w.Document.prototype]) {
            Object.defineProperty(prototype, "moveBefore", { value: undefined, configurable: true });
          }
        } : undefined);
    }

    for (const nested of [false, true]) {
      await observe(`island-events-${nested ? "nested-root" : "owned"}`,
        root(`<div data-hemx-island="widget">${nested ? '<aside data-hemx-root data-hemx-build="0">' : ""}<button type="button" data-hemx-action="7">Action</button>${nested ? "</aside>" : ""}</div>`),
        async (w, d) => { d.querySelector("button").click(); await pause(); return w.observed.requests.length; });
    }

    await observe("moved-root", root('<ul data-hemx-resource="1" data-hemx-collection=""><li data-hemx-key="a"><aside id="nested" data-hemx-root data-hemx-build="0"><button type="button" data-hemx-action="7">Action</button></aside></li><li data-hemx-key="b">B</li><li data-hemx-key="c">C</li></ul>'),
      async (w, d) => {
        const button = d.querySelector("button");
        button.click();
        await pause();
        const before = w.observed.requests.length;
        apply(w, d, "rows-move-last");
        await pause();
        button.click();
        await pause();
        return { before, after: w.observed.requests.length - before, bindings: w.observed.bindings };
      });

    await observe("html-island", root('<section data-hemx-resource="1"><article><div data-hemx-island="editor"><input value="server"></div></article></section>'),
      (w, d) => {
        const island = d.querySelector("[data-hemx-island]");
        island.firstChild.value = "client edit";
        w.hemx.applyHtml('<section data-hemx-resource="1"><article><div data-hemx-island="editor"><input value="replacement"></div></article></section>', d.querySelector("main"));
        return { same: d.querySelector("[data-hemx-island]") === island, value: island.firstChild.value };
      });

    await observe("escaped-text", root(cases.textInitial), (w, d) => {
      const accepted = apply(w, d, "text");
      const target = d.querySelector("#nested");
      return { accepted, text: target.textContent, elements: target.childElementCount };
    }, cases.generatedFingerprint);

    for (const host of [false, true]) {
      for (const changed of [false, true]) {
        const mode = changed ? "changed" : "unchanged";
        await observe(`textarea-${host ? "host" : "inner"}-${mode}`, root('<textarea data-hemx-resource="1">server</textarea>'), (w, d) => {
          const field = d.querySelector("textarea");
          field.value = "unsent edit";
          field.focus();
          field.setSelectionRange(2, 5, "backward");
          const accepted = host
            ? w.hemx.applyHtml(`<textarea data-hemx-resource="1" aria-invalid="true">${changed ? "changed" : "server"}</textarea>`, d.querySelector("main"))
            : apply(w, d, `textarea-${mode}`);
          return { accepted, same: d.querySelector("textarea") === field, value: field.value,
            defaultValue: field.defaultValue, focused: d.activeElement === field,
            selection: [field.selectionStart, field.selectionEnd, field.selectionDirection],
            invalid: field.getAttribute("aria-invalid") };
        });
      }
    }

    for (const variant of ["single", "none", "multiple", "multiple-none", "changed", "reordered", "duplicate-values"]) {
      const multiple = variant.startsWith("multiple");
      const duplicate = variant === "duplicate-values";
      const options = `<option id="a" value="${duplicate ? "same" : "a"}" selected>A</option><option id="b" value="${duplicate ? "same" : "b"}">B</option>`;
      await observe(`select-${variant}`, root(`<select data-hemx-resource="1" ${multiple ? "multiple" : ""}>${options}</select>`), (w, d) => {
        const field = d.querySelector("select");
        const old = Array.from(field.options);
        field.selectedIndex = -1;
        if (!variant.endsWith("none")) old[variant === "changed" ? 0 : 1].selected = true;
        field.focus();
        const accepted = duplicate
          ? w.hemx.applyHtml('<select data-hemx-resource="1"><option id="b" value="same">B</option><option id="a" value="same" selected>A</option></select>', d.querySelector("main"))
          : apply(w, d, `select-${variant === "changed" || variant === "reordered" ? variant : "unchanged"}`);
        return { accepted, same: d.querySelector("select") === field,
          sameOptions: old.every(option => d.getElementById(option.id) === option),
          selected: Array.from(field.selectedOptions, option => option.id),
          defaults: Array.from(field.options).filter(option => option.defaultSelected).map(option => option.id),
          order: Array.from(field.options, option => option.id), focused: d.activeElement === field };
      });
    }

    await observe("file-morph", root('<section data-hemx-resource="1"><input id="upload" type="file"></section>'), async (w, d) => {
      const field = d.querySelector("input");
      const files = new w.DataTransfer();
      files.items.add(new w.File(["local!"], "proof.txt", { type: "text/plain" }));
      field.files = files.files;
      field.focus();
      const accepted = w.hemx.applyHtml('<section data-hemx-resource="1"><input id="upload" type="file" value="ignored-server-value" aria-label="updated"></section>', d.querySelector("main"));
      return { accepted, same: d.querySelector("input") === field, focused: d.activeElement === field,
        count: field.files.length, name: field.files[0]?.name, content: await field.files[0]?.text(),
        label: field.getAttribute("aria-label") };
    });

    for (const changed of [false, true]) {
      await observe(`radio-${changed ? "changed" : "unchanged"}`, root('<section data-hemx-resource="1"><form><input id="a" type="radio" name="group" checked><input id="b" type="radio" name="group"></form></section>'), (w, d) => {
        const old = Array.from(d.querySelectorAll("input"));
        old[1].checked = true;
        const accepted = w.hemx.applyHtml(`<section data-hemx-resource="1"><form><input id="a" type="radio" name="group" ${changed ? "" : "checked"}><input id="b" type="radio" name="group"></form></section>`, d.querySelector("main"));
        return { accepted, same: old.every(field => d.getElementById(field.id) === field),
          checked: old.filter(field => field.checked).map(field => field.id),
          defaults: old.filter(field => field.defaultChecked).map(field => field.id) };
      });
    }

    await observe("radio-ownership", '<form>' + root('<section data-hemx-resource="1"><input id="owned" type="radio" name="group"></section><div data-hemx-island="editor"><input id="foreign" type="radio" name="group" checked></div>') + '</form>', (w, d) => {
      const scope = d.querySelector("main");
      const observer = new w.MutationObserver(() => {});
      observer.observe(scope, { childList: true, attributes: true, subtree: true, characterData: true });
      const accepted = w.hemx.applyHtml('<section data-hemx-resource="1"><input id="owned" type="radio" name="group" checked></section>', scope);
      const mutations = observer.takeRecords().length;
      observer.disconnect();
      return { accepted, mutations, owned: d.querySelector("#owned").checked,
        foreign: d.querySelector("#foreign").checked, diagnostics: w.observed.diagnostics.length };
    });

    for (const [kind, content, external = ""] of [
      ["text", '<input name="field" value="server">'],
      ["textarea", '<textarea name="field">server</textarea>'],
      ["checkbox", '<input name="field" type="checkbox" checked value="a"><input name="field" type="checkbox" checked value="b">'],
      ["radio", '<input name="field" type="radio" checked value="a"><input name="field" type="radio" value="b">'],
      ["select", '<select name="field"><option selected>A</option><option>B</option></select>'],
      ["multiple", '<select name="field" multiple><option selected>A</option><option selected>B</option></select>'],
      ["file", '<input name="field" type="file">'],
      ["external", '<input name="field" value="inside">', '<input name="field" form="controls" value="outside">']
    ]) {
      await observe(`clear-${kind}`, root(`<form id="controls" data-hemx-resource="1">${content}</form>${external}`), (w, d) => {
        const scope = d.querySelector("main");
        const nodes = Array.from(scope.querySelectorAll("*"));
        const fields = Array.from(d.querySelectorAll('[name="field"]'));
        if (kind === "file") {
          const files = new w.DataTransfer();
          files.items.add(new w.File(["local!"], "proof.txt"));
          fields[0].files = files.files;
        }
        const before = scope.innerHTML;
        const accepted = apply(w, d, "clear-field");
        return { accepted, same: nodes.every((node, index) => scope.querySelectorAll("*")[index] === node),
          markupUnchanged: before === scope.innerHTML,
          cleared: fields.every(field => field.type === "checkbox" || field.type === "radio" ? !field.checked : field.value === ""),
          selected: fields.filter(field => field.tagName === "SELECT").map(field => field.selectedIndex),
          files: kind === "file" ? fields[0].files.length : null };
      });
    }

    await observe("clear-ownership", root('<form data-hemx-resource="1"><input name="field" value="keep"><select name="field"><option data-hemx-island selected>A</option><option>B</option></select></form>'), (w, d) => {
      const accepted = apply(w, d, "clear-field");
      return { accepted, value: d.querySelector("input").value, selected: d.querySelector("select").selectedIndex,
        options: d.querySelectorAll("option").length, diagnostics: w.observed.diagnostics.length };
    });

    await observe("throttle", root('<button type="button" data-hemx-action="7" data-hemx-throttle="50ms">Action</button>'),
      async (w, d) => {
        const button = d.querySelector("button");
        button.click(); button.click(); button.click();
        await pause();
        const burst = w.observed.requests.length;
        await pause(80);
        button.click();
        await pause();
        return { burst, afterInterval: w.observed.requests.length };
      });

    await observe("removed-debounce", root('<button type="button" data-hemx-action="7" data-hemx-debounce="30ms">Action</button>'),
      async (w, d) => {
        d.querySelector("button").click();
        d.querySelector("button").remove();
        await pause(80);
        return w.observed.requests.length;
      });

    for (const guard of ["form", "document", "window"]) {
      for (const buttonAction of [false, true]) {
        await observe(`submit-cancel-${guard}-${buttonAction}`, root(`<form action="/submit" method="post" data-hemx-action="7"><input name="value" value="ok"><button ${buttonAction ? 'data-hemx-action="8"' : ""}>Save</button></form>`),
          async (w, d) => {
            const form = d.querySelector("form");
            let submissions = 0;
            (guard === "form" ? form : guard === "document" ? d : w).addEventListener("submit", event => { submissions++; event.preventDefault(); });
            d.querySelector("button").click();
            form.requestSubmit();
            await pause();
            return { submissions, requests: w.observed.requests.length };
          });
      }
    }

    await observe("native-validation", root('<form action="/submit" method="post" data-hemx-action="7"><input name="value" required><button data-hemx-action="8">Save</button></form>'),
      async (w, d) => {
        let invalid = 0;
        d.addEventListener("invalid", () => invalid++, true);
        d.querySelector("button").click();
        await pause();
        return { invalid, requests: w.observed.requests.length };
      });

    await observe("submitter", root('<form id="form" action="/submit" method="post" data-hemx-action="7"><input name="action" value="shadow"><input name="method" value="shadow"><input name="enctype" value="shadow"><input name="target" value="shadow"><input name="empty" required><button name="choice" value="save" formaction="/alternate" formmethod="get" formnovalidate>Save</button></form>'),
      async (w, d) => {
        let invalid = 0;
        let formdata = 0;
        d.addEventListener("invalid", () => invalid++, true);
        d.querySelector("form").addEventListener("formdata", event => { formdata++; event.formData.append("hook", "once"); });
        d.querySelector("button").click();
        await waitFor(() => w.location.pathname !== "/frame" || w.observed.diagnostics.length > 0, "GET form result");
        return { invalid, formdata, requests: w.observed.requests, location: w.location.href, diagnostics: w.observed.diagnostics };
      });

    await observe("named-form-properties", root('<form action="/submit" method="post" data-hemx-action="7"><input name="action" value="action-value"><input name="method" value="method-value"><input name="enctype" value="enctype-value"><input name="target" value="target-value"><button>Save</button></form>'),
      async (w, d) => {
        d.querySelector("button").click();
        await waitFor(() => w.observed.requests.length === 1, "named form request");
        return w.observed.requests;
      });

    const clobberedNames = ["action", "method", "enctype", "target", "attributes", "children", "childNodes",
      "nodeType", "nodeName", "tagName", "localName", "namespaceURI", "ownerDocument", "isConnected",
      "parentNode", "parentElement", "firstChild", "nextSibling", "getAttribute", "hasAttribute", "setAttribute",
      "removeAttribute", "classList", "innerHTML", "addEventListener", "dispatchEvent", "querySelectorAll",
      "elements", "id", "submit", "requestSubmit", "reset", "focus", "contains", "replaceWith", "scrollIntoView"];
    const clobberedForm = phase => `<form id="clobbered" data-hemx-root data-hemx-build="0" data-hemx-resource="1" data-hemx-action="7" action="/submit" method="post" class="${phase}">${clobberedNames.map(name => `<input name="${name}" value="${name}">`).join("")}<button>Save</button></form>`;
    await observe("clobbered-root-form", clobberedForm("before") + '<aside id="sibling">untouched</aside>', async (w, d) => {
      const form = d.querySelector("form");
      const fields = Array.from(d.querySelectorAll("input"));
      const sibling = d.querySelector("aside");
      const shadowed = fields.every(field => form[field.name] === field);
      d.querySelector("button").click();
      await waitFor(() => w.observed.requests.length === 1, "clobbered form request");
      await waitFor(() => !d.querySelector('[aria-busy="true"]'), "form request completion");
      fields[0].value = "unsent edit";
      fields[0].focus();
      fields[0].setSelectionRange(2, 5, "backward");
      const accepted = w.hemx.applyHtml(clobberedForm("after"), form);
      return { accepted, shadowed, same: d.querySelector("form") === form,
        sameFields: fields.every((field, index) => d.querySelectorAll("input")[index] === field),
        sibling: d.querySelector("aside") === sibling && sibling.textContent === "untouched",
        value: fields[0].value, focused: d.activeElement === fields[0],
        selection: [fields[0].selectionStart, fields[0].selectionEnd, fields[0].selectionDirection],
        className: w.Element.prototype.getAttribute.call(form, "class"),
        requests: w.observed.requests, diagnostics: w.observed.diagnostics };
    });

    await observe("submitter-handle", root('<form action="/submit" method="post" data-hemx-action="7"><input type="hidden" name="__h" value="7"><button name="choice" value="alternate" data-hemx-action="8">Save</button></form>'),
      async (w, d) => {
        let submissions = 0;
        d.querySelector("form").addEventListener("submit", () => submissions++);
        d.querySelector("button").click();
        await waitFor(() => w.observed.requests.length === 1, "submitter handle request");
        return { submissions, requests: w.observed.requests };
      });

    await observe("external-submitter", root('<form id="external-form" action="/submit" method="post" data-hemx-action="7"><input name="value" value="ok"></form>') + '<button form="external-form" name="choice" value="external" formaction="/external" formmethod="get">Save</button>',
      async (w, d) => {
        d.querySelector("button").click();
        await waitFor(() => w.location.pathname !== "/frame" || w.observed.diagnostics.length > 0, "external submitter result");
        return { requests: w.observed.requests, location: w.location.href, diagnostics: w.observed.diagnostics };
      });

    await observe("composition", cases.compositionInitial, (w, d) => {
      apply(w, d, "composition");
      const form = d.querySelector("form");
      const action = form?.getAttribute("data-hemx-action");
      return {
        symbolicAttributes: d.querySelectorAll("[data-hemx-form], [data-hemx-handle], [data-hemx-slot]").length,
        numericAction: /^\d+$/.test(action || ""),
        dispatchMatches: form?.querySelector('input[name="__h"]')?.value === action,
        nestedStatus: d.querySelector("#nested")?.textContent,
        diagnostics: w.observed.diagnostics
      };
    }, cases.generatedFingerprint);

    for (const test of cases.keyed) {
      await observe(`keyed-${test.name}`, test.initial, (w, d) => {
        const first = d.querySelector("#rows > *");
        w.hemx.applyBatch(new w.Uint8Array(test.wire).buffer, d.querySelector("[data-hemx-root]"));
        return {
          sameFirst: d.querySelector("#rows > *") === first,
          nested: d.querySelectorAll("#rows > * li").length,
          keys: Array.from(d.querySelector("#rows").children, node => node.getAttribute("data-hemx-key")),
          text: d.querySelector("#rows").textContent,
          status: d.querySelector("#status").textContent,
          diagnostics: w.observed.diagnostics,
          requests: w.observed.requests.length
        };
      }, cases.generatedFingerprint);
    }
    const helpers = { observe, fixture, root, apply, cases, waitFor, pause };
    await observeRuntimeCases(helpers);
    await observeFormAdmission(helpers);
    await observeWireCases(helpers);
    await observeNativeForms(helpers);
    await observeLifecycleCases(helpers);
    await observeStreamCases(helpers);
    await observeReviewCases(helpers);
  } catch (error) {
    results.fatal = String(error);
  } finally {
    const output = document.querySelector("#results");
    output.textContent = JSON.stringify(results);
    try {
      const receipt = await fetch("/results", {
        method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(results)
      });
      if (!receipt.ok) throw new Error(`report rejected: HTTP ${receipt.status}`);
    } catch (error) {
      output.dataset.reportError = String(error);
    }
    output.dataset.complete = "true";
  }
})();
