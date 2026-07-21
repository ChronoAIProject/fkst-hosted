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
      blue: "var(--blue)",
      // Warning semaphore, decoupled from the (now blue) accent pair.
      warn: "var(--warn)",
      // Translucent surfaces for backdrop-blur glass panels (bg-glass utility).
      glass: "var(--glass)",
      "glass-2": "var(--glass-2)",
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
        pill: '9999px',
      },
      borderColor: {
        DEFAULT: 'var(--line)', // overrides preflight border color default
      },
      boxShadow: {
        'modal-seat': '0 24px 60px -22px rgba(0,0,0,.6)',
        // Layered elevation scale (see tokens.css --shadow-*).
        1: 'var(--shadow-1)',
        2: 'var(--shadow-2)',
        3: 'var(--shadow-3)',
        glow: 'var(--shadow-glow)', // card depth + amber accent bloom
        'glow-amber': 'var(--glow-amber)',
        'glow-green': 'var(--glow-green)',
        'glow-red': 'var(--glow-red)',
        'highlight-top': 'var(--highlight-top)', // inner top edge on raised cards
      },
      backgroundImage: {
        // Brand + heading gradients and the fixed app bloom (see tokens.css).
        'grad-accent': 'var(--grad-accent)',
        'grad-fg': 'var(--grad-fg)',
        'bg-glow': 'var(--bg-glow)',
        'grad-hairline': 'var(--grad-hairline)',
        'grad-hairline-accent': 'var(--grad-hairline-accent)',
      },
      backdropBlur: {
        glass: '12px', // pairs with bg-glass for layered-glass panels
      },
      fontSize: {
        // Eyebrows / labels — mono, wide tracking.
        eyebrow: ['11px', { letterSpacing: '0.18em' }],
        'eyebrow-lg': ['12px', { letterSpacing: '0.16em', lineHeight: '1' }],
        // Body + nav.
        body: ['14px', { lineHeight: '1.5' }],
        nav: '13.5px',
        // Headings — refined display scale. clamp() makes the hero sizes fluid;
        // tighter tracking + snug line-heights give a confident, modern feel.
        'modal-title': ['19px', { letterSpacing: '-0.01em' }],
        'display-sm': ['clamp(1.25rem, 1vw + 1rem, 1.5rem)', { lineHeight: '1.15', letterSpacing: '-0.015em' }],
        'display-md': ['clamp(1.5rem, 2vw + 0.75rem, 2rem)', { lineHeight: '1.1', letterSpacing: '-0.02em' }],
        'display-lg': ['clamp(2rem, 3.5vw + 0.75rem, 3rem)', { lineHeight: '1.05', letterSpacing: '-0.025em', fontWeight: '700' }],
        'display-xl': ['clamp(2.5rem, 5vw + 1rem, 4rem)', { lineHeight: '1.02', letterSpacing: '-0.03em', fontWeight: '700' }],
      },
      transitionDuration: {
        DEFAULT: '120ms',
      },
      transitionTimingFunction: {
        // The house easing already used by the CSS entrance animations; exposed
        // so hover-lift / underline-grow transitions share one motion feel.
        emphasized: 'cubic-bezier(0.2, 0.7, 0.3, 1)',
      },
      // ---- Motion registry -------------------------------------------------
      // Canonical @keyframes + animation NAMES for the NEW rich-motion vocabulary.
      // Registering them here emits the @keyframes once and yields animate-*
      // utilities; the motion agent wraps these into .anim-*/.hover-* classes in
      // index.css and MUST reference (not redefine) these keyframe names to avoid
      // duplicate @keyframes. Every one collapses to its final state under
      // prefers-reduced-motion via the index.css @media block.
      keyframes: {
        'gradient-shift': {
          '0%, 100%': { backgroundPosition: '0% 50%' },
          '50%': { backgroundPosition: '100% 50%' },
        },
        'glow-pulse': {
          '0%, 100%': { boxShadow: '0 0 0 0 transparent' },
          '50%': { boxShadow: 'var(--glow-amber)' },
        },
        float: {
          '0%, 100%': { transform: 'translateY(0)' },
          '50%': { transform: 'translateY(-6px)' },
        },
        sheen: {
          '0%': { transform: 'translateX(-120%)' },
          '100%': { transform: 'translateX(120%)' },
        },
      },
      animation: {
        // Slow shimmer of a 200%-sized gradient (pair with .grad-text/.grad-border).
        'gradient-shift': 'gradient-shift 6s ease-in-out infinite',
        // Breathing accent glow for active/primary elements.
        'glow-pulse': 'glow-pulse 2.4s ease-in-out infinite',
        // Tiny vertical bob for hero accent marks.
        float: 'float 4s ease-in-out infinite',
        // One-shot highlight sweep across a surface.
        sheen: 'sheen 1.2s ease-out',
      },
      maxWidth: {
        shell: '1440px',
      },
    },
  },
  plugins: [],
}
