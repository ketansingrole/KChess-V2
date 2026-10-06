<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import type { VoiceAttempt, VoiceOutcome } from '../../src/shared/types'

/**
 * Settings › Voice: what voice input heard, how it was read and what the player meant. The point
 * is to find what gets misheard — the weakest words, and the phrases that stood for another move.
 */
const entries = ref<VoiceAttempt[]>([])
const loading = ref(true)
const error = ref('')
const confirmClear = ref(false)
const shown = ref(40)

const SUCCESS: VoiceOutcome[] = ['played', 'confirmed', 'picked', 'correct']
const MISS: VoiceOutcome[] = ['invalid', 'unclear', 'wrong', 'cancelled', 'replaced']
const OUTCOME_LABEL: Record<VoiceOutcome, string> = {
  played: 'Played',
  pending: 'Waiting',
  confirmed: 'Confirmed',
  picked: 'Picked',
  cancelled: 'Cancelled',
  replaced: 'Said again',
  abandoned: 'Moved by hand',
  invalid: 'No match',
  unclear: 'Unclear',
  command: 'Command',
  correct: 'Correct',
  wrong: 'Wrong square',
}
const LETTERS = new Set(['b', 'c', 'd', 'e', 'g'])
/** Advice for words that keep scoring low. */
function tipFor(word: string): string {
  if (word === 'pawn' || word === 'pond')
    return 'For pawn moves you can skip the word: just say the square, like “E four”.'
  if (LETTERS.has(word))
    return 'B, C, D, E and G rhyme. The phonetic names (Bravo, Charlie, Delta, Echo, Golf) are much clearer.'
  if (word === 'to' || word === 'two')
    return '“To” and “two” sound the same. Leaving “to” out (“E two E four”) works too.'
  return ''
}

async function load(): Promise<void> {
  loading.value = true
  error.value = ''
  try {
    entries.value = await window.kchess.voiceHistory(2000)
  } catch (cause) {
    console.warn('[voice-history] Could not read voice history:', cause)
    error.value = cause instanceof Error ? cause.message : 'The voice history couldn’t be read.'
  } finally {
    loading.value = false
  }
}
onMounted(load)

const counted = computed(() =>
  entries.value.filter((entry) => SUCCESS.includes(entry.outcome) || MISS.includes(entry.outcome)),
)
const stats = computed(() => {
  const total = counted.value.length
  const success = counted.value.filter((entry) => SUCCESS.includes(entry.outcome)).length
  const firstTry = counted.value.filter(
    (entry) => SUCCESS.includes(entry.outcome) && !entry.retryOf,
  ).length
  return {
    phrases: entries.value.length,
    rate: total ? Math.round((success / total) * 100) : 0,
    firstTry,
    repeats: entries.value.filter((entry) => entry.retryOf).length,
    unclear: entries.value.filter((entry) => entry.outcome === 'unclear').length,
    noise: entries.value.filter((entry) => entry.heard.includes('[unk]')).length,
  }
})

/** Words by average confidence, weakest first; only words heard a few times say anything. */
const weakWords = computed(() => {
  const byWord = new Map<string, { sum: number; count: number }>()
  for (const entry of entries.value)
    for (const { word, conf } of entry.words) {
      const item = byWord.get(word) ?? { sum: 0, count: 0 }
      item.sum += conf
      item.count++
      byWord.set(word, item)
    }
  return [...byWord]
    .map(([word, { sum, count }]) => ({ word, count, conf: sum / count, tip: tipFor(word) }))
    .filter((item) => item.count >= 2 && item.conf < 0.85)
    .sort((a, b) => a.conf - b.conf)
    .slice(0, 10)
})

/** Phrases that missed, paired with what the player meant, most frequent first. */
const mishearings = computed(() => {
  const groups = new Map<string, { heard: string; meant: string; count: number }>()
  for (const entry of entries.value) {
    if (!entry.expected || !MISS.includes(entry.outcome)) continue
    const key = `${entry.heard}\u0000${entry.expected}`
    const group = groups.get(key) ?? { heard: entry.heard, meant: entry.expected, count: 0 }
    group.count++
    groups.set(key, group)
  }
  return [...groups.values()].sort((a, b) => b.count - a.count).slice(0, 10)
})

const recent = computed(() => entries.value.slice(0, shown.value))
function outcomeColor(outcome: VoiceOutcome): 'success' | 'error' | 'warning' | 'neutral' {
  if (SUCCESS.includes(outcome)) return 'success'
  if (outcome === 'unclear' || outcome === 'cancelled' || outcome === 'replaced') return 'warning'
  if (MISS.includes(outcome)) return 'error'
  return 'neutral'
}
const timeFormat = new Intl.DateTimeFormat(undefined, {
  month: 'short',
  day: 'numeric',
  hour: 'numeric',
  minute: '2-digit',
})
const percent = (value: number): string => `${Math.round(value * 100)}%`

async function exportLog(): Promise<void> {
  try {
    await window.kchess.exportVoiceHistory()
  } catch (cause) {
    console.warn('[voice-history] Voice history export failed:', cause)
    error.value = cause instanceof Error ? cause.message : 'The export failed.'
  }
}
async function clearLog(): Promise<void> {
  await window.kchess.clearVoiceHistory()
  await load()
}
</script>

<template>
  <section class="card voice-history" aria-labelledby="voice-history-title">
    <div class="card-header">
      <div>
        <h2 id="voice-history-title" class="section-title">Voice history</h2>
      </div>
      <div class="friend-actions">
        <UButton
          variant="ghost"
          color="neutral"
          icon="i-lucide-rotate-cw"
          aria-label="Refresh"
          :loading="loading"
          @click="load"
        />
        <UButton
          variant="outline"
          color="neutral"
          icon="i-lucide-download"
          :disabled="!entries.length"
          @click="exportLog"
          >Export JSON</UButton
        >
        <UButton
          variant="ghost"
          color="error"
          icon="i-lucide-trash-2"
          :disabled="!entries.length"
          @click="confirmClear = true"
          >Clear</UButton
        >
      </div>
    </div>

    <p v-if="error" class="voice-history-error" role="alert">{{ error }}</p>
    <p v-else-if="!loading && !entries.length" class="muted text-sm">
      Nothing yet. Speak a few moves in Play with Computer or a Coordinates run, then come back.
    </p>

    <template v-if="entries.length">
      <div class="voice-stats">
        <div>
          <strong class="tabular">{{ stats.phrases }}</strong
          ><span>phrases</span>
        </div>
        <div>
          <strong class="tabular">{{ stats.rate }}%</strong><span>understood</span>
        </div>
        <div>
          <strong class="tabular">{{ stats.repeats }}</strong
          ><span>had to repeat</span>
        </div>
        <div>
          <strong class="tabular">{{ stats.unclear }}</strong
          ><span>too unclear</span>
        </div>
        <div>
          <strong class="tabular">{{ stats.noise }}</strong
          ><span>with unknown sounds</span>
        </div>
      </div>

      <div class="voice-insights">
        <div>
          <h3 class="voice-subtitle">Least certain words</h3>
          <p v-if="!weakWords.length" class="muted text-xs">No word stands out yet.</p>
          <ul class="voice-words">
            <li v-for="item in weakWords" :key="item.word">
              <div class="voice-word-row">
                <strong>{{ item.word }}</strong>
                <span class="voice-bar-track" aria-hidden="true"
                  ><span :style="{ width: percent(item.conf) }" :class="{ low: item.conf < 0.6 }"
                /></span>
                <span class="tabular muted text-xs"
                  >{{ percent(item.conf) }} · {{ item.count }}×</span
                >
              </div>
              <p v-if="item.tip" class="muted text-xs">{{ item.tip }}</p>
            </li>
          </ul>
        </div>
        <div>
          <h3 class="voice-subtitle">Heard → meant</h3>
          <p v-if="!mishearings.length" class="muted text-xs">No corrections yet.</p>
          <ul class="voice-pairs">
            <li v-for="pair in mishearings" :key="`${pair.heard}→${pair.meant}`">
              <span class="voice-quote">“{{ pair.heard }}”</span>
              <UIcon name="i-lucide-arrow-right" class="muted shrink-0" />
              <strong>{{ pair.meant }}</strong>
              <span v-if="pair.count > 1" class="muted text-xs tabular">{{ pair.count }}×</span>
            </li>
          </ul>
        </div>
      </div>

      <h3 class="voice-subtitle">Recent</h3>
      <div class="voice-table" role="table" aria-label="Recent voice phrases">
        <div class="voice-table-row head" role="row">
          <span role="columnheader">When</span>
          <span role="columnheader">Heard</span>
          <span role="columnheader">Read as</span>
          <span role="columnheader">Meant</span>
          <span role="columnheader">Result</span>
        </div>
        <div v-for="entry in recent" :key="entry.id" class="voice-table-row" role="row">
          <span role="cell" class="muted tabular">{{ timeFormat.format(entry.at) }}</span>
          <span role="cell" class="voice-heard-cell">
            <UIcon
              v-if="entry.retryOf"
              name="i-lucide-repeat"
              class="muted shrink-0"
              title="Said again after a miss"
            />
            <span>
              <template v-for="(word, index) in entry.words" :key="index"
                ><span
                  :class="{ 'low-word': word.conf < 0.6 }"
                  :title="`${word.word}: ${percent(word.conf)} sure`"
                  >{{ word.word }}</span
                >{{ ' ' }}</template
              ><template v-if="!entry.words.length">{{ entry.heard }}</template>
              <span class="muted text-xs tabular"> {{ percent(entry.confidence) }}</span>
            </span>
          </span>
          <span role="cell">{{ entry.parsed?.replaceAll('|', ' / ') ?? '—' }}</span>
          <span role="cell">{{ entry.expected ?? '—' }}</span>
          <span role="cell">
            <UBadge :color="outcomeColor(entry.outcome)" variant="soft" size="sm">{{
              OUTCOME_LABEL[entry.outcome]
            }}</UBadge>
          </span>
        </div>
      </div>
      <UButton
        v-if="entries.length > shown"
        variant="ghost"
        color="neutral"
        size="sm"
        @click="shown += 80"
        >Show more</UButton
      >
    </template>

    <ConfirmDialog
      v-model:open="confirmClear"
      title="Clear voice history?"
      description="Every logged phrase is deleted. Export it first if you want to keep it."
      confirm-label="Clear"
      @confirm="clearLog"
    />
  </section>
</template>

<style scoped>
.voice-history {
  display: flex;
  flex-direction: column;
  gap: 16px;
}
.voice-history-error {
  color: var(--ui-error);
  font-size: 13px;
}
.voice-stats {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(110px, 1fr));
  gap: 8px;
}
.voice-stats > div {
  display: flex;
  flex-direction: column;
  padding: 10px 12px;
  border-radius: 10px;
  background: var(--ui-bg-elevated);
}
.voice-stats strong {
  font-size: 20px;
}
.voice-stats span {
  font-size: 12px;
  color: var(--ui-text-muted);
}
.voice-insights {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(min(100%, 300px), 1fr));
  gap: 16px 28px;
}
.voice-subtitle {
  margin-bottom: 8px;
  font-size: 13px;
  font-weight: 650;
}
.voice-words,
.voice-pairs {
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.voice-word-row {
  display: grid;
  grid-template-columns: minmax(60px, auto) 1fr auto;
  align-items: center;
  gap: 10px;
  font-size: 13px;
}
.voice-bar-track {
  height: 6px;
  border-radius: 999px;
  background: var(--ui-bg-accented);
  overflow: hidden;
}
.voice-bar-track > span {
  display: block;
  height: 100%;
  background: var(--ui-primary);
}
.voice-bar-track > span.low {
  background: var(--ui-warning);
}
.voice-pairs li {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 13px;
  min-width: 0;
}
.voice-quote {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.voice-table {
  display: flex;
  flex-direction: column;
  border: 1px solid var(--ui-border);
  border-radius: 10px;
  overflow: hidden;
  font-size: 13px;
}
.voice-table-row {
  display: grid;
  grid-template-columns: 110px minmax(0, 2fr) minmax(0, 1fr) 70px 120px;
  gap: 10px;
  align-items: center;
  padding: 7px 12px;
}
.voice-table-row + .voice-table-row {
  border-top: 1px solid var(--ui-border);
}
.voice-table-row.head {
  font-size: 12px;
  font-weight: 600;
  color: var(--ui-text-muted);
  background: var(--ui-bg-elevated);
}
.voice-heard-cell {
  display: flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
  overflow-wrap: anywhere;
}
.low-word {
  color: var(--ui-warning);
  text-decoration: underline dotted;
}
@container page (max-width: 700px) {
  .voice-table-row {
    grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
  }
  .voice-table-row.head {
    display: none;
  }
}
</style>
