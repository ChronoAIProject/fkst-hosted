/** @type {import('tailwindcss').Config} */
export default {
  content: [
    "./index.html",
    "./src/**/*.{js,ts,jsx,tsx}",
  ],
  theme: {
    colors: {
      transparent: 'transparent',
      current: 'currentColor',
      inherit: 'inherit',
      white: '#ffffff',
      black: '#000000',
      bg: "var(--bg)",
      raise: "var(--raise)",
      "raise-2": "var(--raise-2)",
      line: "var(--line)",
      "line-2": "var(--line-2)",
      fg: "var(--fg)",
      dim: "var(--dim)",
      faint: "var(--faint)",
      ghost: "var(--ghost)",
      amber: "var(--amber)",
      "amber-ink": "var(--amber-ink)",
      green: "var(--green)",
      red: "var(--red)",
      gold: "var(--gold)",
    },
    fontFamily: {
      display: ["var(--display)"],
      ui: ["var(--ui)"],
      mono: ["var(--mono)"],
      sans: ["var(--ui)"], // overrides Tailwind default sans so nothing resolves to system-ui
    },
    extend: {
      borderRadius: {
        chip: '6px',
        control: '8px',
        card: '10px',
        panel: '14px',
        modal: '16px',
      },
      borderColor: {
        DEFAULT: 'var(--line)', // overrides preflight border color default
      },
      boxShadow: {
        'modal-seat': '0 24px 60px -22px rgba(0,0,0,.6)',
      },
      fontSize: {
        eyebrow: ['11px', { letterSpacing: '0.18em' }],
        body: ['14px', { lineHeight: '1.5' }],
        nav: '13.5px',
        'modal-title': ['19px', { letterSpacing: '-0.01em' }],
      },
      transitionDuration: {
        DEFAULT: '120ms',
      },
      maxWidth: {
        shell: '1440px',
      },
    },
  },
  plugins: [],
}
