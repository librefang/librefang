/** @type {import('tailwindcss').Config} */
export default {
  content: [
    "./index.html",
    "./src/**/*.{js,ts,jsx,tsx}",
  ],
  theme: {
    extend: {
      colors: {
        primary: '#10b981',
        'primary-dark': '#059669',
      },
      fontFamily: {
        display: ['Outfit', 'sans-serif'],
      },
    },
  },
  plugins: [],
}
