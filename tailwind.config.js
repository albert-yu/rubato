/** @type {import('tailwindcss').Config} */
module.exports = {
  content: ["./templates/**/*.html"],
  theme: {
    extend: {
      colors: {
        nocturne: {
          bg: '#09090b', // Zinc 950
          panel: '#18181b', // Zinc 900
          gold: '#d4af37',
          'gold-light': '#f3cf55',
          text: '#e4e4e7', // Zinc 200
          muted: '#a1a1aa', // Zinc 400
        }
      },
      fontFamily: {
        sans: ['Inter', 'sans-serif'],
        serif: ['Playfair Display', 'serif'],
      }
    },
  },
  plugins: [],
}