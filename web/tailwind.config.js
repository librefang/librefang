/** @type {import('tailwindcss').Config} */
export default {
  darkMode: 'class',
  content: [
    "./index.html",
    "./src/**/*.{js,ts,jsx,tsx}",
  ],
  theme: {
    extend: {
      colors: {
        cyan: {
          400: '#22d3ee',
          500: '#06b6d4',
          600: '#0891b2',
        },
        amber: {
          400: '#fbbf24',
          500: '#f59e0b',
        },
        surface: {
          DEFAULT: 'var(--bg-surface)',
          100: 'var(--bg-surface-100)',
          200: 'var(--bg-surface-200)',
          300: 'var(--bg-surface-300)',
        },
      },
      fontFamily: {
        sans: ['Inter', 'Noto Sans SC', 'Noto Sans TC', 'Noto Sans JP', 'Noto Sans KR', 'system-ui', 'sans-serif'],
        mono: ['JetBrains Mono', 'SF Mono', 'monospace'],
      },
    },
  },
  plugins: [],
}
