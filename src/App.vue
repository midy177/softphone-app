<script setup lang="ts">
import { onMounted } from 'vue'
import { Toaster } from '@/components/ui/sonner'
import { restoreSipFlowConfig } from '@/utils/configManager'

// 应用启动时恢复配置
onMounted(async () => {
  await restoreSipFlowConfig()
})
</script>

<template>
  <RouterView />
  <Toaster position="top-center" />
</template>
