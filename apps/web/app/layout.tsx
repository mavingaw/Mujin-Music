import "./globals.css";

import type { Metadata, Viewport } from "next";
import type { ReactNode } from "react";
import SessionProvider from "@/features/account/session-provider";
import AppBootstrap from "@/components/app/splash-screen";
import NativeDragGuard from "@/components/app/native-drag-guard";

import { THEME_BOOT_SCRIPT } from "@/features/appearance/theme-boot";
import ThemedToaster from "@/features/appearance/themed-toaster";

export const metadata: Metadata = {
  metadataBase: new URL(
    process.env.NEXT_PUBLIC_SITE_URL ?? "https://parson.dev",
  ),
  applicationName: "Mujin Music",
  title: "Mujin Music",
  description: "Own your music.",
  manifest: "/site.webmanifest",
  icons: {
    icon: [
      { url: "/favicon.ico", sizes: "48x48" },
      { url: "/icons/icon.svg", type: "image/svg+xml" },
      { url: "/icons/favicon-32x32.png", sizes: "32x32", type: "image/png" },
      { url: "/icons/favicon-16x16.png", sizes: "16x16", type: "image/png" },
    ],
    apple: [{ url: "/icons/apple-touch-icon.png", sizes: "180x180" }],
  },
  openGraph: {
    title: "Mujin Music",
    description: "Own your music.",
    images: [{ url: "/images/og/mujin-music.jpg", width: 1200, height: 630 }],
  },
  twitter: {
    card: "summary_large_image",
    title: "Mujin Music",
    description: "Own your music.",
    images: ["/images/og/mujin-music.jpg"],
  },
};

export const viewport: Viewport = { themeColor: "#0A0A0B" };

export default function RootLayout({ children }: { children: ReactNode }) {
  return (
    <html lang="en" className="dark" suppressHydrationWarning>
      <head>
        {/* Paints the profile's theme before the app starts (see theme-cache). */}
        <script dangerouslySetInnerHTML={{ __html: THEME_BOOT_SCRIPT }} />
      </head>
      <body className="min-h-screen bg-black font-sans text-white antialiased">
        <SessionProvider>
          <NativeDragGuard />
          <AppBootstrap>{children}</AppBootstrap>
          <ThemedToaster />
        </SessionProvider>
      </body>
    </html>
  );
}
