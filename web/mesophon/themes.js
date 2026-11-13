// The terminal's themes (T-1208), in the TUI's Settings order: each the
// TUI's own palette (crates/mesimon-tui/src/theme.rs), drawn by style.css's
// tokens under `data-theme`, and here only the few colours a picker's tile
// shows. "terminal" follows the board's own theme; "system" is graphite or
// chalk by the device's appearance.
export const THEMES = [
  { id: "graphite", name: "Graphite", about: "dark, the default", light: false, bg: "#131417", card: "#272b31", ink: "#e9e7e1", dim: "#8c8880", attn: "#f0a93a", calm: "#6fbfb0", tags: ["#46acfc", "#20c188", "#f47a7a"] },
  { id: "chalk", name: "Chalk", about: "light, paper", light: true, bg: "#faf8f4", card: "#e7e3da", ink: "#22252b", dim: "#5f6169", attn: "#8f5600", calm: "#0b5f55", tags: ["#00629e", "#006f4c", "#a12e36"] },
  { id: "void", name: "Void", about: "graphite on true black", light: false, bg: "#000000", card: "#1c1d20", ink: "#e9e7e1", dim: "#8c8880", attn: "#f0a93a", calm: "#6fbfb0", tags: ["#46acfc", "#20c188", "#f47a7a"] },
  { id: "nord", name: "Nord", about: "nord, arctic blue-grey", light: false, bg: "#2e3440", card: "#3b4252", ink: "#eceff4", dim: "#9398a0", attn: "#f2c26b", calm: "#88c0d0", tags: ["#5cb5ff", "#2bc68d", "#ff8f8e"] },
  { id: "mocha", name: "Mocha", about: "catppuccin mocha, lavender ink", light: false, bg: "#1e1e2e", card: "#313244", ink: "#cdd6f4", dim: "#7d829a", attn: "#ffa46b", calm: "#94e2d5", tags: ["#46acfc", "#20c188", "#f47a7a"] },
  { id: "tokyo", name: "Tokyo", about: "tokyo night, periwinkle ink", light: false, bg: "#1a1b26", card: "#292e42", ink: "#c0caf5", dim: "#787f9c", attn: "#ff9e64", calm: "#73daca", tags: ["#46acfc", "#20c188", "#f47a7a"] },
  { id: "rose", name: "Rosé", about: "rosé pine, gold on plum", light: false, bg: "#191724", card: "#26233a", ink: "#e0def4", dim: "#848294", attn: "#f8b95a", calm: "#9ccfd8", tags: ["#46acfc", "#20c188", "#f47a7a"] },
  { id: "gruvbox", name: "Gruvbox", about: "gruvbox dark, warm cream ink", light: false, bg: "#282828", card: "#3c3836", ink: "#ebdbb2", dim: "#938a74", attn: "#fabd2f", calm: "#8ec07c", tags: ["#45acfc", "#21c188", "#f87d7d"] },
  { id: "blue", name: "Blue", about: "navy and gold, the Borland look", light: false, bg: "#000087", card: "#2c3590", ink: "#f6f0e7", dim: "#b0aaa2", attn: "#ffd700", calm: "#59d3d3", tags: ["#52b2ff", "#21c289", "#ff8887"] },
  { id: "amber", name: "Amber", about: "amber on black", light: false, bg: "#1b1201", card: "#322205", ink: "#f3ecde", dim: "#a0751c", attn: "#ffb000", calm: "#deae65", tags: ["#46acfc", "#20c188", "#f47a7a"] },
  { id: "green", name: "Green", about: "green-black, white ink, a green glow", light: false, bg: "#081a0c", card: "#18301e", ink: "#e8f1e6", dim: "#8d968b", attn: "#45e66b", calm: "#a3d6ae", tags: ["#46acfc", "#20c188", "#f47a7a"] },
  { id: "ice", name: "Ice", about: "teal-black, white ink, a cyan glow", light: false, bg: "#071b1a", card: "#153030", ink: "#e6f1f2", dim: "#829190", attn: "#2ee6e0", calm: "#a1eae5", tags: ["#46acfc", "#20c188", "#f47a7a"] },
  { id: "solarized", name: "Solarized", about: "solarized light, cream and blue-grey", light: true, bg: "#fdf6e3", card: "#eee8d5", ink: "#073642", dim: "#657b83", attn: "#8a6600", calm: "#1e6f6a", tags: ["#00629e", "#006f4c", "#a12e36"] },
  { id: "latte", name: "Latte", about: "catppuccin latte, cool paper", light: true, bg: "#eff1f5", card: "#dce0e8", ink: "#4c4f69", dim: "#6d7086", attn: "#b04a00", calm: "#0f6b70", tags: ["#00629e", "#006f4c", "#a12e36"] },
  { id: "gruvbox-light", name: "Gruvbox light", about: "gruvbox light, cream-yellow paper", light: true, bg: "#fbf1c7", card: "#ebdbb2", ink: "#3c3836", dim: "#777063", attn: "#8a5a06", calm: "#386a4b", tags: ["#00629e", "#006f4c", "#a12e36"] },
];

export const isTheme = (id) => THEMES.some((t) => t.id === id);

// What the page wears for a choice: the board's theme while it follows the
// terminal and the terminal said one, else the choice, else the device's.
export function wornTheme(choice, terminal) {
  if (choice === "terminal") return isTheme(terminal) ? terminal : "system";
  return choice === "system" || isTheme(choice) ? choice : "system";
}
