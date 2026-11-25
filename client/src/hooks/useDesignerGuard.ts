import { useEffect } from 'react'

/**
 * /designer is for roles that configure screens. Anyone else (a viewer
 * following a shared link, an operator) is sent to the 2D view of the same
 * screen instead of a blank page. Waits until the session is known.
 */
export function useDesignerGuard({ routePage, sessionKnown, canConfigure, leave }: {
  routePage: string
  sessionKnown: boolean
  canConfigure: boolean
  leave: () => void
}) {
  useEffect(() => {
    if (routePage === 'designer' && sessionKnown && !canConfigure) leave()
  }, [routePage, sessionKnown, canConfigure, leave])
}
