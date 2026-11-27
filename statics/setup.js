// Runs in every page before its own scripts. Everything here reports to the
// host's IPC handler, each message prefixed so it can be told apart from the
// others and from page-initiated messages.
(() => {
  // Report URL changes that load no new document, such as `#anchor` links and
  // `history.pushState`, which the page load handler never sees.
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

  // Report clicks: the page is a native child window, so a click in it reaches
  // no GPUI element. Capture phase, so a page script that handles the click
  // cannot hide it. Main frame only, so a click inside an iframe still goes
  // unreported.
  addEventListener('mousedown', () => window.ipc.postMessage('click:'), true);
})();
