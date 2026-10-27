import { CastButton } from "react-native-google-cast";
import { palette } from "@/constants/colors";

export function CastOutputButton() {
  return (
    <CastButton
      accessibilityLabel="Play on a Cast device"
      style={{ height: 32, tintColor: palette.text, width: 32 }}
    />
  );
}
