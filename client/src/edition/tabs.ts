export interface TabGroup<T> {
  label: string
  items: T[]
}

/**
 * Append extra tabs to the core groups. An extra tab joins the group named by
 * its `group` field (a new group is added after the core ones when the name is
 * unknown). A tab whose id is already taken is dropped so a faulty module
 * cannot shadow a core tab.
 */
export function mergeTabs<T extends { id: string }>(
  coreGroups: readonly TabGroup<T>[],
  extra: readonly (T & { group: string })[],
): TabGroup<T>[] {
  const groups = coreGroups.map(g => ({ label: g.label, items: [...g.items] }))
  const taken = new Set(groups.flatMap(g => g.items.map(i => i.id)))
  for (const tab of extra) {
    if (taken.has(tab.id)) continue
    taken.add(tab.id)
    let group = groups.find(g => g.label === tab.group)
    if (!group) {
      group = { label: tab.group, items: [] }
      groups.push(group)
    }
    group.items.push(tab)
  }
  return groups
}
