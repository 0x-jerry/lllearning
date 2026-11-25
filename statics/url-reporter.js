// Report URL changes that load no new document, such as `#anchor` links and
// `history.pushState`, which the page load handler never sees. The message goes
// to the host's IPC handler, prefixed so it can be told apart from other ones.
(() => {
  const report = () => window.ipc.postMessage('url:' + location.href);
  addEventListener('hashchange', report);
  addEventListener('popstate', report);
  for (const name of ['pushState', 'replaceState']) {
    const original = history[name];
    history[name] = function (...args) {
      const result = original.apply(this, args);
      report();
      return result;
    };
  }
})();
