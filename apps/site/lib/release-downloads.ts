import packageJson from "../package.json";
export const releaseVersion = packageJson.version;
export const releaseTag = `v${releaseVersion}`;
/**
 * The releases page the site links to: MUJIN_RELEASES_BASE at build time
 * (next.config.mjs checks and inlines it; tools/releases-base.cjs has the
 * rules). Both GitHub and Gitea serve a tag's files under /download/<tag>
 * and its page under /tag/<tag>.
 */
const releasesBase =
  process.env.MUJIN_RELEASES_BASE ||
  "https://github.com/mavingaw/Mujin-Music/releases";
export const releaseBase = `${releasesBase}/download/${releaseTag}`;
export const releasePage = `${releasesBase}/tag/${releaseTag}`;

/** The Windows installer, e.g. `Mujin-Music-Setup-1.0.1-x64.exe`. */
export const windowsSetup = (arch: string) =>
  `${releaseBase}/Mujin-Music-Setup-${releaseVersion}-${arch}.exe`;
const asset = (suffix: string) =>
  `${releaseBase}/Mujin-Music-${releaseVersion}-${suffix}`;

export const releaseDownloads = {
  windowsX64: windowsSetup("x64"),
  windowsArm64: windowsSetup("arm64"),
  linuxX64AppImage: asset("x86_64.AppImage"),
  linuxArm64AppImage: asset("arm64.AppImage"),
  linuxX64Deb: asset("amd64.deb"),
  linuxArm64Deb: asset("arm64.deb"),
  macX64Dmg: asset("x64.dmg"),
  macArm64Dmg: asset("arm64.dmg"),
} as const;
