<script setup lang="ts">
defineProps<{
  modelValue: string
}>()

const emit = defineEmits<{
  'update:modelValue': [value: string]
  'dial': []
}>()

const keys = [
  '1', '2', '3',
  '4', '5', '6',
  '7', '8', '9',
  '*', '0', '#',
]

function onKey(key: string) {
  emit('update:modelValue', key)
}
</script>

<template>
  <div class="grid grid-cols-3 gap-2">
    <button
      v-for="key in keys"
      :key="key"
      class="h-14 rounded-lg bg-secondary text-secondary-foreground text-xl font-semibold hover:bg-secondary/80 active:bg-secondary/60 transition-colors"
      @click="onKey(key)"
    >
      {{ key }}
    </button>
  </div>
</template>
