<script setup lang="ts">
import { computed } from 'vue'
import { Label } from '@/components/ui/label'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
} from '@/components/ui/select'
import type { MediaDevice } from '@/composables/useAudio'

const props = defineProps<{
  label: string
  devices: MediaDevice[]
  modelValue: string
}>()

const emit = defineEmits<{
  'update:modelValue': [value: string]
}>()

const selectedLabel = computed(() => {
  const found = props.devices.find((d) => d.name === props.modelValue)
  return found?.description || `选择${props.label}`
})
</script>

<template>
  <div class="w-1/2 min-w-0 space-y-1">
    <Label class="text-xs text-muted-foreground">{{ label }}</Label>
    <Select :model-value="modelValue" @update:model-value="emit('update:modelValue', $event)">
      <SelectTrigger class="h-8 text-xs w-full overflow-hidden">
        <span class="truncate block">{{ selectedLabel }}</span>
      </SelectTrigger>
      <SelectContent>
        <SelectItem
          v-for="device in devices"
          :key="device.name"
          :value="device.name"
        >
          {{ device.description }}
        </SelectItem>
      </SelectContent>
    </Select>
  </div>
</template>
