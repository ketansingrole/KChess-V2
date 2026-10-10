<script setup lang="ts">
import { computed } from 'vue'
import { USERNAME } from '@kchess/rules/patterns'

/** A Lichess username that opens the player's profile; anything else (Anonymous, AI) is plain. */
const props = defineProps<{ username: string }>()
const linkable = computed(() => USERNAME.test(props.username))
</script>

<template>
  <NuxtLink
    v-if="linkable"
    :to="{ path: '/players', query: { name: username } }"
    class="player-link"
    :title="`Open ${username}'s profile`"
    @click.stop
    ><slot>{{ username }}</slot></NuxtLink
  >
  <span v-else
    ><slot>{{ username }}</slot></span
  >
</template>
