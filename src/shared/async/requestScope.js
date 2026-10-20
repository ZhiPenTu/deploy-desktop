// Ignore responses from an older target, an older request, or a disposed view.
export function createRequestScope() {
  let revision = 0;
  let disposed = false;
  return {
    begin: () => ++revision,
    isCurrent: (ticket) => !disposed && ticket === revision,
    invalidate: () => { revision += 1; },
    dispose: () => { disposed = true; revision += 1; }
  };
}
