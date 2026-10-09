<script setup lang="ts">
/** Glyphs from Lichess's icon font; attribution is in apps/desktop/public/licenses/lichess-icons/. */
const props = defineProps<{ name: string }>()
const glyphs: Record<string, string> = {
  ultraBullet: '\ue059',
  bullet: '\ue032',
  blitz: '\ue008',
  rapid: '\ue002',
  classical: '\ue00a',
  correspondence: '\ue019',
  standard: '\ue021',
  chess960: '\ue006',
  kingOfTheHill: '\ue007',
  antichess: '\ue01e',
  atomic: '\ue01c',
  threeCheck: '\ue00d',
  horde: '\ue03d',
  racingKings: '\ue065',
  crazyhouse: '\ue066',
  fromPosition: '\ue04b',
  shield: '\ue013',
  trophy: '\ue045',
  user: '\ue050',
  check: '\ue023',
}
</script>

<template>
  <span class="lichess-icon" aria-hidden="true">{{ glyphs[props.name] ?? glyphs.trophy }}</span>
</template>

<style scoped>
@font-face {
  font-family: 'KChess Lichess Icons';
  src: url('../assets/fonts/lichess.woff2') format('woff2');
  font-weight: normal;
  font-style: normal;
  font-display: block;
}
.lichess-icon {
  display: inline-block;
  flex: none;
  font-family: 'KChess Lichess Icons';
  font-weight: normal;
  font-style: normal;
  line-height: 1;
  text-transform: none;
  letter-spacing: normal;
  white-space: nowrap;
  -webkit-font-smoothing: antialiased;
  -moz-osx-font-smoothing: grayscale;
}
</style>
