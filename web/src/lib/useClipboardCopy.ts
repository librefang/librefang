import { useCallback, useEffect, useRef, useState } from 'react'

export function useClipboardCopy(resetAfterMs = 1500) {
  const [copied, setCopied] = useState(false)
  const resetTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const copyRequestRef = useRef(0)

  useEffect(() => () => {
    copyRequestRef.current += 1
    if (resetTimerRef.current !== null) clearTimeout(resetTimerRef.current)
    resetTimerRef.current = null
  }, [])

  const copy = useCallback((text: string) => {
    const request = ++copyRequestRef.current
    if (resetTimerRef.current !== null) {
      clearTimeout(resetTimerRef.current)
      resetTimerRef.current = null
    }
    setCopied(false)

    void Promise.resolve()
      .then(() => navigator.clipboard.writeText(text))
      .then(
        () => {
          if (request !== copyRequestRef.current) return
          setCopied(true)
          resetTimerRef.current = setTimeout(() => {
            if (request === copyRequestRef.current) setCopied(false)
            resetTimerRef.current = null
          }, resetAfterMs)
        },
        () => {
          if (request === copyRequestRef.current) setCopied(false)
        },
      )
  }, [resetAfterMs])

  return { copied, copy }
}
