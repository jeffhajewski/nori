import type { Config } from "tailwindcss";

const config: Config = {
  darkMode: "class",
  content: [
    "./src/pages/**/*.{js,ts,jsx,tsx,mdx}",
    "./src/components/**/*.{js,ts,jsx,tsx,mdx}",
    "./src/app/**/*.{js,ts,jsx,tsx,mdx}",
  ],
  theme: {
    extend: {
      colors: {
        // Background layers
        background: "hsl(var(--background))",
        foreground: "hsl(var(--foreground))",

        // Card/surface colors
        card: {
          DEFAULT: "hsl(var(--card))",
          foreground: "hsl(var(--card-foreground))",
        },

        // Muted text/backgrounds
        muted: {
          DEFAULT: "hsl(var(--muted))",
          foreground: "hsl(var(--muted-foreground))",
        },

        // Borders
        border: "hsl(var(--border))",

        // Primary accent (violet - NoriKV brand)
        primary: {
          DEFAULT: "hsl(var(--primary))",
          foreground: "hsl(var(--primary-foreground))",
        },

        // Secondary accent
        secondary: {
          DEFAULT: "hsl(var(--secondary))",
          foreground: "hsl(var(--secondary-foreground))",
        },

        // Status colors
        healthy: "hsl(var(--healthy))",
        warning: "hsl(var(--warning))",
        critical: "hsl(var(--critical))",

        // Role colors
        leader: "hsl(var(--leader))",
        follower: "hsl(var(--follower))",

        // LSM level colors
        "level-0": "hsl(var(--level-0))",
        "level-1": "hsl(var(--level-1))",
        "level-2": "hsl(var(--level-2))",
        "level-3": "hsl(var(--level-3))",
        "level-4": "hsl(var(--level-4))",
        "level-5": "hsl(var(--level-5))",
        "level-6": "hsl(var(--level-6))",

        // Event type colors
        "evt-wal": "hsl(var(--evt-wal))",
        "evt-compaction": "hsl(var(--evt-compaction))",
        "evt-lsm": "hsl(var(--evt-lsm))",
        "evt-raft": "hsl(var(--evt-raft))",
        "evt-swim": "hsl(var(--evt-swim))",
        "evt-shard": "hsl(var(--evt-shard))",
      },
      fontFamily: {
        sans: ["Inter", "system-ui", "-apple-system", "sans-serif"],
        mono: ["JetBrains Mono", "Fira Code", "Consolas", "monospace"],
      },
      animation: {
        "pulse-slow": "pulse 3s cubic-bezier(0.4, 0, 0.6, 1) infinite",
        "pulse-fast": "pulse 1s cubic-bezier(0.4, 0, 0.6, 1) infinite",
        "spin-slow": "spin 3s linear infinite",
        "bounce-subtle": "bounce-subtle 2s infinite",
        "glow": "glow 2s ease-in-out infinite alternate",
      },
      keyframes: {
        "bounce-subtle": {
          "0%, 100%": {
            transform: "translateY(-5%)",
            animationTimingFunction: "cubic-bezier(0.8, 0, 1, 1)",
          },
          "50%": {
            transform: "translateY(0)",
            animationTimingFunction: "cubic-bezier(0, 0, 0.2, 1)",
          },
        },
        glow: {
          from: {
            boxShadow: "0 0 5px var(--glow-color), 0 0 10px var(--glow-color)",
          },
          to: {
            boxShadow: "0 0 10px var(--glow-color), 0 0 20px var(--glow-color)",
          },
        },
      },
      borderRadius: {
        lg: "var(--radius)",
        md: "calc(var(--radius) - 2px)",
        sm: "calc(var(--radius) - 4px)",
      },
    },
  },
  plugins: [],
};

export default config;
