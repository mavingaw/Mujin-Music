import { createRequire } from "node:module";
import { createMDX } from "fumadocs-mdx/next";

const require = createRequire(import.meta.url);
const { releasesBase } = require("../../tools/releases-base.cjs");

/** @type {import('next').NextConfig} */
const config = {
  reactStrictMode: true,
  devIndicators: false,
  // Download links point at the forge that publishes the release packages.
  env: { MUJIN_RELEASES_BASE: releasesBase(process.env) },
  webpack(webpackConfig) {
    // Webpack cannot statically trace Fumadocs' generated file-URL imports.
    webpackConfig.infrastructureLogging = { level: "error" };
    return webpackConfig;
  },
  async rewrites() {
    return [
      {
        source: "/docs/:path*.md",
        destination: "/llms.mdx/docs/:path*",
      },
    ];
  },
};

export default createMDX()(config);
