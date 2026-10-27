import type { Metadata, Viewport } from "next";
import type { ReactNode } from "react";

export const metadata: Metadata = {
  title: "Display · Mujin Music",
  robots: { index: false, follow: false },
};

export const viewport: Viewport = {
  themeColor: "#050506",
  viewportFit: "cover",
};

export default function DisplayLayout({ children }: { children: ReactNode }) {
  return children;
}
