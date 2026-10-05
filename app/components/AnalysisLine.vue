<script setup lang="ts">
import { computed } from 'vue'
import { moveGlyph, moveNumber, pathOf, movesOf, type TreeNode } from '../utils/analysisTree'
import { useAnalysisStore } from '../stores/analysis'
import type { Judgment } from '../../src/shared/types'

/**
 * One line of the analysis tree, Lichess-style: the moves of a line run on, and the alternatives
 * to a move follow it in brackets before the line resumes (“2. Nf3 (2. f4 exf4) 2… Nc6”).
 * Variations off the main line get their own indented block; deeper ones stay inline.
 */
const props = defineProps<{
  parent: TreeNode
  parentPath: string
  /** The move this line starts with; the parent's first child (its main line) when omitted. */
  first?: TreeNode
  current: string
  depth: number
}>()
const emit = defineEmits<{ select: [path: string] }>()
/** The game review's labels for main-line moves. */
const { reviewMarks } = storeToRefs(useAnalysisStore())
const GLYPHS = { inaccuracy: '?!', mistake: '?', blunder: '??' } as const

type Item =
  | { kind: 'move'; node: TreeNode; path: string; number: string; judgment?: Judgment }
  | { kind: 'variations'; parent: TreeNode; parentPath: string; alternatives: TreeNode[] }

const items = computed<Item[]>(() => {
  const list: Item[] = []
  let parent = props.parent
  let parentPath = props.parentPath
  let node: TreeNode | undefined = props.first ?? props.parent.children[0]
  let numbered = true
  while (node) {
    const path = pathOf([...movesOf(parentPath), node.uci])
    list.push({
      kind: 'move',
      node,
      path,
      number: moveNumber(parent.ply, numbered),
      judgment: reviewMarks.value.get(path)?.judgment,
    })
    numbered = false
    // The alternatives to a main move come right after it (a variation's own siblings were
    // already listed by the line it branches from).
    if (node === parent.children[0] && parent.children.length > 1) {
      list.push({
        kind: 'variations',
        parent,
        parentPath,
        alternatives: parent.children.slice(1),
      })
      numbered = true
    }
    parent = node
    parentPath = path
    node = node.children[0]
  }
  return list
})
</script>

<template>
  <template v-for="item in items" :key="item.kind === 'move' ? item.path : `${item.parentPath}|v`">
    <template v-if="item.kind === 'move'">
      <button
        type="button"
        class="tree-move"
        :class="{ active: item.path === current, main: depth === 0 }"
        :data-path="item.path"
        :aria-current="item.path === current ? 'step' : undefined"
        @click="emit('select', item.path)"
      >
        <span v-if="item.number" class="tree-number">{{ item.number }}</span>
        {{ item.node.san
        }}<span
          v-if="moveGlyph(item.node)"
          class="tree-glyph"
          :title="moveGlyph(item.node)!.label"
          >{{ moveGlyph(item.node)!.glyph }}</span
        ><span
          v-else-if="item.judgment"
          :class="['tree-glyph', item.judgment]"
          :title="item.judgment"
          >{{ GLYPHS[item.judgment] }}</span
        >
      </button>
      <span
        v-if="item.node.comments?.length"
        class="tree-comment"
        :data-path="item.path"
        :class="{ block: depth === 0 }"
        @click="emit('select', item.path)"
        >{{ item.node.comments.join(' ') }}</span
      >
    </template>
    <component
      :is="depth === 0 ? 'div' : 'span'"
      v-else
      :class="depth === 0 ? 'tree-variations' : 'tree-inline'"
    >
      <span v-for="alternative in item.alternatives" :key="alternative.uci" class="tree-variation">
        <span class="tree-paren">(</span>
        <AnalysisLine
          :parent="item.parent"
          :parent-path="item.parentPath"
          :first="alternative"
          :current="current"
          :depth="depth + 1"
          @select="emit('select', $event)"
        />
        <span class="tree-paren">)</span>
      </span>
    </component>
  </template>
</template>
