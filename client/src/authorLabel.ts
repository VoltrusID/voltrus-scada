/** Who wrote a handover entry: the username, or `#id` when the account no longer exists. */
export function authorLabel(username: string | null | undefined, id: number): string {
  return username ? username : `#${id}`
}
