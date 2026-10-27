/**
 * Name for the copy of a tag. The default naming rule only allows letters,
 * digits, space, `_`, `.` and `-`, so the server answers 400 for "(Copy)".
 */
export function duplicateName(name: string): string {
  return `${name} copy`
}
