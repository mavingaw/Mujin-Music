import { personInitials, personTone } from "@parson/music-sdk";
import { StyleSheet, Text, View, type ViewStyle } from "react-native";

import { Artwork } from "@/components/artwork";
import { apiUrl } from "@/lib/runtime";
import { tint } from "@/constants/colors";

/** A round photo of a person, or their initials when there is none. */
export function PersonAvatar({
  name,
  imageUrl,
  size,
  style,
}: {
  name: string;
  /** Server path (`/artists/{id}/image`) or a full URL. */
  imageUrl?: string | null;
  size: number;
  style?: ViewStyle;
}) {
  if (imageUrl) {
    const path = /^https?:\/\//.test(imageUrl) ? imageUrl : apiUrl(imageUrl);
    return <Artwork path={path} rounded={size / 2} size={size} style={style} />;
  }
  const [tone] = personTone(name);
  return (
    <View
      accessibilityElementsHidden
      importantForAccessibility="no-hide-descendants"
      style={[
        styles.initials,
        {
          width: size,
          height: size,
          borderRadius: size / 2,
          backgroundColor: tone,
        },
        style,
      ]}
    >
      <Text style={[styles.letters, { fontSize: Math.round(size * 0.36) }]}>
        {personInitials(name)}
      </Text>
    </View>
  );
}

const styles = StyleSheet.create({
  initials: {
    alignItems: "center",
    justifyContent: "center",
    borderWidth: StyleSheet.hairlineWidth,
    borderColor: tint(0.15),
  },
  letters: { color: tint(0.92), fontWeight: "800" },
});
