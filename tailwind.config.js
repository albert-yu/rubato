/** @type {import('tailwindcss').Config} */
module.exports = {
  content: ["./templates/**/*.html", "./src/**/*.rs"],
  theme: {
    extend: {
      colors: {
        'sonata-black': 'rgb(var(--sonata-black) / <alpha-value>)',
        'sonata-charcoal': 'rgb(var(--sonata-charcoal) / <alpha-value>)',
        'sonata-slate': 'rgb(var(--sonata-slate) / <alpha-value>)',
        'sonata-pearl': 'rgb(var(--sonata-pearl) / <alpha-value>)',
        'sonata-border': 'rgb(var(--sonata-border) / <alpha-value>)',
        'sonata-border-hover': 'rgb(var(--sonata-border-hover) / <alpha-value>)',
        'sonata-highlight': 'rgb(var(--sonata-highlight) / <alpha-value>)',
      },
      backgroundImage: {
        'card-gradient': 'var(--card-gradient)',
        'hero-gradient': 'var(--hero-gradient)',
      }
    },
  },
  plugins: [],
}