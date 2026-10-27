/** Mujin Music palette (brand tokens): deep blacks, ivory text, one copper accent. */
export const palette = {
  background: "#0A0A0B",
  elevated: "#111113",
  elevatedStrong: "#1E1E22",
  border: "rgba(242,238,230,0.10)",
  borderStrong: "rgba(242,238,230,0.18)",
  text: "#F2EEE6",
  secondary: "#B8B3AA",
  muted: "#5C5955",
  accent: "#D98B5F",
  accentSoft: "#E6A27A",
  accentDeep: "#BF7246",
  accentText: "#160D07",
  qualityHigh: "#A3C98B",
  qualityLow: "#7E7A74",
  danger: "#F2473F",
  success: "#A3C98B",
  tertiary: "#8E8A84",
  surface: "#17171A",
} as const;

export const layout = {
  horizontal: 20,
  tabBar: 76,
  miniPlayer: 68,
  radius: 16,
} as const;

/**
 * Brand type. Jost (SIL OFL 1.1, bundled under assets/fonts/jost) for
 * display and section titles; body text stays on the platform face.
 */
export const fonts = {
  display: "Jost-Light",
  title: "Jost-Regular",
  titleMedium: "Jost-Medium",
} as const;

/** Small caps label above a title ("DAILY PICKS"). */
export const eyebrow = {
  color: "#8E8A84",
  fontSize: 11,
  fontWeight: "600",
  letterSpacing: 1.6,
  textTransform: "uppercase",
} as const;
