// Registration may finish after navigation. A late listener must still be removed.
export function createProgressSubscription(subscribe, onProgress, onError = () => {}) {
  let active = true;
  let unsubscribe = null;
  const report = (error) => { if (active) onError(error); };
  const ready = Promise.resolve()
    .then(() => subscribe((payload) => { if (active) onProgress(payload); }))
    .then(async (stop) => {
      if (typeof stop !== 'function') return;
      if (active) unsubscribe = stop;
      else await stop();
    })
    .catch(report);

  return {
    ready,
    dispose() {
      if (!active) return;
      active = false;
      if (unsubscribe) {
        Promise.resolve().then(unsubscribe).catch(report);
        unsubscribe = null;
      }
    }
  };
}
