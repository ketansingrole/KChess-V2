import type { ActivityDay, ActivityItem, TimelineEntry } from '@kchess/contracts/types'

export interface ActivityGroup {
  label: string
  items: ActivityItem[]
}

function dayLabel(ms: number): string {
  return new Date(ms).toLocaleDateString(undefined, {
    weekday: 'short',
    day: 'numeric',
    month: 'short',
  })
}

/** A player's activity, one heading per day. */
export function activityGroups(days: readonly ActivityDay[]): ActivityGroup[] {
  return days.map((day) => ({ label: dayLabel(day.start), items: day.items }))
}

/** Timeline entries (newest first) under the day they happened. */
export function timelineGroups(entries: readonly TimelineEntry[]): ActivityGroup[] {
  const groups: ActivityGroup[] = []
  for (const entry of entries) {
    const label = dayLabel(entry.date)
    const last = groups.at(-1)
    if (last?.label === label) last.items.push(entry.item)
    else groups.push({ label, items: [entry.item] })
  }
  return groups
}
