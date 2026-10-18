async function observeReviewCases({ observe, root, cases, waitFor, pause }) {
  const apply = (w, d, wire) => w.hemx.applyBatch(new w.Uint8Array(wire).buffer, d.querySelector('main'));
  const noMove = w => {
    w.Element.prototype.moveBefore = undefined;
    w.DocumentFragment.prototype.moveBefore = undefined;
    w.Document.prototype.moveBefore = undefined;
  };
  for (const test of cases.review.folds) {
    for (const mode of ['sequential', 'merged', 'sse']) {
      const collection = test.name === 'rows' || test.name === 'delta';
      const content = collection
        ? '<ul data-hemx-resource="1" data-hemx-collection><li data-hemx-key="a"><input value="a"></li></ul>'
        : '<div data-hemx-resource="1"><input value="a"></div>';
      await observe(`review-fold-${test.name}-${mode}`, root(content), async (w, d) => {
        const accepted = [];
        if (mode === 'sse') {
          const main = d.querySelector('main');
          main.setAttribute('data-hemx-sse', `/review/${test.name}`);
          await waitFor(() => test.name === 'replace' ? d.querySelector('section div')?.textContent === 'b' : d.querySelector('input').value === 'a', 'lossless SSE sequence');
          main.removeAttribute('data-hemx-sse');
          await pause();
        } else for (const wire of test[mode]) accepted.push(apply(w, d, wire));
        return { accepted, value: d.querySelector('input')?.value ?? null, wrapper: !!d.querySelector('section'), text: d.querySelector('[data-hemx-resource="1"]').textContent, diagnostics: w.observed.diagnostics };
      }, '0', (w, d) => { d.querySelector('input').value = 'user draft'; });
    }
  }
  const abc = '<ul data-hemx-resource="1" data-hemx-collection><li data-hemx-key="a">A</li><li data-hemx-key="b">B</li><li data-hemx-key="c">C</li></ul>';
  for (const test of cases.review.movement) {
    await observe(`review-move-${test.name}`, root(abc), (w, d) => {
      const main = d.querySelector('main'), before = main.innerHTML;
      const observer = new w.MutationObserver(() => {});
      observer.observe(main, { childList: true, attributes: true, characterData: true, subtree: true });
      const accepted = apply(w, d, test.wire);
      const mutations = observer.takeRecords().length;
      observer.disconnect();
      return { accepted, unchanged: before === main.innerHTML, mutations, diagnostics: w.observed.diagnostics.length };
    }, '0', noMove);
  }
  const nested = '<ul data-hemx-resource="1" data-hemx-collection><li data-hemx-key="a"><ul data-hemx-resource="2" data-hemx-collection><li data-hemx-key="x">x</li></ul></li></ul>';
  for (const mode of ['rows', 'delta']) {
    await observe(`review-nested-${mode}`, root(nested), async (w, d) => {
      const wires = cases.review.nested;
      const accepted = [apply(w, d, wires.seed)];
      const inner = d.querySelector('[data-hemx-resource="2"]');
      accepted.push(apply(w, d, wires[mode]));
      const retained = inner === d.querySelector('[data-hemx-resource="2"]');
      let resyncs = 0;
      d.addEventListener('hemx:resync', () => resyncs++);
      // Observe the requested recovery without changing the page under test.
      w.fetch = async () => new w.Response(null, { status: 204 });
      accepted.push(apply(w, d, wires.stale));
      const afterStale = Array.from(inner.children, e => e.getAttribute('data-hemx-key'));
      await pause();
      accepted.push(apply(w, d, wires.current));
      return { accepted, retained, afterStale, resyncs, keys: Array.from(inner.children, e => e.getAttribute('data-hemx-key')), diagnostics: w.observed.diagnostics };
    });
  }
}
