export function Logo(props: React.ComponentPropsWithoutRef<'span'>) {
  return (
    <span {...props} className="text-lg font-bold text-zinc-900 dark:text-white">
      LibreFang
    </span>
  )
}

export function LogoMark(props: React.ComponentPropsWithoutRef<'span'>) {
  return (
    <span {...props} className="text-lg font-bold text-emerald-500">
      LF
    </span>
  )
}

export function IconMark() {
  return null
}
