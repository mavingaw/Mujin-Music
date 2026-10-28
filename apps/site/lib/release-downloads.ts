import packageJson from "../package.json";
export const releaseVersion = packageJson.version;
export const releaseTag = `v${releaseVersion}`;
export const releaseBase =
  `https://github.com/mavingaw/Mujin-Music/releases/download/${releaseTag}` as const;
export const releasePage =
  `https://github.com/mavingaw/Mujin-Music/releases/tag/${releaseTag}` as const;

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
