import "./globals.css";

import type { Metadata, Viewport } from "next";
import type { ReactNode } from "react";
import SessionProvider from "@/features/account/session-provider";
import AppBootstrap from "@/components/app/splash-screen";
import NativeDragGuard from "@/components/app/native-drag-guard";

import { Toaster } from "sonner";

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
    <html lang="en" className="dark">
      <body className="min-h-screen bg-black font-sans text-white antialiased">
        <SessionProvider>
          <NativeDragGuard />
          <AppBootstrap>{children}</AppBootstrap>
          <Toaster
            closeButton={false}
            position="bottom-right"
            theme="dark"
            toastOptions={{
              classNames: {
                toast: "!border-white/10 !bg-black !text-white !shadow-2xl",
                description: "!text-zinc-400",
                actionButton: "!bg-white !text-black",
                cancelButton: "!bg-zinc-900 !text-white",
              },
            }}
          />
        </SessionProvider>
      </body>
    </html>
  );
}
