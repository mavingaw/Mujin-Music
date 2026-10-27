import { Platform } from "react-native";

import { nativePlaybackCapabilities } from "@/lib/playback-formats";

/** What this device's decoders play, sent with every stream request. */
export const devicePlaybackCapabilities = () =>
  nativePlaybackCapabilities(Platform.OS);
