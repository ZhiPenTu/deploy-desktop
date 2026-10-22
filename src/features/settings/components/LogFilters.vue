<template>
  <section class="log-filters">
    <button
      v-for="item in filters"
      :key="item.value"
      type="button"
      class="filter-card"
      :class="[`is-${item.tone}`, { 'is-active': level === item.value }]"
      @click="emit('select', item.value)"
    >
      <span class="filter-label">{{ item.label }}</span>
      <span class="filter-count">{{ item.count }}</span>
    </button>
  </section>
</template>

<script setup>
defineProps({
  filters: { type: Array, default: () => [] },
  level: { type: String, default: '' }
});

const emit = defineEmits(['select']);
</script>

<style scoped>
.log-filters {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(150px, 1fr));
  gap: 12px;
}

.filter-card {
  display: flex;
  flex-direction: column;
  gap: 6px;
  padding: 14px 16px;
  text-align: left;
  border: 1px solid var(--border-color);
  border-radius: var(--radius-md);
  background: var(--panel-bg);
  box-shadow: var(--shadow-card);
  cursor: pointer;
  transition: border-color 0.15s ease, box-shadow 0.15s ease, transform 0.15s ease;
}

.filter-card:hover {
  border-color: var(--border-strong);
}

.filter-card.is-active {
  border-color: var(--primary-color);
  box-shadow: 0 0 0 3px var(--primary-ring);
}

.filter-label {
  font-size: 12px;
  color: var(--text-muted);
}

.filter-count {
  font-size: 22px;
  font-weight: 650;
  letter-spacing: -0.02em;
  color: var(--text-main);
  font-variant-numeric: tabular-nums;
}

.filter-card.is-info .filter-count {
  color: var(--success-color);
}

.filter-card.is-warn .filter-count {
  color: var(--warning-color);
}

.filter-card.is-error .filter-count {
  color: var(--danger-color);
}

.filter-card.is-debug .filter-count {
  color: var(--text-secondary);
}

@media (max-width: 960px) {
  .log-filters {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }
}
</style>
