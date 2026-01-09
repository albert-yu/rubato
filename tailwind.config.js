/** @type {import('tailwindcss').Config} */
module.exports = {
  content: ["./templates/**/*.html"],
  theme: {
    extend: {
      colors: {
        atelier: {
          bg: '#fdfcf8', // Warm Paper / Off-white
          panel: '#ffffff', // Pure White
          text: '#292524', // Stone 800 (Warm Black)
          muted: '#78716c', // Stone 500
          ink: '#1e293b', // Slate 800 (Deep Ink Blue)
          'ink-light': '#334155', // Slate 700
          border: '#e7e5e4', // Stone 200
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