<template>
  <el-table :data="services" size="small" empty-text="未发现服务">
    <el-table-column prop="name" label="服务" width="190" />
    <el-table-column label="镜像" min-width="280">
      <template #default="{ row }">
        <code>{{ row.image || '-' }}</code>
      </template>
    </el-table-column>
    <el-table-column prop="containerName" label="容器名" min-width="200" />
    <el-table-column label="端口" min-width="130">
      <template #default="{ row }">
        <span v-if="!row.ports.length" class="muted">-</span>
        <el-tag v-for="port in row.ports" :key="port" size="small" effect="plain" class="service-tag">
          {{ port }}
        </el-tag>
      </template>
    </el-table-column>
    <el-table-column label="依赖" min-width="150">
      <template #default="{ row }">
        <span v-if="!row.dependsOn.length" class="muted">-</span>
        <el-tag
          v-for="dependency in row.dependsOn"
          :key="dependency"
          size="small"
          effect="plain"
          class="service-tag"
        >
          {{ dependency }}
        </el-tag>
      </template>
    </el-table-column>
    <el-table-column label="重启策略" width="130">
      <template #default="{ row }">
        <span class="muted">{{ row.restart || '-' }}</span>
      </template>
    </el-table-column>
  </el-table>
</template>

<script setup>
defineProps({
  services: { type: Array, default: () => [] }
});
</script>

<style scoped>
.service-tag {
  margin: 2px 4px 2px 0;
}

.muted {
  color: var(--text-faint);
  font-size: 12px;
}
</style>
