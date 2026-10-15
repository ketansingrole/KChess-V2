<script setup lang="ts">
import { useId } from 'vue'
import { RUN_LENGTHS } from '../composables/useRunLength'

/** The Length choice the practice drills share: timed runs or untimed practice. */
defineProps<{ disabled?: boolean }>()
const length = defineModel<number>({ required: true })
const id = useId()
</script>

<template>
  <div class="field">
    <span :id="id" class="field-label">Length</span>
    <UTabs
      v-model="length"
      :items="[...RUN_LENGTHS]"
      :aria-labelledby="id"
      :content="false"
      variant="pill"
      class="w-full"
      :ui="{ trigger: 'grow' }"
      :disabled="disabled"
    />
  </div>
</template>
