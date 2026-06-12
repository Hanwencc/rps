/** @type {import("tailwindcss").Config} */
export default {
  content: ["./index.html", "./src/**/*.{vue,ts}"],
  theme: {
    extend: {
      colors: {
        ink: "#111827",
        muted: "#6b7280",
      },
    },
  },
  plugins: [],
};
