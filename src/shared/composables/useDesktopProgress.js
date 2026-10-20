import { onMounted, onScopeDispose } from 'vue';
import { createProgressSubscription } from '../async/progressSubscription.js';

export function useDesktopProgress(subscribe, onProgress, onError) {
  let subscription;
  onMounted(() => {
    subscription = createProgressSubscription(subscribe, onProgress, onError);
  });
  onScopeDispose(() => subscription?.dispose());
}
