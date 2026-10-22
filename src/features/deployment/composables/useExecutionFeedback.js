import { computed, ref } from 'vue';
import { onRemoteUpdateProgress } from '../../../api/tauri.js';
import { createProgressSubscription } from '../../../shared/async/progressSubscription.js';

/** 执行阶段的进度与计时反馈，只负责展示状态，不参与部署决策。 */
export function useExecutionFeedback() {
  const progressStep = ref('');
  const progressPercent = ref(0);
  const progressMessage = ref('');
  const elapsedSeconds = ref(0);

  let subscription = null;
  let elapsedTimer = null;

  const elapsedText = computed(() => {
    const minutes = Math.floor(elapsedSeconds.value / 60);
    const seconds = elapsedSeconds.value % 60;
    return minutes ? `${minutes} 分 ${String(seconds).padStart(2, '0')} 秒` : `${seconds} 秒`;
  });

  function start(onPhaseMessage) {
    stop();
    progressStep.value = '准备执行';
    progressPercent.value = 2;
    progressMessage.value = '';
    elapsedSeconds.value = 0;
    subscription = createProgressSubscription(
      onRemoteUpdateProgress,
      (payload) => {
        if (!payload) return;
        const percent = Number(payload.percent);
        if (Number.isFinite(percent)) {
          progressPercent.value = Math.max(0, Math.min(100, Math.round(percent)));
        }
        if (payload.step) progressStep.value = String(payload.step);
        if (payload.message) {
          progressMessage.value = String(payload.message);
          onPhaseMessage?.(progressMessage.value);
        }
      },
      () => {
        progressMessage.value = '无法订阅执行进度，完成结果不受影响。';
      }
    );
    elapsedTimer = setInterval(() => {
      elapsedSeconds.value += 1;
    }, 1000);
  }

  function stop() {
    subscription?.dispose();
    subscription = null;
    if (elapsedTimer) clearInterval(elapsedTimer);
    elapsedTimer = null;
  }

  return { progressStep, progressPercent, progressMessage, elapsedText, start, stop };
}
