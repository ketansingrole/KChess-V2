<script setup lang="ts">
import { computed } from 'vue'
import OpeningsBrowser from '../components/openings/OpeningsBrowser.vue'
import OpeningsDetail from '../components/openings/OpeningsDetail.vue'
import { openingLine } from '../components/openings/openingLink'

const route = useRoute()
const line = computed(() => openingLine(route.query.line))
</script>

<template>
  <div>
    <PageHeader title="Openings" />
    <!-- The tree keeps its search and open families while an opening is shown. -->
    <KeepAlive>
      <OpeningsBrowser v-if="!line" />
    </KeepAlive>
    <OpeningsDetail v-if="line" :moves="line" />
  </div>
</template>
