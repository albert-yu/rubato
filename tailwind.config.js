/** @type {import('tailwindcss').Config} */
module.exports = {
  content: ["./templates/**/*.html"],
  theme: {
    extend: {
      colors: {
        'sonata-black': '#0a0a0a',
        'sonata-charcoal': '#1a1a1a',
        'sonata-slate': '#666666',
        'sonata-pearl': '#e8e8e8',
        'sonata-border': '#2a2a2a',
        'sonata-border-hover': '#3a3a3a',
      },
      backgroundImage: {
        'card-gradient': 'linear-gradient(135deg, #1a1a1a 0%, #0f0f0f 100%)',
        'hero-gradient': 'linear-gradient(135deg, #ffffff 0%, #a0a0a0 100%)',
      }
    },
  },
  plugins: [],
}