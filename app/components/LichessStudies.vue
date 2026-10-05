<script setup lang="ts">
import { ref } from 'vue'
import { useAnalysisStore } from '../stores/analysis'
import { useStudyStore } from '../stores/studies'
import { treeFromPgn } from '../utils/analysisTree'
import type { LichessStudy, LichessStudyChapter } from '../../src/shared/types'

/** Studies on the player's Lichess accounts: open or keep their chapters, or send the board to one. */
const analysis = useAnalysisStore()
const library = useStudyStore()
const app = useKChessStore()
const toast = useToast()

const account = ref(app.activeOnlineAccount)
const remote = ref<LichessStudy[] | null>(null)
const remoteStudy = ref('')
const chapters = ref<LichessStudyChapter[] | null>(null)
const busy = ref(false)
const message = ref('')
const target = ref('')

function reconnectNote(result: unknown): boolean {
  if (result && typeof result === 'object' && 'needsReconnect' in result) {
    message.value = `Lichess needs the study permission: connect @${account.value} again in Settings → My accounts.`
    return true
  }
  return false
}
async function work(task: () => Promise<void>): Promise<void> {
  busy.value = true
  message.value = ''
  try {
    await task()
  } catch (cause) {
    message.value = cause instanceof Error ? cause.message : String(cause)
  } finally {
    busy.value = false
  }
}
function load(): Promise<void> {
  return work(async () => {
    const result = await window.kchess.lichessStudies(account.value)
    if (reconnectNote(result)) return
    remote.value = result as LichessStudy[]
    if (!remote.value.length) message.value = 'No studies on this account yet.'
  })
}
function openRemote(id: string): Promise<void> {
  if (remoteStudy.value === id) {
    remoteStudy.value = ''
    return Promise.resolve()
  }
  remoteStudy.value = id
  chapters.value = null
  return work(async () => {
    const result = await window.kchess.lichessStudyChapters(account.value, id)
    if (reconnectNote(result)) return
    chapters.value = result as LichessStudyChapter[]
  })
}
function openChapter(chapter: LichessStudyChapter): void {
  if (!analysis.loadPgn(chapter.pgn)) {
    message.value = 'That chapter uses rules the analysis board cannot show (a variant).'
    return
  }
  app.selectPage('analysis')
}
function saveChapters(): void {
  const studyName = remote.value?.find((s) => s.id === remoteStudy.value)?.name ?? 'Lichess study'
  let saved = 0
  try {
    for (const chapter of chapters.value ?? []) {
      if (!treeFromPgn(chapter.pgn)) continue
      library.save(`${studyName} · ${chapter.name}`.slice(0, 120), chapter.pgn)
      saved++
    }
    toast.add({ title: `Saved ${saved} chapters on this device`, icon: 'i-lucide-download' })
  } catch (cause) {
    message.value = cause instanceof Error ? cause.message : String(cause)
  }
}
function send(): Promise<void> {
  return work(async () => {
    const chapterName = (
      analysis.study?.name ||
      analysis.root.headers?.Event ||
      'KChess analysis'
    ).slice(0, 100)
    const result = await window.kchess.exportToLichessStudy(
      account.value,
      target.value,
      chapterName,
      analysis.pgn(),
    )
    if (reconnectNote(result)) return
    message.value = target.value
      ? `Added “${chapterName}” as a chapter.`
      : `Created a private study with “${chapterName}”.`
    void load()
  })
}
</script>

<template>
  <div class="flex flex-col gap-4">
    <div class="flex flex-wrap items-center gap-2">
      <USelect
        v-if="app.connectedAccounts.length > 1"
        v-model="account"
        :items="app.connectedAccounts.map((a) => ({ label: `@${a.username}`, value: a.username }))"
        size="sm"
        aria-label="Account"
        @update:model-value="remote = null"
      />
      <UButton
        size="sm"
        variant="outline"
        color="neutral"
        icon="i-lucide-refresh-cw"
        :loading="busy"
        @click="load"
        >{{ remote ? 'Refresh' : 'Show my Lichess studies' }}</UButton
      >
    </div>
    <p v-if="message" role="status" class="text-sm">{{ message }}</p>

    <ul v-if="remote?.length" class="remote-list">
      <li v-for="item in remote" :key="item.id">
        <button
          type="button"
          class="remote-row"
          :aria-expanded="remoteStudy === item.id"
          @click="openRemote(item.id)"
        >
          <UIcon
            :name="remoteStudy === item.id ? 'i-lucide-chevron-down' : 'i-lucide-chevron-right'"
          />
          <span class="truncate">{{ item.name }}</span>
        </button>
        <div v-if="remoteStudy === item.id" class="chapters">
          <p v-if="!chapters" class="text-xs muted">Loading chapters…</p>
          <template v-else>
            <UButton
              v-for="chapter in chapters"
              :key="chapter.name + chapter.pgn.length"
              size="xs"
              variant="ghost"
              color="neutral"
              icon="i-lucide-square-play"
              class="justify-start"
              @click="openChapter(chapter)"
              >{{ chapter.name }}</UButton
            >
            <UButton
              size="xs"
              variant="soft"
              icon="i-lucide-download"
              class="mt-1 self-start"
              @click="saveChapters"
              >Save all chapters on this device</UButton
            >
          </template>
        </div>
      </li>
    </ul>

    <form class="card flex flex-wrap items-center gap-2" @submit.prevent="send">
      <span class="text-sm font-semibold">Send the analysis board to</span>
      <USelect
        v-model="target"
        :items="[
          { label: 'A new private study', value: '' },
          ...(remote ?? []).map((s) => ({ label: s.name, value: s.id })),
        ]"
        size="sm"
        aria-label="Lichess study to add to"
        class="min-w-48"
      />
      <UButton type="submit" size="sm" icon="i-lucide-upload" :loading="busy">Send</UButton>
    </form>
  </div>
</template>

<style scoped>
.remote-list {
  display: flex;
  flex-direction: column;
  gap: 2px;
  margin: 0;
  padding: 0;
  list-style: none;
}
.remote-row {
  display: flex;
  align-items: center;
  gap: 8px;
  width: 100%;
  padding: 8px 10px;
  border: 0;
  border-radius: 8px;
  background: transparent;
  color: inherit;
  font: inherit;
  text-align: left;
  cursor: pointer;
}
.remote-row:hover {
  background: var(--ui-bg-elevated);
}
.chapters {
  display: flex;
  flex-direction: column;
  gap: 2px;
  margin: 0 0 8px 30px;
}
</style>
