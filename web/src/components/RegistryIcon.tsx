import {
  BarChart3, Bell, Bird, BookOpen, Bot, Box, Briefcase, Bug, Building,
  Calendar, Castle, CheckCircle, Circle, ClipboardList, Cloud, Command,
  Construction, Database, Diamond, Droplet, Feather, FileText, Film,
  Flame, FlaskConical, Folder, Gamepad2, Globe, HardHat, Heart,
  Hexagon, Leaf, Link, Lock, LockKeyhole, Mail, Megaphone, MessageCircle,
  Mic, Monitor, Package, Palette, Plug, Rocket, Ruler, SatelliteDish,
  Save, Search, Shield, Shuffle, Signal, Smartphone, Sparkles, Square,
  Target, TrendingUp, Users, Volume2,
} from 'lucide-react'
import type { LucideIcon } from 'lucide-react'
import { Github, Twitter } from './BrandIcons'

// Mapping of the kebab-case lucide names used in librefang-registry TOMLs
// to the actual React components. Keep in sync with the set of icons
// the registry repo ships — anything missing falls back to <Box/>.
const MAP: Record<string, LucideIcon> = {
  'bar-chart-3': BarChart3, bell: Bell, bird: Bird, 'book-open': BookOpen,
  bot: Bot, briefcase: Briefcase, bug: Bug, building: Building,
  calendar: Calendar, castle: Castle, 'check-circle': CheckCircle,
  circle: Circle, 'clipboard-list': ClipboardList, cloud: Cloud,
  command: Command, construction: Construction, database: Database,
  diamond: Diamond, droplet: Droplet, feather: Feather, 'file-text': FileText,
  film: Film, flame: Flame, 'flask-conical': FlaskConical, folder: Folder,
  'gamepad-2': Gamepad2, github: Github, globe: Globe, 'hard-hat': HardHat,
  heart: Heart, hexagon: Hexagon, leaf: Leaf, link: Link, lock: Lock,
  'lock-keyhole': LockKeyhole, mail: Mail, megaphone: Megaphone,
  'message-circle': MessageCircle, mic: Mic, monitor: Monitor,
  package: Package, palette: Palette, plug: Plug, rocket: Rocket,
  ruler: Ruler, 'satellite-dish': SatelliteDish, save: Save, search: Search,
  shield: Shield, shuffle: Shuffle, signal: Signal, smartphone: Smartphone,
  sparkles: Sparkles, square: Square, target: Target,
  'trending-up': TrendingUp, twitter: Twitter, users: Users, 'volume-2': Volume2,
}

interface Props {
  icon: string | undefined
  className?: string
  fallbackClassName?: string
}

// Render a registry item's icon. The upstream TOMLs now store
// "lucide:<kebab-name>" — older/unmigrated entries may still store a raw
// emoji glyph, which we render as text for backwards compatibility.
export default function RegistryIcon({ icon, className = 'w-5 h-5', fallbackClassName }: Props) {
  if (!icon) return null
  if (icon.startsWith('lucide:')) {
    const name = icon.slice(7)
    const Cmp = MAP[name] ?? Box
    return <Cmp className={className} aria-hidden />
  }
  // Legacy emoji — render as glyph.
  return (
    <span className={fallbackClassName ?? 'text-xl leading-none'} aria-hidden>
      {icon}
    </span>
  )
}
