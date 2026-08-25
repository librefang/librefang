/// <reference types="vite/client" />
declare global {
  interface Window {
    __INITIAL_LANG__?: string
    gtag?: (
      command: 'event',
      action: string,
      params: { event_category: string; event_label: string },
    ) => void
  }
}
export {}
