<template>
  <div class="steps-header card-surface">
    <ol class="stepper" aria-label="部署步骤">
      <li
        v-for="(step, index) in steps"
        :key="step"
        class="step"
        :class="{ active: activeStep === index, done: activeStep > index }"
        :aria-current="activeStep === index ? 'step' : undefined"
      >
        <span class="step-node">
          <el-icon v-if="activeStep > index"><Check /></el-icon>
          <span v-else>{{ index + 1 }}</span>
        </span>
        <span>{{ step }}</span>
      </li>
    </ol>
  </div>
</template>

<script setup>
import { Check } from '@element-plus/icons-vue';

defineProps({
  steps: { type: Array, default: () => [] },
  activeStep: { type: Number, default: 0 }
});
</script>

<style scoped>
.steps-header {
  margin-bottom: 20px;
  padding: 18px 24px;
}
.stepper {
  display: flex;
  align-items: center;
  justify-content: space-between;
  list-style: none;
  gap: 12px;
  overflow-x: auto;
  margin: 0;
  padding: 0;
}
.step {
  display: flex;
  align-items: center;
  gap: 10px;
  color: var(--text-muted);
  white-space: nowrap;
  font-size: 12.5px;
}
.step:not(:last-child)::after {
  content: '';
  width: clamp(8px, 2.4vw, 46px);
  height: 2px;
  background: var(--border-color);
  margin-left: 10px;
}
.step-node {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 28px;
  height: 28px;
  flex: none;
  border: 1px solid var(--border-color);
  border-radius: 50%;
  background: var(--panel-bg);
  font-size: 12px;
  font-weight: 600;
  transition: all 0.18s ease;
}
.step.active {
  color: var(--text-main);
  font-weight: 600;
}
.step.active .step-node {
  background: var(--primary-color);
  border-color: var(--primary-color);
  color: white;
  box-shadow: 0 0 0 3px var(--primary-ring);
}
.step.done .step-node {
  color: var(--success-color);
  border-color: rgba(34, 197, 94, 0.4);
  background: var(--success-soft);
}

@media (max-width: 1100px) {
  .step:not(:last-child)::after {
    width: 10px;
    margin-left: 2px;
  }
  .steps-header {
    padding: 16px 18px;
  }
  .stepper {
    gap: 8px;
  }
}
</style>
