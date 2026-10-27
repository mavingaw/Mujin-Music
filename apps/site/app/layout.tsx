import type { Metadata, Viewport } from "next";
import { Inter } from "next/font/google";
import { RootProvider } from "fumadocs-ui/provider/next";
import "./global.css";

const inter = Inter({ subsets: ["latin"] });

export const metadata: Metadata = {
  metadataBase: new URL(
    process.env.NEXT_PUBLIC_SITE_URL ?? "https://docs.parson.dev",
  ),
  title: { default: "Mujin Music Docs", template: "%s · Mujin Music Docs" },
  description:
    "Practical documentation for Mujin Music, the local-first music server and player.",
  applicationName: "Mujin Music Docs",
  icons: {
    icon: [
      { url: "/icons/icon.svg", type: "image/svg+xml" },
      { url: "/icons/favicon-32x32.png", sizes: "32x32" },
      { url: "/icons/favicon-16x16.png", sizes: "16x16" },
    ],
    apple: "/icons/apple-touch-icon.png",
  },
  openGraph: {
    title: "Mujin Music Docs",
    description: "Practical documentation for your Mujin Music music library.",
    type: "website",
  },
};

export const viewport: Viewport = {
  themeColor: "#090909",
  colorScheme: "dark",
};

export default function RootLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return (
    <html lang="en" className="dark" suppressHydrationWarning>
      <body className={`${inter.className} min-h-screen antialiased`}>
        <RootProvider theme={{ enabled: false }}>{children}</RootProvider>
      </body>
    </html>
  );
}
