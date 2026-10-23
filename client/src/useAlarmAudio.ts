import { useRef, useCallback, useEffect } from 'react'

export function useAlarmAudio() {
  const audioRef = useRef<HTMLAudioElement | null>(null)
  const mutedRef = useRef(false)
  const playingRef = useRef(false)

  useEffect(() => {
    audioRef.current = new Audio('/alarm.wav')
    audioRef.current.loop = true
    audioRef.current.volume = 0.5
    return () => {
      if (audioRef.current) {
        audioRef.current.pause()
        audioRef.current = null
      }
    }
  }, [])

  const startAlarm = useCallback(() => {
    if (!audioRef.current || mutedRef.current || playingRef.current) return
    audioRef.current.play().catch(() => {})
    playingRef.current = true
  }, [])

  const stopAlarm = useCallback(() => {
    if (!audioRef.current) return
    audioRef.current.pause()
    audioRef.current.currentTime = 0
    playingRef.current = false
  }, [])

  const toggleMute = useCallback(() => {
    mutedRef.current = !mutedRef.current
    if (mutedRef.current) stopAlarm()
  }, [stopAlarm])

  const isMuted = useCallback(() => mutedRef.current, [])

  return { startAlarm, stopAlarm, toggleMute, isMuted }
}